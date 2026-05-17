---
id: 0067
title: engine-owned-rotation-fanout-self-refresh-hook
status: accepted
date: 2026-05-17
supersedes: []
superseded_by: []
amends: [0044]
tags: [resource, engine, credential, rotation, api, m11, m12, supersession]
related:
  - docs/adr/0030-engine-owned-credential-orchestration.md
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md
  - docs/adr/0043-dependency-declaration-dx.md
  - docs/adr/0044-supersede-0036-resource-credential-singular.md
  - docs/adr/0051-external-provider-redesign.md
  - docs/superpowers/specs/2026-05-15-nebula-resource-finalization-design.md
---

# 0067. Engine-owned per-slot rotation fan-out + `&self` refresh hook

## Status

Accepted (2026-05-17). The original infrastructure (engine-side reverse
index, dispatch port, `RotationOutcome` aggregation, `SlotCell`
substrate, API config-CRUD surface) plus, landed 2026-05-17 on
`feat/engine-resource-rotation-wiring`, the **rotation/lease-revoke
dispatch wiring and the per-slot-rotation correctness fixes**: an
engine-owned `ResourceFanoutDriver` subscribes the credential-runtime
`CredentialEvent::{Refreshed,Revoked}` and `LeaseEvent::LeaseRevoked`
buses and invokes `dispatch_refresh` / `dispatch_revoke` (proven e2e
through the real bus path), and the three items that became live with
it are fixed — per-resource drain + post-taint re-check (#679), a
two-phase cancellation-safe revoke port so a timed-out/dropped revoke
still leaves the row tainted (#681), and epoch-reconcile of a rotation
racing a resident's first acquire (#680). **Still deferred:** the
*bind-population* producer — populating the reverse index when a
credential is resolved into a `#[credential]` slot in production —
depends on the resource-activation path (plugin-driven registrar
auto-population / a production `ResourceRepo`), which remains deferred
(see [Deferred](#deferred)); the bind seam (`register_and_bind`) is
implemented and ready but has no production caller yet. Net: §M11.5's
rotation **orchestration and correctness** are closed; full end-to-end
§M11.5 / §M12.4 (`nebula-resource` frontier→stable) closure
additionally requires that deferred resource-activation bind-population
— a real rotation/revoke event now fans out correctly to every *bound*
row, but production binds none until activation lands. Amends ADR-0044 (hook
signature **and** the credential-slot-field / migration shape — see
[Supersession](#supersession)); overrides the
`.ai-factory/PHASE4_BLOCKED.md §1` "re-add rotation orchestration to
`resource::Manager`" candidate (a phase-note, never an accepted ADR).

> Numbering: the worktree-local ADR space (`docs/adr/`) and the parent
> L1 canon (`C:/Users/vanya/RustroverProjects/docs/adr/0001..0041`) are
> deliberately distinct spaces with intentional low-number collisions
> (see the out-of-scope ADR-0042 collision note in the finalization
> spec §2). Within the worktree-local space the action+schema redesign
> batch consumed `0052..0065` and a concurrent sandbox-broker line
> claimed `0066`; `0067` is the first globally-free number across every
> ref. The finalization spec/plan drafted against the then-expected
> `0052` before that batch landed — this ADR is that record, filed at
> the real next-free number.

## Context

ADR-0044 retired the singular `Resource::Credential` associated type in
favour of declarative `#[credential(key = …)]` slot fields and a
per-slot `on_credential_refresh` hook. Two things it specified were left
unimplementable or unmodelled, and `PHASE4_BLOCKED.md §1` explicitly
recorded them as open:

1. **Reentrancy of the refresh hook.** ADR-0044 wrote
   `on_credential_refresh(&mut self, slot_name)`. But `ManagedResource`
   hands callers `Arc<R>` (no `&mut R`), and the resource impl object
   `Self` is an *immutable factory descriptor* — per ADR-0043 §5,
   per-execution and live state live in `Self::Runtime`, not on `self`.
   ADR-0036 itself states blue-green pool replacement is "internalised
   by the resource impl … owns its `Arc<RwLock<Pool>>` write-lock
   window" — i.e. the swap point is in the *runtime*, not the
   descriptor. `&mut self` therefore conflates two responsibilities and
   would force `Arc<RwLock<R>>` on **every** `ManagedResource`, putting
   a lock on the acquire hot path for all resources to support a hook
   that, under correct blue-green, never needs `&mut self`.
   `PHASE4_BLOCKED.md §1.2` flagged this trade-off as unresolved.

2. **The credential-slot field shape.** ADR-0044's migration note shows
   a bare `#[credential] auth: CredentialGuard<C>` field that "the
   framework writes before `create`". A pure `#[proc_macro_derive]`
   **cannot add or rewrite struct fields**, and nothing in the shipped
   code stored a resolved `CredentialGuard<C>` on a resource instance at
   runtime. The migration shape as written is not implementable.

3. **Ownership of rotation orchestration.** ADR-0030 already placed
   credential-rotation orchestration — *when* to rotate, fan-out across
   affected resources, per-resource timeout budgets, outcome
   aggregation, cross-replica coalescing — in `nebula-engine`.
   `PHASE4_BLOCKED.md §1` *proposed* re-adding this to
   `resource::Manager`. Doing so would re-violate ADR-0030's SRP split
   and give `Manager` a second reason to change.

The associated abuse review also confirmed a latent cross-tenant bug:
`ResourceConfig::fingerprint()` defaults to `0`, so the `Manager`
register/acquire dedup key collapsed every config of a type to one
runtime regardless of the resolved credential — a cross-tenant bleed
relying on authors remembering to override `fingerprint()`
(discipline-based, rejected).

## Decision

### D1 — Reverse-index + fan-out lives in `nebula-engine`, not `resource::Manager`

`resource::Manager`'s single responsibility stays resource lifecycle
(register / acquire / health / release / shutdown). Credential-rotation
orchestration stays in `nebula-engine` per ADR-0030.

- `nebula-engine` gains `crates/engine/src/credential/rotation/resource_fanout.rs`
  (sibling of the existing `scheduler.rs` / `grace_period.rs` /
  `blue_green.rs` / `transaction.rs` / `token_refresh.rs`, ADR-0030 §1):
  a reverse index keyed by resolved credential identity to the
  `(ResourceKey, ScopeLevel, slot_name, slot_identity)` binds it feeds,
  populated when the engine resolves a credential into a resource slot
  and drained on resource removal/shutdown.
- On a rotation event (ADR-0030 scheduler) or a lease-revoke
  (ADR-0051 `LeaseEvent`), the engine looks up affected binds and
  `join_all`s `Manager::refresh_slot` / `revoke_slot` with a
  **per-resource timeout budget** — never a single global timeout; one
  slow resource must not cascade-fail siblings (ADR-0036 invariant).
  Results aggregate into a `RotationOutcome { success, failed,
  timed_out }`.
- `resource::Manager` exposes only the narrow typed port:

  ```rust
  impl Manager {
      /// Engine-driven. Apply a rotated slot to the live runtime.
      /// Idempotent; per-resource isolated (caller wraps in a timeout).
      pub async fn refresh_slot(
          &self, key: &ResourceKey, scope: ScopeLevel, slot_name: &str,
      ) -> Result<(), crate::Error>;

      pub async fn revoke_slot(
          &self, key: &ResourceKey, scope: ScopeLevel, slot_name: &str,
      ) -> Result<(), crate::Error>;
  }
  ```

  No `nebula-resource → nebula-engine` edge is introduced: engine
  already holds `Arc<nebula_resource::Manager>` and owns
  `crates/engine/src/credential/`. Cross-crate fan-out signalling goes
  through `nebula-eventbus`, not direct sibling imports (AGENTS.md).

- **SRP rejected counter** ("Manager already holds the
  `ManagedResource` map; routing through engine adds indirection"):
  rejected. Rotation is rare (per-expiry-window, cross-replica
  coalesced per ADR-0030/0041); `engine → manager.refresh_slot` is one
  typed call of the same class as `engine → manager.acquire`, no extra
  hop, and the SRP/DIP win dominates.

### D2 — `&self` + `&Self::Runtime` hook; `#[credential]` field is `SlotCell<CredentialGuard<C>>`

Corrected trait shape in `crates/resource/src/resource.rs` (async
default no-op; the descriptor `self` is immutable, the reaction acts on
the live runtime's own interior mutability):

```rust
fn on_credential_refresh(
    &self,
    slot_name: &str,
    runtime: &Self::Runtime,
) -> impl Future<Output = Result<(), Self::Error>> + Send {
    let _ = (slot_name, runtime);
    async { Ok(()) }
}

fn on_credential_revoke(
    &self,
    slot_name: &str,
    runtime: &Self::Runtime,
) -> impl Future<Output = Result<(), Self::Error>> + Send {
    let _ = (slot_name, runtime);
    async { Ok(()) }
}
```

Because a pure derive cannot synthesise fields, the `#[credential]`
field's **declared type is `SlotCell<CredentialGuard<C>>`** (a
lock-free `ArcSwapOption` cell) directly on the author's struct.
`#[derive(Resource)]` emits, per slot, an inherent read accessor
`fn <field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>`
(= `self.<field>.load()`); the framework populates and rotates via
`SlotCell::store` through `&self`. No hidden side-table, fully
per-instance. `CredentialGuard` is `!Clone` + `Drop`-zeroizing and the
cell holds `Arc<CredentialGuard<C>>`, so a rotation swap never clones
secret material. `decode_field_type` rejects `Option`/`Lazy`-wrapped
slot cells at the derive site (compile-fail), so the slot shape cannot
silently drift.

- **Rejected sub-option** (hybrid: opt-in `RwLock` if the author
  declared a `mut` field): a discipline-based escape hatch for a case
  the corrected model does not produce; complicates the derive for dead
  weight.
- **Rejected churn counter** ("ADR-0044 is freshly accepted in this
  same epic"): ADR-0044's *core* — drop `type Credential`, declarative
  `#[credential(key = …)]` slot binding — is correct and untouched.
  Only the reentrancy modelling and the field *type/read pattern* are
  corrected; both were the items `PHASE4_BLOCKED.md §1` left open. The
  owner authorised breaking changes for spec-correctness.

### D3 — API is config-CRUD (write) + read-only status (read); no lifecycle over HTTP

- §11.4: acquire/release are engine-owned. §13.1: lifecycle must be
  attributable in the durable journal / operator trace — an
  observability projection, not an HTTP mutation surface.
- CQRS split: write = `ResourceEntry` config CRUD validated against
  `R::Config` schema; read = list/get config + a read-only runtime
  status projection (phase/health only, via an engine-side
  `EngineResourceStatus` seam — no `nebula-resource` type crosses into
  the public API tier; ADR-0028 §7, ADR-0047 wrappers).
- Exposing acquire/release over HTTP would be a confused-deputy
  ("acquire an arbitrary resource") and an SRP violation (API owning
  lifecycle it does not own). The only `{res}` sub-route is a GET
  status read; there is intentionally no acquire/release/drain/reload
  route, and a regression test asserts a POST acquire route does not
  exist.

## Abuse-case invariants

| # | Abuse | Invariant |
|---|---|---|
| 1 | Cross-tenant dedup — `ResourceConfig::fingerprint()` defaults to `0`, collapsing every config of a type to one runtime regardless of resolved credential | **Confirmed bug, fixed structurally at `Manager`**: the dedup key gains a slot-identity component derived from the resolved credential identity per `#[credential]` slot, independent of the author's `fingerprint()`. Not discipline-based — authors cannot regress it by forgetting an override. |
| 2 | Revoke race on a shared runtime (ADR-0036: zero authenticated traffic post-revoke) | Engine-driven ordering: engine marks the credential `revoking` → `Manager::revoke_slot` taints the runtime (new acquires rejected via the existing `tainted` guard) → drains in-flight guards (`ReleaseQueue` + `drain_tracker`) → reports → engine completes revoke. **Satisfied (2026-05-17):** the drain step awaits a *per-resource* in-flight counter (the manager-wide `drain_tracker` stays only for `graceful_shutdown`) and every `run_*_acquire` re-checks taint after the in-flight increment (#679); the revoke port is two-phase — a synchronous taint is applied before any `.await`, so a timed-out or dropped `drain_and_revoke` still leaves the row tainted (#681). The "no acquire after taint observes the revoked credential" guarantee is now per-resource and tested end-to-end through the wired bus path. |
| 3 | Secret in config JSON via API (ADR-0028 §7) | `register_from_value(json)` validates against `<R::Config as HasSchema>::schema()`; `ResourceConfig` carries no secrets (slots are credential *references* by key/id, §3.5). API DTOs use ADR-0047 wrappers — zero core/engine/storage types in the wire schema. A regression test rejects secret-shaped config (negative + positive control). |
| 4 | Type confusion `kind: String → R` in `register_from_value` | `kind → registrar` is a **closed allowlist** (INTEGRATION_MODEL §114-120 closed dependency graph), never reflection. Unknown `kind` ⇒ a typed conflict error, never a silent runtime grab. |

## Deferred

Carried forward from the stale 2026-04-24 concerns register
(`C:/Users/vanya/RustroverProjects/docs/tracking/nebula-resource-concerns-register.md`)
so retirement of that register is traceable. Each item has an explicit
re-open trigger; none is silently dropped.

| Concern | Why deferred | Re-open trigger |
|---|---|---|
| R-006 — `AuthScheme: Clone` zeroize obligation | Future-cascade; not on the §M11.5/§M12.4 critical path | A cross-crate credential reshape that re-touches `AuthScheme` |
| R-041 — no `benches/` / CodSpeed for `nebula-resource` | Post-cascade; perf harness is a separate milestone | A bench-harness milestone reaches `nebula-resource` |
| R-042 — zero feature flags (no constrained-context build) | Future-cascade; no current constrained-context consumer | A constrained-context (no-std / minimal) build requirement lands |
| R-050 — five associated types ⇒ combinatorial trait bounds | Future-cascade; the shape is sound for the single current consumer | A second consumer needs a distinct associated-type shape |
| R-052 — `Resource::destroy` no-op leak | Sequenced after the rotation rebuild | Revisit now the per-slot rotation fan-out has landed |

Concerns superseded by this ADR's engine-side rebuild (their П2
machinery was deleted in Phase 4 and rebuilt engine-owned here):
R-002 / R-003 / R-004 / R-060. R-040 is independently resolved
(`deny.toml:108` carries the `nebula-resource` layer-wrapper rule). The
register **retires** when the MATURITY frontier→stable flip lands; that
flip is its own close-condition (the register's "→ core" wording is the
stale Strategy-§6.4 term — the live taxonomy is `frontier`/`stable`).

Implementation-discovered deferrals (recorded here so they are not lost
behind the plan):

- **Rotation fan-out dispatch wired (2026-05-17); bind-population still
  deferred.** The dispatch path is live and e2e-proven: an
  engine-owned `ResourceFanoutDriver` subscribes the credential-runtime
  `CredentialEvent::{Refreshed,Revoked}` and `LeaseEvent::LeaseRevoked`
  buses and invokes `ResourceFanoutIndex::dispatch_refresh` /
  `dispatch_revoke` (branch `feat/engine-resource-rotation-wiring`;
  tested through the real bus path, not by calling `dispatch_*`
  directly). The remaining unwired piece is **bind-population** —
  calling `ResourceFanoutIndex::bind` (via the ready `register_and_bind`
  seam) when a credential is resolved into a `#[credential]` slot in
  production. No production path resolves credential→slot yet; that
  producer is the resource-activation path covered by the *Plugin-driven
  registrar auto-population* / *No production `ResourceRepo`* items
  below. Until it lands, a real rotation/revoke event fans out
  correctly to every *bound* row but production binds none. Trigger: a
  production resource-activation path resolves a credential into a slot
  and calls `register_and_bind`.
- **Revoke per-resource drain — resolved (2026-05-17).** `Manager` now
  drains a *per-resource* in-flight counter for revoke (the manager-wide
  `drain_tracker` is retained only for `graceful_shutdown`), and every
  `run_*_acquire` re-checks taint after the in-flight increment (#679).
  The revoke port is two-phase: a synchronous `taint_slot{,_for}`
  applied before any `.await`, then `drain_and_revoke` wrapped in the
  per-resource timeout by the fan-out, so a timed-out/dropped revoke
  still leaves the row tainted (#681). A revoke no longer blocks on
  traffic to a sibling resource.
- **Plugin-driven registrar auto-population.** The engine holds a
  closed `ResourceRegistrarRegistry`, but it is fed by the composition
  root, not auto-populated from `PluginRegistry::resources()`. Wiring a
  type-erased plugin → registrar bridge is a cross-crate follow-up.
  Trigger: a plugin ships resources that must register without explicit
  composition-root wiring.
- **`RotationOutcome` → eventbus emission.** The fan-out aggregates a
  `RotationOutcome`; publishing it on `nebula-eventbus` for
  metrics/alerting (not audit — ADR-0028 §4) is the remaining wire-up.
  Trigger: a dashboard/alerting consumer subscribes.
- **No production `ResourceRepo` impl yet.** Cross-tenant isolation in
  the API holds because every by-id handler routes through a single
  audited `fetch_owned_resource`. Any production `ResourceRepo` impl
  **must** keep `get` keyed by id only (never `(workspace, id)`), or the
  isolation argument changes. Before any production `ResourceRepo`
  ships, cross-tenant isolation **must** be verified with automated
  tests that exercise `fetch_owned_resource` and the by-id handlers
  against a foreign-workspace row (a foreign / soft-deleted / unparsable
  target must collapse to an indistinguishable 404). Trigger: a concrete
  `ResourceRepo` lands.
- **Update version ownership.** `ResourceRepo::update` is pinned to a
  post-CAS version contract in its doc. Trigger: a backend implements
  `update` and must honour that contract.

## Consequences

- **Breaking, by design.** The `#[credential]` field type changes from
  bare `CredentialGuard<C>` (ADR-0044's now-amended migration note) to
  `SlotCell<CredentialGuard<C>>`, and reads move from `&self.<field>`
  to the derive-generated `self.<field>_slot()`. The hook signature
  changes from `&mut self` to `&self` + `&Self::Runtime` and gains a
  paired `on_credential_revoke`. All in-tree resource impls and the
  per-crate README/design-doc contract prose are re-touched in one pass
  — no deprecated alias, no codemod (there are no external consumers).
- The acquire hot path takes **no** new lock: the rejected
  `Arc<RwLock<R>>` consequence of ADR-0044's `&mut self` is avoided
  entirely; slot reads are lock-free `ArcSwap` loads.
- `nebula-resource` `Engine-integration` MATURITY flips
  `partial → stable` only after the full workspace verification gate
  passes with the per-slot rotation fan-out, the JSON/typed
  registration bridge, and the read-only API surface landed (ADR-0028
  §5 operational honesty — no early flip).
- The new state/error/hot path ships with typed `Error` variants
  (`Revoked` / `Ambiguous` → `Conflict`), a `tracing` span carrying
  `key`/`slot`/`topology`/duration but **never** credential material, a
  credential-data-free `ResourceEvent` family, and a redaction gate
  test that injects a secret and asserts no leak across
  spans/events/metrics/errors (ADR-0030 §4).

## Supersession

This ADR **amends** ADR-0044 — it does not retire it. ADR-0044's core
(delete `type Credential`; declarative `#[credential(key = …)]` slot
*binding declaration* model) stands unchanged. Two specific elements of
ADR-0044 are overridden:

| Amended ADR-0044 element | Replacement (this ADR) |
|---|---|
| Refresh hook `on_credential_refresh(&mut self, slot_name)` | `on_credential_refresh(&self, slot_name, &Self::Runtime)` + paired `on_credential_revoke`; reaction acts on the runtime's interior mutability, never `&mut self` |
| Migration-note credential field `#[credential] auth: CredentialGuard<C>` (framework-written before `create`) | `#[credential] auth: SlotCell<CredentialGuard<C>>` + derive-generated `auth_slot() -> Option<Arc<CredentialGuard<C>>>`; framework populates/rotates via `SlotCell::store` |

It also overrides the `.ai-factory/PHASE4_BLOCKED.md §1` candidate to
re-add rotation orchestration to `resource::Manager`: orchestration
stays engine-owned per ADR-0030.
