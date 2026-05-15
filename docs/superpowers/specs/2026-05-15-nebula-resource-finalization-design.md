# nebula-resource finalization — design spec

> Status: **APPROVED for planning** (panel verdict D1/D2/D3 accepted by owner 2026-05-15).
> Scope: close §M11.5 (per-slot rotation fan-out) + §M12.4 (frontier→stable) end-to-end,
> plus the JSON/typed registration bridge and the HTTP API surface.
> Authority lineage: PRODUCT_CANON §3.5/§11.4/§13.1/§13.2, INTEGRATION_MODEL §3.6/§114-120,
> ADR-0028/0030/0036/0043/0044/0047/0051, `.ai-factory/PHASE4_BLOCKED.md`, `.ai-factory/ROADMAP.md §M6/§M11.5/§M12.4`.

## 1. Problem

`nebula-resource` has **no inline stubs** (`grep` clean: no `TODO`/`unimplemented!`/`panic!`),
81 unit tests green, 5 topologies + `Manager` + `ResourceGuard` + `ReleaseQueue` authoritative.
"No markers" is **not** "finalized". The canon and ROADMAP track four unclosed pieces:

1. **§M11.5** — per-slot credential rotation: the trait hook
   `on_credential_refresh` exists but **nothing invokes it**. The entire
   reverse-index + fan-out subsystem (`manager/rotation.rs`,
   `credential_resources` DashMap, `ResourceEvent::CredentialRefreshed/Revoked`,
   `OutcomeBoundCounters`, `RefreshOutcome/RevokeOutcome/RotationOutcome`) was
   **deleted** in Phase 4 because every signature referenced the now-removed
   `R::Credential` projection (`PHASE4_BLOCKED.md §1`).
2. **Engine-side `kind → registrar` bridge** — the resource-side
   `Manager::register_from_value<R>(json, expr_engine, slot_bindings, …)` is
   **verified implemented** (`manager/mod.rs:611-681`: template resolve +
   schema validate + deserialize + dispatch to typed `register`) —
   `PHASE4_BLOCKED.md §2`'s "stubbed" note is **stale** (landed since). The
   real remaining gap is the engine-side erased `kind: &str → registrar`
   indirection from `PluginRegistry` so a stored `ResourceEntry.kind` string
   reaches the right typed `register_from_value::<R>`.
3. **API** — `crates/api/src/handlers/resource.rs` is a 501 stub per ADR-0047;
   `ResourceRepo`+`ResourceEntry` exist in `crates/storage/src/repos/resource.rs`
   but are **not wired into `AppState`**.
4. **§M12.4 frontier→stable** — plan-file audit, honest MATURITY flip
   (ADR-0028 §5), topology docstring cleanup (`PHASE4_BLOCKED.md §4`).

### 1.1 Authority topology & known gaps (verified 2026-05-15)

Per `docs/adr/README.md:5`: the parent **`C:/Users/vanya/RustroverProjects/docs/`**
holds the L1 canon — `PRODUCT_CANON.md`, `INTEGRATION_MODEL.md`, `MATURITY.md`,
ADR `0001..0041` — **and** historical cascade logs (`tracking/`, `plans/`,
`research/`). The worktree **`docs/`** holds the *current* cascade ADRs
`0042..0051` + `audits/` + `superpowers/specs/`. Consequence for this spec:

- `tracking/nebula-resource-concerns-register.md` is the **2026-04-24
  ADR-0036-cascade log, pre-ADR-0044/Phase-4**. Its `R-002/R-003/R-004/R-060`
  "landed П2" rows describe the rotation machinery that Phase 4 then
  **deleted** (`PHASE4_BLOCKED.md §1`) — they are **not** current truth and are
  not used as authority here (the owner flagged this tree as "old logs").
- The master plan `m6-resource-finalization-integration-audit.md` is
  referenced by `docs/adr/README.md:29`, `PHASE4_BLOCKED.md`, and ROADMAP but
  is **absent** from the repo (verified: `find` + empty `.ai-factory/plans/`).
  Surviving authority for "done" = ROADMAP §M11.5/§M12.4 + `PHASE4_BLOCKED.md`
  + the stale-aware concerns register + the frozen tech spec
  (`docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`). The plan
  phase must not block on the missing master plan.

