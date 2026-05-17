# Plan — wire the resource rotation fan-out (close §M11.5 end-to-end)

> Branch `feat/engine-resource-rotation-wiring` off merged `main` (`7da0174c`, ADR-0067 landed).
> Authority: **ADR-0067 §Deferred** (binding — read first), ADR-0030 (engine owns rotation
> orchestration), ADR-0051 (`LeaseEvent` on `EventBus<LeaseEvent>`), ADR-0028 §4 (eventbus≠audit).
> Closes GitHub issues **#679 / #680 / #681** and de-latents §M11.5.

## Goal

The engine-side `ResourceFanoutIndex` is implemented + unit-tested but **no production path
invokes it** (every caller is `#[cfg(test)]`). Wire it into the real rotation/lease-revoke
paths and fix the 3 correctness P1s that become live the moment it is wired. ADR-0067
sequences them together: **#679 must land before/with the wiring**.

## Non-goals (explicitly deferred, do not pull in)

- Plugin-driven registrar auto-population (cross-crate; separate trigger).
- `RotationOutcome` → eventbus emission for dashboards (ADR-0028 §4; separate trigger — but
  the aggregate must still be *returned* and logged here).
- Production `ResourceRepo` impl + its cross-tenant isolation test gate (#685/#686 area).
- ADR-0067 §Deferred R-006/R-041/R-042/R-050/R-052.

## Units (sequenced — U1 before U4 per ADR-0067; U2/U3 land with U4)

### U1 — #679: per-resource drain + post-taint re-check (PREREQUISITE)
`crates/resource/src/manager/` — `revoke_resolved` currently taints the row then awaits the
**manager-wide** `drain_tracker` (the `graceful_shutdown` primitive); an unrelated busy
resource delays the revoke and the post-taint re-check is not per-resource.
- Add a **per-`ManagedResource` in-flight counter**; `revoke_resolved` drains only the
  revoked row's counter. Keep the manager-wide counter for `graceful_shutdown`.
- Add a `managed.is_tainted()` **re-check after `InFlightCounter::new`** in every
  `run_*_acquire` pipeline (mirror the `shutting_down` Defense-B pattern) → `Error::revoked`.
- Tests: multi-threaded revoke-vs-acquire race; an unrelated resource holding a lease must
  NOT delay the target revoke's drain.

### U2 — #681: synchronous pre-await taint (cancellation-safe revoke port)
`crates/resource/src/manager/` + `crates/engine/.../resource_fanout.rs` — a dropped
`tokio::time::timeout` future must not skip taint. Split the port: a **synchronous
`taint(key,scope,slot_identity)`** that runs and returns before any `.await`, then an
awaited drain+hook step the fan-out wraps in the per-resource timeout. The fan-out calls
the sync taint first, then the bounded awaited phase.
- Test: a revoke whose awaited phase times out still left the row tainted (new acquires
  rejected).

### U3 — #680: create-vs-rotate ordering guard (resident lost-update)
`crates/resource/src/runtime/managed.rs` (Resident arm) + `slot.rs` — `dispatch_slot_hook`
maps `current()==None` to `Ok(())`, so a refresh racing the first acquire records success
on a runtime built from the OLD credential and the hook is never delivered.
- Add an epoch/generation on `SlotCell` (bump on `store`); a runtime records the epoch it
  was built at; on rotation, if the live runtime's epoch < slot epoch (or `None` race),
  reconcile (rebuild or re-deliver the hook) instead of silent `Ok`.
- Tests: create-during-rotation for Resident (and the revoke inverse) — assert the hook is
  delivered / runtime not left on the stale credential.

### U4 — the wiring (engine; lands with U1–U3)
`crates/engine/src/credential/` — engine holds `Arc<ResourceFanoutIndex>` + the
`Arc<nebula_resource::Manager>` it already owns:
- **`bind`** on credential→slot resolution at the engine register/create path (where a
  credential is resolved into a `#[credential]` slot); **`unbind`** on resource
  remove/shutdown.
- **`dispatch_refresh`** after `RefreshCoordinator::refresh_coalesced` succeeds and the new
  material is stored into the slot cell — call site in `credential/dispatchers.rs` /
  `refresh.rs` per ADR-0067 D1, with `per_resource_rotation_timeout`.
- **`dispatch_revoke`** on `LeaseEvent::LeaseRevoked` (subscribe `EventBus<LeaseEvent>`;
  `lease/scheduler.rs:551` is the emit site) and on the ADR-0030 scheduler revoke.
- `RotationOutcome` aggregated + logged (typed, credential-data-free span/log); eventbus
  emission stays deferred (Non-goal) but the outcome must not be silently dropped.
- No `nebula-resource → nebula-engine` edge; cross-crate signal via `nebula-eventbus`.

### U5 — end-to-end verification + honest close-out
- New integration tests: rotation event → fan-out → `Manager::refresh_slot_for` → hook;
  `LeaseEvent::LeaseRevoked` → `dispatch_revoke` → taint→drain→revoke hook; redaction gate
  end-to-end through the wired path.
- Flip the honest claims now true: ADR-0067 §Status → §M11.5/§M12.4 **closed end-to-end**;
  abuse-case row 2 → satisfied (per-resource drain landed); parent `MATURITY.md`
  nebula-resource Engine-integration → drop the "fan-out unwired" caveat (save-only,
  parent tree non-git).
- `task dev:check` / per-crate clippy+nextest green for touched crates (resource, engine).
- Close #679 / #680 / #681 with the landing commit refs.

## Discipline
No plan/task IDs in committed code/comments (cite ADRs). No `unwrap/expect/panic/todo` in
lib code; `// guard-justified:` above any `#[allow]`/`unreachable!`. Cross-crate via
eventbus. Subagent-driven: per unit implementer → review → fix loop; reviewers verify
code not reports; honest escalation over green-washing. U1 commits before U4 (ADR-0067
sequencing). Squash-merge to `main` via PR.
