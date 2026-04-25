---
id: 0037
title: daemon-eventsource-engine-fold
status: proposed
date: 2026-04-25
supersedes: []
superseded_by: []
tags: [resource, engine, daemon, eventsource, topology, extraction, breaking-change, canon-3.5]
related:
  - docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md
  - docs/PRODUCT_CANON.md#35-architectural-vocabulary
  - docs/INTEGRATION_MODEL.md
  - crates/resource/src/runtime/daemon.rs
  - crates/resource/src/runtime/event_source.rs
  - crates/resource/src/runtime/managed.rs
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md
linear: []
---

# 0037. Daemon and EventSource topology — engine fold

## Status

**Proposed** at Phase 5 of the nebula-resource redesign cascade. Acceptance gates on Tech Spec CP1 ratification (Phase 6), matching [ADR-0036](./0036-resource-credential-adoption-auth-retirement.md) gating.

Records the secondary structural decision from the [nebula-resource redesign Strategy](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) §4.4 (frozen 2026-04-24) — the two non-resource topologies (Daemon, EventSource) leave `nebula-resource` and fold into the engine layer rather than a sibling crate. Sibling decision to ADR-0036 in the same cascade.

**Cross-cascade coordination:** none. Engine-side landing site (module layout, primitive naming) is Phase 6 Tech Spec §13 deliverable; this ADR commits to the *target layer*, not the layout.

## Context

### What was wrong