## 2. Scope

In scope: Tracks **A+B+C+D** (full §M11.5 + §M12.4 close + bridge + API).

Non-goals (explicit):
- No `cargo-nebula-migrate-resource` codemod (YAGNI — no external consumers; the
  ~33 internal impl sites are re-touched in-pass under ADR-0052; `feedback_no_shims`).
- No HTTP acquire/release/drain endpoints (engine-owned lifecycle, §11.4 — see D3).
- No ADR-0042 numbering-collision fix (`docs/adr/0042-layered-retry.md` +
  `docs/adr/0042-node-binding-mechanism.md` + canon `0042-tool-provider`) — flagged
  out-of-scope, separate task.
- ToolProvider (ADR-0042 PROPOSED, `unstable-*`) — untouched.

## 3. Panel verdict (SOLID-defended, adversarially survived)

### D1 — Reverse-index + fan-out lives in `nebula-engine`, not `resource::Manager`

- **SRP**: `resource::Manager`'s single responsibility is resource lifecycle
  (register/acquire/health/release/shutdown). Credential-rotation orchestration
  (when, fan-out, timeout budgets, outcome aggregation, cross-replica coalescing)
  is a different responsibility that ADR-0030 ("engine owns credential
  orchestration") already placed in engine. Re-adding it to `resource::Manager`
  (as `PHASE4_BLOCKED.md §1` *proposes* — a phase-note, not an ADR) re-violates
  ADR-0030 and gives `Manager` two reasons to change.
- **DIP**: engine (Exec layer) depends on a narrow port exposed by resource
  (Business layer); resource does not absorb credential-rotation internals.
  No `nebula-resource → nebula-engine` edge is introduced (engine already holds
  `Arc<nebula_resource::Manager>` at `crates/engine/src/engine.rs:49` and owns
  `crates/engine/src/credential/`). No layer cycle.
- **Survived counter (locality)**: "Manager already holds the ManagedResource
  map; routing through engine adds indirection." Rejected: rotation is rare
  (per expiry window, cross-replica-coalesced per ADR-0030/0041);
  `engine→manager.refresh_slot` is one typed call of the same class as
  `engine→manager.acquire` — no extra hop; the SRP/DIP win dominates.

Narrow port exposed by `resource::Manager`:

```rust
impl Manager {
    /// Engine-driven. Apply a rotated/revoked slot to the live resource.
    /// Reentrancy model per D2. Idempotent; returns typed Error (Classify).
    pub async fn refresh_slot(
        &self, key: &ResourceKey, scope: ScopeLevel, slot_name: &str,
    ) -> Result<(), crate::Error>;

    pub async fn revoke_slot(
        &self, key: &ResourceKey, scope: ScopeLevel, slot_name: &str,
    ) -> Result<(), crate::Error>;
}
```

### D2 — Hook signature → `&self` + interior-mutable slots (ArcSwap) + `&Self::Runtime`; **supersedes ADR-0044 hook signature**

- The resource impl object `Self` holds **only** slot fields + static config
  (ADR-0043 §5: per-execution data is in `Self::Input`, not on `self`). The
  reaction to rotation (blue-green pool swap) acts on the **live
  `Self::Runtime`**, not on the factory object. ADR-0036 itself states
  blue-green is "internalised by the resource impl … owns its
  `Arc<RwLock<Pool>>` write-lock window" — the swap point lives in `Runtime`.
- ADR-0044's `&mut self` is therefore a modeling error: it conflates
  "immutable factory descriptor" with "mutable rotation target" (SRP), and
  forces `Arc<RwLock<R>>` on **every** `ManagedResource` → a lock on the
  acquire hot path for all resources to support a hook that, under correct
  blue-green, does not need `&mut self`. `PHASE4_BLOCKED.md §1.2` itself flags
  this as an unresolved trade-off.
- **Corrected trait shape** (in `crates/resource/src/resource.rs`):

  ```rust
  /// Default no-op. Called by the engine fan-out (D1) after it has swapped the
  /// rotated credential into this resource's interior-mutable slot.
  /// `&self`: the impl object is an immutable descriptor. Blue-green / re-auth
  /// acts on `runtime`'s own interior mutability (e.g. ArcSwap<Pool>).
  fn on_credential_refresh(
      &self,
      slot_name: &str,
      runtime: &Self::Runtime,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send {
      let _ = (slot_name, runtime);
      async { Ok(()) }
  }

  /// Default: no-op. Post-invocation invariant: the resource emits no further
  /// authenticated traffic on the revoked credential (ADR-0036).
  fn on_credential_revoke(
      &self,
      slot_name: &str,
      runtime: &Self::Runtime,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send {
      let _ = (slot_name, runtime);
      async { Ok(()) }
  }
  ```

- Credential slot fields become interior-mutable so the engine can swap the
  resolved credential without `&mut`. `arc-swap` is already a dependency
  (used for `Cell<T>`). The slot wrapper used by `#[credential]` fields
  (`CredentialGuard<C>` / `Option<…>` / `Lazy<…>`) is stored behind a per-slot
  `ArcSwap` inside `ManagedResource`; the resource reads the current value
  through an accessor, never a bare `&mut` field.
- Rejected sub-option (hybrid "opt-in RwLock if author declared a mut field"):
  discipline-based escape hatch for a case the corrected model does not
  produce; complicates the derive macro for dead weight
  (`feedback_type_enforce_not_discipline`).
- **Survived counter (churn)**: "ADR-0044 is freshly accepted in this same
  epic." Rejected: ADR-0044's core (drop `type Credential`, slot-binding) is
  correct and untouched; only the reentrancy modeling — which
  `PHASE4_BLOCKED.md §1.2` left explicitly open — is corrected. User authorized
  breaking changes for correctness. Recorded in **ADR-0052**.

### D3 — API surface = config CRUD (write) + read-only status/health/metrics (read); no lifecycle over HTTP

- §11.4: acquire/release are engine-owned. §13.1: lifecycle must be
  attributable in durable journal / operator trace — an observability
  projection, not an HTTP mutation surface.
- CQRS split: write = `ResourceEntry` config CRUD (validated against
  `R::Config` schema via Track B); read = list/get config + a status
  projection (phase/health/ops-metrics from `Manager` + metrics registry).
- Exposing acquire/release over HTTP = confused-deputy ("acquire arbitrary
  resource") + SRP violation (API owning lifecycle it does not own). Matches
  the existing workflow/credential CRUD pattern in `crates/api`.

## 4. Abuse-case invariants (security review before design freeze)

| # | Abuse | Invariant enforced by this design |
|---|---|---|
| 1 | Cross-tenant dedup: `ResourceConfig::fingerprint()` **defaults to `0`** (`resource.rs:64-66`) — every config of a type collapses to one runtime regardless of resolved credential → cross-tenant bleed | **Confirmed bug.** The dedup key is fixed **structurally at the `Manager` level**: the dedup tuple becomes `(R::key(), ScopeLevel, slot-identity-hash)` where slot-identity-hash is derived from the resolved `CredentialKey`/`CredentialId` per `#[credential]` slot — **independent of the author's `fingerprint()`** (relying on authors to override `fingerprint()` is discipline-based and rejected, `feedback_type_enforce_not_discipline`). Test: two scopes, same `R`, default `fingerprint()`, different credential per slot ⇒ two distinct runtimes. |
| 2 | Revoke race on shared runtime (ADR-0036: zero authenticated traffic post-revoke) | Ordering, engine-driven: engine marks credential `revoking` → `Manager::revoke_slot` taints the runtime (reject new acquires via existing guard `tainted`) → drains in-flight `ResourceGuard`s via `ReleaseQueue` + `drain_tracker` → reports → engine completes revoke. No new acquire observes the revoked credential. |
| 3 | Secret in config JSON via API (ADR-0028 §7) | `register_from_value(json)` validates against `<R::Config as HasSchema>::schema()`; `ResourceConfig` carries no secrets by §3.5 (slots are credential *references* by key/id). API DTOs use ADR-0047 wrappers — zero `nebula-core`/`engine`/`storage` types in the wire schema. |
| 4 | Type confusion `kind:String → R` in `register_from_value` | `kind → registrar` is a **closed allowlist** built from `PluginRegistry` (INTEGRATION_MODEL §114-120 closed dependency graph), never reflection. Unknown `kind` ⇒ typed activation error, never a silent runtime grab. |

## 5. Target architecture per track

### Track A — §M11.5 per-slot rotation (engine-owned fan-out)

**`nebula-resource` changes:**
- **Slot-storage substrate (verified absent today).** `#[derive(Resource)]`
  currently emits only `DeclaresDependencies` + a `todo!()` `create` body
  (`macros/src/resource.rs:60-94`); **nothing stores a resolved
  `CredentialGuard<C>` on a resource instance at runtime** — the "slots
  already populated on `&self`" comment in `resource.rs` has no implementing
  code. Track A builds the substrate: a per-slot `Cell<CredentialGuard<C>>`
  (the existing lock-free `ArcSwapOption` cell, `cell.rs:10-46`) stored on
  `ManagedResource`, plus a generated typed accessor. `CredentialGuard` is
  `!Clone` + `Drop`-zeroizing (`credential/src/secrets/guard.rs:36-64`) — the
  cell holds `Arc<CredentialGuard<C>>` so swap does not clone secret material.
- `resource.rs`: replace `on_credential_refresh(&mut self, slot_name)`
  (`resource.rs:289-295`) with the D2 `&self` + `&Self::Runtime` pair
  (`on_credential_refresh` + `on_credential_revoke`), async default no-op.
  Update `#[derive(Resource)]` (`macros/src/resource.rs`,
  `macros/src/field_slots.rs:180-208`) to emit the slot cells + accessor;
  no `&mut self` anywhere in generated code.
- `manager/mod.rs`: add `refresh_slot` / `revoke_slot` (the D1 port). They
  resolve `(key, scope)` to the live `ManagedResource`, dispatch through
  `TopologyRuntime` to invoke the hook with `&Self::Runtime`. Per-topology:
  Pooled fans the hook across pooled instances (or blue-green-swaps the pool
  wholesale per author design); Resident/Service/Transport/Exclusive invoke
  once on the shared runtime.
- `events.rs`: reinstate rotation events in the new (credential-data-free)
  form: `ResourceEvent::SlotRefreshed { key, slot }`,
  `ResourceEvent::SlotRevoked { key, slot }`,
  `ResourceEvent::SlotRefreshFailed { key, slot, error }`. No `CredentialId`
  in the payload (engine owns that mapping).
- `error.rs`: add typed variants for slot-refresh failure paths; keep the
  existing `nebula_error::Classify` impl coherent (category mapping).
- Dedup-key fix (abuse #1): `Manager` register/acquire path
  (`manager/mod.rs:265,423` `config.fingerprint()` call sites +
  `runtime/pool.rs:138` `current_fingerprint`) — the dedup key gains a
  structural slot-identity component derived from resolved `CredentialKey`
  per slot, independent of the (default-`0`) author `fingerprint()`.
- DoD per operation: typed `Error` variant + `tracing` span
  (`nebula.resource.slot_refresh`, fields: `key`, `slot`, `topology`,
  duration; **never** credential material) + `ResourceEvent` + metrics
  (`ResourceOpsMetrics` gains `slot_refresh_total` / `slot_refresh_error_total`).

**`nebula-engine` changes:**
- New module `crates/engine/src/credential/rotation/resource_fanout.rs`
  (sibling of the existing `scheduler.rs`/`grace_period.rs`/`blue_green.rs`/
  `transaction.rs`/`token_refresh.rs` per ADR-0030 §1):
  - Reverse index `DashMap<CredentialId, SmallVec<[(ResourceKey, ScopeLevel, SlotName); 2]>>`,
    populated when the engine resolves a credential into a resource slot at
    `Resource::create` time, drained on resource removal/shutdown.
  - On a credential rotation event (from the ADR-0030 scheduler) or a
    lease-revoke (ADR-0051 `LeaseEvent`), look up affected resources and
    `futures::future::join_all` over `manager.refresh_slot/revoke_slot` with a
    **per-resource timeout budget** (ADR-0036 invariant — never a single
    global timeout; one slow resource must not cascade-fail siblings).
  - Aggregate into a `RotationOutcome` summary; emit via `nebula-eventbus`
    (metrics/dashboard fanout only — not a substitute for any audit write,
    ADR-0028 §4).
- Subscriptions wired via `nebula-eventbus` (AGENTS.md: cross-crate comms
  through eventbus, not direct sibling imports).

**Data flow (rotation):**
```
credential expiry  → engine ADR-0030 scheduler → engine resolves new material
                   → engine swaps material into resource slot (ArcSwap)
                   → engine resource_fanout: index lookup CredentialId
                   → join_all[ per (key,scope,slot): manager.refresh_slot
                          (per-resource timeout) → Resource::on_credential_refresh
                          (&self, slot, &Runtime) → author blue-green on Runtime ]
                   → RotationOutcome → eventbus (metrics/alerts)
```
**Data flow (revoke):** engine marks `revoking` → `manager.revoke_slot` taints
runtime (reject new acquires) → drain in-flight guards (`ReleaseQueue` +
`drain_tracker`) → `Resource::on_credential_revoke` → report → engine completes
revoke. Invariant: no acquire after taint observes the revoked credential.

### Track B — JSON/typed registration bridge

- `nebula-resource`: **verified already implemented** —
  `Manager::register_from_value<R>(config_json, expr_engine, slot_bindings,
  resource, scope, topology, resilience, recovery_gate)` at
  `manager/mod.rs:611-681` already resolves `{{ … }}` templates via
  `nebula_expression`, validates against `<R::Config as HasSchema>::schema()`,
  deserializes, and dispatches into typed `register()`. Track B's
  resource-side work is limited to: (a) a regression test for secret-shaped
  config rejection (abuse #3) if not already covered, (b) confirming
  `slot_bindings: HashMap<String, CredentialKey>` is the seam the engine
  reverse-index (Track A) keys off. No re-implementation.
  Authority: `deny.toml:121-133` already whitelists the
  `nebula-resource → nebula-expression` edge with the reason "ADR-0043 §9 /
  Phase 9: register_from_value bridges resource config to expression engine for
  `{{ }}` template resolution" — this design is the sanctioned closure of that
  pre-declared edge, not a new dependency.
- `nebula-engine`: erased `ResourceRegistrar` trait + a registry
  `kind: &str → Arc<dyn ErasedResourceRegistrar>` built from `PluginRegistry`
  (closed dependency graph, abuse #4). `ErasedResourceRegistrar::register`
  takes the `ResourceEntry.config` JSON and calls the right
  `register_from_value::<R>` for that plugin-declared type. Unknown `kind` ⇒
  typed activation error.

### Track C — API

- `crates/api/src/state.rs`: add `pub resource_repo: Arc<dyn ResourceRepo>`
  (port trait from `crates/storage/src/repos/resource.rs`) + builder
  `with_resource_repo`. Optional like other registries; the composition root
  (`nebula-server`) injects the concrete impl.
- `crates/api/src/handlers/resource.rs`: replace the 501 stub with:
  - `list_resources` (GET, paginated `ListResourcesResponse`),
  - `get_resource` (GET one),
  - `create_resource` (POST — body validated against the target `R::Config`
    schema through the Track B bridge; CAS `version`),
  - `update_resource` (PUT — `expected_version` CAS),
  - `delete_resource` (DELETE — `soft_delete`),
  - `get_resource_status` (GET `…/resources/{id}/status` — read model:
    `ResourcePhase`/health/`ResourceOpsSnapshot` from `Manager` +
    metrics registry; **no** secret/credential material).
  Drop `#[deprecated]` and the 501 response; honest `#[utoipa::path]` per
  ADR-0047. Error mapping: 401/403/404/409(version)/422(schema)/500 →
  `ProblemDetails` (RFC 9457, existing `ApiError`).
- `crates/api/src/models/resource.rs`: extend DTOs (status projection,
  pagination); ADR-0047 wrappers only (no core/engine/storage types).
- `crates/api/src/routes/workspace.rs`: register the handlers via
  `routes!(…)` (utoipa-axum) so the OpenAPI spec stays drift-checked.

### Track D — frontier→stable (§M12.4)

- Audit `crates/resource/plans/*.md` non-SUPERSEDED set (01-core, 02-topology,
  03-infrastructure, 04-recovery-resilience, 05-manager, 07-implementation,
  08-correctness, 09-topology-guide): mark closed items, fold residue into
  this spec's tracks or explicitly defer with a marker.
- Topology docstring cleanup (`PHASE4_BLOCKED.md §4`): remove stale
  scheme-threading references in `crates/resource/src/topology/*`.
- **Concerns-register reconciliation** (`RustroverProjects/docs/tracking/nebula-resource-concerns-register.md`,
  stale Apr-24 ADR-0036 log — see §1.1):
  - Already closed, no action: R-013/R-030..R-037/R-051 (landed post-cascade),
    R-040 (**verified resolved** — `deny.toml:108` has the `nebula-resource`
    wrapper rule).
  - Superseded by this spec's Tracks A/B/D: R-002/R-003/R-004/R-060 (the П2
    rotation machinery they reference was deleted Phase 4; Track A rebuilds it
    engine-side per D1, not in `resource::Manager`), R-020/R-021/R-053
    (manager file-split already done; `integration/` rename folded into
    Track A touch), R-043 (`DeclaresDependencies` trace wiring — covered by
    the Track A macro rework).
  - **Explicitly deferred with trigger markers** (register lifecycle rule 4;
    `feedback_incomplete_work` — not silently dropped): R-006
    (`AuthScheme: Clone` zeroize obligation — future-cascade, trigger:
    cross-crate credential reshape), R-041 (no `benches/`/CodSpeed —
    post-cascade, trigger: bench harness milestone), R-042 (zero feature flags
    — future-cascade, trigger: constrained-context requirement), R-050
    (5 assoc types combinatorial bounds — future-cascade, trigger: second
    consumer needing distinct shape), R-052 (`Resource::destroy` no-op leak —
    revisit post Track A). These are recorded in ADR-0052's "Deferred" section.
  - Register **retires** when the MATURITY flip below lands (its own
    close-condition; the register's "→ core" wording is the stale Strategy-§6.4
    term — the live taxonomy is `frontier → stable`, see next bullet).
- Honest MATURITY flip — authoritative file is the **parent**
  `C:/Users/vanya/RustroverProjects/docs/MATURITY.md:37` (the worktree has no
  `docs/MATURITY.md`; `crates/resource/README.md:162` points at it). Taxonomy
  is `frontier`/`stable` (NOT `core`). Flip the `nebula-resource`
  **Engine-integration column** `partial (lifecycle visible; CAS guards
  partial)` → `stable` (mirroring the honest `partial → stable` credential
  upgrade at `MATURITY.md:66`) **only after** Tracks A/B/C land and pass the
  verification gate (ADR-0028 §5 operational honesty — no early flip). This is
  a **parent-tree edit** outside the worktree — the plan must call it out as a
  distinct, gated step.
- **ADR-0052** "Engine-owned per-slot rotation fan-out + `&self` refresh hook":
  records D1+D2+D3, supersedes ADR-0044's hook signature, overrides
  `PHASE4_BLOCKED.md §1`'s "re-add to `resource::Manager`" candidate, and
  carries a **"Deferred" section** enumerating the concerns-register
  future/post-cascade items (R-006/R-041/R-042/R-050/R-052) with their trigger
  conditions so retirement of the stale register is traceable. Filed in the
  worktree `docs/adr/` (next free number is **0052** — verified highest
  existing is `0051`; the pre-existing `0042` collision —
  `0042-layered-retry.md` + `0042-node-binding-mechanism.md` + parent
  `0042-tool-provider` — is noted but out of scope).

## 6. Breaking changes

- `Resource::on_credential_refresh` signature change (`&mut self` →
  `&self` + `&Self::Runtime`) + new `on_credential_revoke`. All ~33 internal
  impl sites + the `#[derive(Resource)]` macro output updated in one pass
  (`feedback_bold_refactor_pace`). No deprecated alias (`feedback_no_shims`).
- `ManagedResource` internal: slot fields move behind per-slot `ArcSwap`
  (no public `&mut` slot access). Internal to the crate; not a consumer break
  beyond the trait signature.
- `ResourceEvent` gains `SlotRefreshed/SlotRevoked/SlotRefreshFailed`
  (`#[non_exhaustive]` already, so additive at the enum level).

## 7. Error handling

- All new failure paths return typed `crate::Error` (resource) /
  `thiserror` enums classified via `#[derive(nebula_error::Classify)]`
  (`#[classify(category=…, code=…)]`), matching the established
  `crates/credential/src/error.rs` pattern. No `unwrap`/`expect`/`panic!` in
  library code (clippy `-D warnings`; tests/const/bins exempt).
- Engine fan-out: per-resource error isolation; one resource's `Err` does not
  abort sibling dispatches; aggregated into `RotationOutcome`.
- Redaction: no credential/token material in any `tracing` span, event
  payload, metrics label, or error string (PRODUCT_CANON §12.5, ADR-0030 §4).

## 8. Testing strategy

- **Unit** (`nebula-resource`): `refresh_slot`/`revoke_slot` per topology;
  ArcSwap slot swap visibility; fingerprint includes slot identity
  (abuse #1 regression test); taint→drain ordering (abuse #2).
- **Macro** (`trybuild`): `#[derive(Resource)]` emits `&self` hooks, no
  `&mut self`; compile-fail probe for the old signature.
- **Integration** (`crates/engine/tests/`): rotation fan-out end-to-end
  (credential rotate → affected resources' hooks fire, unaffected do not);
  per-resource timeout isolation (one slow resource does not fail siblings);
  revoke ⇒ zero post-revoke authenticated acquire.
- **Redaction** (`crates/engine/tests/`): inject secret-bearing material into
  rotation path; assert no substring in spans/events/metrics/errors
  (ADR-0030 §4 CI gate pattern).
- **API**: handler tests for CRUD + status; OpenAPI drift is structural
  (utoipa-axum compile gate); schema-validation rejection of secret-shaped
  config (abuse #3); unknown `kind` ⇒ typed error (abuse #4).
- Existing `crates/engine/tests/resource_integration.rs::shared_resource`
  cross-workflow dedup test must stay green.

## 9. Verification gate

`task dev:check` (fmt + clippy `-D warnings` + nextest + doctests + deny)
green workspace-wide; `cargo doc` with broken-intra-doc-links denied green for
touched crates; `task build` examples green; `cargo deny` layer-wrapper check
green (no new cross-layer edge — engine→resource only). MATURITY row flips
only after this gate passes with Tracks A/B/C landed.

## 10. Open questions

None. D1/D2/D3 resolved by panel + owner; abuse invariants fixed; non-goals
explicit; doc-authority topology + the stale Apr-24 concerns register + the
missing master plan reconciled in §1.1 / Track D. Source-verified 2026-05-15:
MATURITY taxonomy is `frontier→stable`; `deny.toml:121-133` edge pre-declared;
R-040 resolved (`deny.toml:108`); `register_from_value` **already implemented**
(`manager/mod.rs:611-681`) so Track B is engine-side-only; slot runtime storage
**absent today** (Track A builds the substrate); `fingerprint()` defaults to
`0` (`resource.rs:64-66`) so abuse #1 is a confirmed bug fixed structurally at
the `Manager` dedup key. The 4 plan tracks are dependency-ordered: A
independent; B before C; D is closure (ADR-0052 + MATURITY + docs).