**`nebula-resource` ships seven topologies; two fall outside canon §3.5** ([Strategy §1.2](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), [pain-enumeration §1.2 / §1.6 / §2.4, 🔴-2 / 🔴-6 / 🟠-9](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).

Topologies in trunk: Pool, Resident, Service, Transport, Exclusive, EventSource, Daemon. [`PRODUCT_CANON.md §3.5`](../PRODUCT_CANON.md) line 80 defines Resource as "long-lived managed object (connection pool, SDK client). Engine owns lifecycle." Daemon (long-running worker) and EventSource (event-driven ingress) do not fit.

**Daemon has no public start path.** `Manager::register(daemon)` succeeds; `DaemonRuntime::start()` is reachable only via `ManagedResource.topology`, declared `pub(crate)` at [`runtime/managed.rs:35`](../../crates/resource/src/runtime/managed.rs). Phase 1 evidence: zero `Manager`-level integration tests; three `DaemonRuntime` unit tests bypass `Manager`. No `register_daemon` helper exists — workspace grep across `manager.rs` and `lib.rs` returns zero hits.

**EventSource is a thin wrapper without lifecycle to exercise.** [`runtime/event_source.rs`](../../crates/resource/src/runtime/event_source.rs) is 75 lines delegating to `Resource::subscribe` / `Resource::recv`. No orchestration beyond what `Resource` already provides. Same orphan-surface profile: zero `Manager`-level tests, no `register_event_source` helper.

**TriggerAction substrate already exists in the engine.** [`PRODUCT_CANON.md §3.5`](../PRODUCT_CANON.md) line 82 enumerates `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`. [`INTEGRATION_MODEL.md:99`](../INTEGRATION_MODEL.md) confirms engine "dispatches by which action trait the type implements." Event-driven ingress is already an engine concept; EventSource maps onto `TriggerAction` via a thin adapter.

### What forced the decision

- **Canon §3.5 alignment** ([Strategy §2.2](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)). Keeping Daemon + EventSource in-crate perpetuates a false-capability profile — the crate's name claims "resource = pool/SDK client" while two of its seven topologies are not.
- **Atomic landing.** ADR-0036 commits to a bundled breaking-change PR wave migrating five in-tree consumers ([Strategy §4.8](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)). Daemon/EventSource extraction must travel with that wave or consumers absorb topology breaks twice.
- **Two extraction targets exist** ([Strategy §4.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), [scope-decision §4.6](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)): (a) fold into engine — Daemon as new engine primitive, EventSource as adapter onto existing `TriggerAction`; (b) extract to a sibling crate (`nebula-worker` / `nebula-background` / `nebula-scheduler`).

## Decision

Fold Daemon and EventSource into the engine layer (option (a)). Specifics per [Strategy §4.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md):

- Daemon trait + `DaemonRuntime` move from `crates/resource/src/runtime/daemon.rs` (493 LOC) into `crates/engine/`. Conceptually a `DaemonRegistry` parallel to existing action dispatch.
- EventSource trait + `EventSourceRuntime` move from `crates/resource/src/runtime/event_source.rs` (75 LOC) into engine, mapped onto the existing `TriggerAction` substrate via an EventSource→Trigger adapter.
- The `TopologyRuntime` enum on `ManagedResource` ([`runtime/managed.rs:35`](../../crates/resource/src/runtime/managed.rs)) loses its `Daemon` and `EventSource` variants; the enum shrinks 7 → 5.
- `nebula-resource` retains zero references to `DaemonRuntime` / `EventSourceRuntime` post-extraction. Canon §3.5 honored.
- Existing tests (3 `DaemonRuntime` unit tests; no `Manager`-level tests) migrate alongside the implementation.

This ADR commits to the *target layer*, not the *module layout*. Phase 6 Tech Spec §13 produces the file paths, primitive name (`DaemonRegistry` / `WorkerRuntime` / etc.), the EventSource→Trigger adapter signature, and per-consumer migration steps.

## Consequences

### Positive

- **Canon §3.5 alignment restored.** `nebula-resource` is purely "long-lived managed object" — crate name matches contents.
- **`TopologyRuntime` enum simplifies 7 → 5.** Smaller dispatch surface, narrower `Manager` API, fewer combinatorial `where` bounds in `acquire_*` paths ([pain-enumeration §2.3 🟡](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).
- **Engine surface gains coherent worker/event substrate.** Daemon lands alongside existing `TriggerAction` and control-queue dispatch; reviewer tracing a `Cancel` through control-queue versus a `Daemon::start` through the new primitive sees mechanically symmetric shapes. Same boundary logic as [ADR-0030 §7](./0030-engine-owns-credential-orchestration.md) ("rotation orchestration lives next to execution orchestration") — extended, not new.
- **Atomic consumer migration with the trait reshape.** Same PR wave that lands ADR-0036 carries this extraction. Five in-tree consumers (`nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox`) absorb one structural break, not two. Phase 1 evidence (zero `Manager`-level Daemon/EventSource tests) suggests extraction is mostly removal, not rewrite.

### Negative

- **Engine surface grows by ~568 LOC** (493 daemon + 75 event_source) plus `ManagedResource` wiring. Engine compile time and binary size increase. Accepted: same posture as [ADR-0030 negative consequence #1](./0030-engine-owns-credential-orchestration.md) — engine is the layer that owns runtime work; the cost lives there.
- **No `nebula-worker` / `nebula-scheduler` precedent.** [Strategy §4.4 rationale (1)](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) verified workspace `Cargo.toml` lists 23 top-level non-macro crates; none is scheduler/worker/background-shaped. Creating one for two small concepts introduces a crate-level boundary with zero adopters — `feedback_boundary_erosion.md` cuts both ways. Sibling-crate remains available via §5.1.
- **Potential engine-focus dilution.** If Daemon code grows heavyweight, or non-trigger workers proliferate, engine's focus weakens. [Strategy §5.1](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) records the escape valve: "Daemon-specific engine code grows beyond ~500 LOC OR non-trigger long-running workers proliferate beyond 2." This ADR does NOT preclude a future split — it locks the immediate target while documenting the revisit conditions.
- **Five-consumer import-path updates.** Per Phase 6 Tech Spec §13. Phase 1 evidence (zero `Manager`-level tests) suggests the actual call-site surface is small.

### Neutral

- **Strategy-level decision, not Phase 6 implementation.** Tech Spec §13 produces file paths, primitive naming, adapter signatures, per-consumer steps. Same framing as ADR-0036.
- **Future cascade preserved.** §5.1 escape valve means a future cascade can spin out `nebula-scheduler` from engine without re-routing through `nebula-resource`. [Strategy §6.5](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) flags this as future-cascade material.
- **Security-neutral.** [Phase 2 security-lead review](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) endorsed Option B with three amendments (B-1 isolation, B-2 revocation, B-3 warmup) — none touched topology choice. Credential-bearing topologies (Pool, Resident, Service, Transport, Exclusive) all stay in `nebula-resource`.

## Alternatives considered

### Alternative 1 — Sibling crate (`nebula-worker` / `nebula-background` / `nebula-scheduler`)

Daemon + EventSource extracted to a dedicated sibling crate alongside `nebula-engine`.

**Rejected** at [Strategy §4.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) for three reasons:

- **No workspace precedent.** `Cargo.toml` lists 23 top-level non-macro crates; none is scheduler/worker/background-shaped. A new crate for two small concepts (493-LOC + 75-LOC) creates a crate boundary with zero adopters. `feedback_boundary_erosion.md` cuts both ways — extracting to a brand-new crate without adopters is itself boundary erosion.
- **TriggerAction precedent in engine.** Engine already dispatches event-driven trigger lifecycles per [`PRODUCT_CANON.md §3.5`](../PRODUCT_CANON.md) line 82 and [`INTEGRATION_MODEL.md:99`](../INTEGRATION_MODEL.md). Splitting EventSource into a sibling crate makes the EventSource→Trigger adaptation cross an extra boundary for no benefit.
- **Atomic migration is simpler with engine-fold.** Sibling crate means new `Cargo.toml` updates across all five consumers + new crate publishing concerns. Engine-fold reuses existing `nebula-engine` dep — consumers update import paths, not dependency lists.

§5.1 preserves sibling-crate as future-cascade material if engine surface grows uncomfortably.

### Alternative 2 — Keep Daemon + EventSource in `nebula-resource`

Status quo. Fix Daemon's `pub(crate)` start-path gap by exposing `register_daemon` / `register_event_source` on `Manager`.

**Rejected** because:

- **Canon §3.5 violation persists.** Adding public start helpers means the crate continues to mis-describe its contents — a more entrenched false-capability surface, not less.
- **Public-API completion inflates an already-overloaded crate.** Fixing the [pain-enumeration §1.2 🔴-2](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) gap publicly within `nebula-resource` adds API surface to a crate the redesign is shrinking (7→5 topologies, `_with` cleanup, doc rewrite).
- **Phase 1 findings 🔴-2 + 🔴-6 + 🟠-9 all converge on extraction.** Three independent reviewers (dx-tester, tech-lead, rust-senior) reached the same conclusion.

### Alternative 3 — Selective extraction (Daemon → engine, EventSource → keep)

Daemon leaves; EventSource (75 LOC, thin wrapper) stays.

**Rejected** because:

- **Same §3.5 misalignment for EventSource.** Size is not a license to keep an out-of-band concept in-crate.
- **Same orphan-surface profile.** Both have zero `Manager`-level tests and no `register_*` helper. A split adds an API decision rather than removing one.
- **TriggerAction adaptation is cleaner co-located with TriggerAction.** A split keeps EventSource on the wrong side of the substrate seam.

## References

- [nebula-resource redesign Strategy](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) — §1.2 (problem), §2.2 (canon §3.5 constraint), §3.2 (Option B), §4.4 (decision), §5.1 (revisit trigger), §6.5 (future-cascade flagged).
- [Phase 1 pain enumeration](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — §1.2 (Daemon orphan, 🔴-2), §1.6 (EventSource same pattern, 🔴-6), §2.4 (tech-lead canon analysis, 🟠-9).
- [Phase 2 scope decision](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) — §4.6 (extraction target options); tech-lead priority-call ("Extract Daemon + EventSource. Keep Pool / Resident / Service / Transport / Exclusive.").
- [`PRODUCT_CANON.md §3.5`](../PRODUCT_CANON.md) line 80 (resource = pool/SDK client) and line 82 (action trait family includes `TriggerAction`).
- [`INTEGRATION_MODEL.md:99`](../INTEGRATION_MODEL.md) (engine dispatches by action trait — TriggerAction substrate precedent).
- [ADR-0030 — engine owns credential orchestration](./0030-engine-owns-credential-orchestration.md) — boundary precedent this ADR extends; engine as runtime-orchestration layer.
- [ADR-0036 — `Resource::Credential` adoption + `Auth` retirement](./0036-resource-credential-adoption-auth-retirement.md) — sibling decision in the same redesign cascade; same Phase 5 + same Tech Spec CP1 acceptance gate.
- Topology source files: [`crates/resource/src/runtime/daemon.rs`](../../crates/resource/src/runtime/daemon.rs) (493 LOC), [`crates/resource/src/runtime/event_source.rs`](../../crates/resource/src/runtime/event_source.rs) (75 LOC), `pub(crate)` topology field at [`crates/resource/src/runtime/managed.rs:35`](../../crates/resource/src/runtime/managed.rs).

## Review

Ratified through the Phase 2 + Phase 3 co-decision protocol of the redesign cascade:

- **architect** — drafts this ADR (Phase 5, follow-up to ADR-0036).
- **tech-lead** — Phase 2 priority-call: *"Extract Daemon + EventSource. Keep Pool / Resident / Service / Transport / Exclusive."* ([pain-enumeration §7 / Phase 2 input](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). Phase 3 CP2 ratification: strong endorse engine-fold over sibling crate ([Strategy §4.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)).
- **security-lead** — Phase 2 ENDORSE Option B with three amendments (B-1 isolation, B-2 revocation, B-3 warmup); none touched topology choice. Engine-fold is security-neutral — credential-bearing topologies (Pool, Resident, Service, Transport, Exclusive) all stay in `nebula-resource`.

Acceptance gate: this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the engine-side landing site (module layout, primitive name, EventSource→TriggerAction adapter signature) against the target layer recorded above. Same gating posture as ADR-0036.

### Amended in place on

(empty on first draft; future amendments listed here per the ADR-0035 / ADR-0036 amended-in-place pattern.)
