---
date: 2026-05-18
topic: nebula-resource-topology-collapse
---

<!-- // budget-justified: ce-brainstorm requirements artifact — one coherent PRD-shaped document by design, not decomposable. The ADR-0083 blob cap is a Rust per-function complexity proxy (clippy.toml too-many-lines, itself set to allow workspace-wide) and carries no meaning for a markdown requirements doc. -->

# nebula-resource: Topology Collapse + Manager De-spaghettification

## Summary

Collapse the 5-topology taxonomy in `nebula-resource` to its two genuinely
distinct shapes plus one parameterized shape, behind a single object-safe
acquire seam. This removes the root cause of a 3118-line God-object `Manager`
(~35 hand-copied methods), one canonical revoke-invariant doc replaces nine
copies, and the registry lookup type makes one impossible-state branch
unrepresentable. Behavior is preserved; the API break is absorbed in-tree via
expand-contract with no shim and no ADR.

---

## Problem Frame

A code review of `nebula-resource` (the engine-owned resource lifecycle crate,
Business tier) found six structural defects whose common root is an
over-generalized topology model.

`crates/resource/src/topology/` defines five topology traits — `Pooled`,
`Resident`, `Service`, `Transport`, `Exclusive` — each with a parallel runtime
in `crates/resource/src/runtime/` and a config module. Reading all five
runtimes shows only **two** real axes of variation: a concurrency cap
(`Exclusive` = semaphore(1), `Transport` = semaphore(N), `Service` =
unbounded) and a per-acquire hook pair. `ExclusiveRuntime` is byte-equivalent
to `TransportRuntime` with the permit count fixed at 1; `ServiceRuntime` is the
same skeleton without the semaphore. A 2-axis decision is encoded as five
hardcoded points.

That taxonomy leaks into `crates/resource/src/manager/mod.rs`: five
`register_*` + five `register_*_with` shorthands, five `acquire_*` + five
`acquire_*_for` + five `acquire_*_at_scope` + five `run_*_acquire` pipelines.
The five `run_*_acquire` bodies are identical except one `match` arm and one
argument list, each carrying a dead `_ => Err("expected X topology")` arm that
is unreachable by construction. Adding one step to the acquire pipeline means
editing six near-identical copies — shotgun surgery, and the copies will drift.

Around it: a three-name passthrough chain (`Manager::erased_acquire_pooled` →
`erased_acquire_pooled` → `arc_acquire_pooled`) with identical bodies ×5; four
`#[allow(clippy::too_many_arguments)]` with prose essays arguing the linter
instead of introducing a params struct (`register_from_value` = 9 args); the
sync-taint/lazy-future-timeout invariant rationale copy-pasted verbatim across
nine sites; and a registry lookup branch that handles an `Ambiguous` case its
own comment calls "Unreachable", fabricating an error string for a state the
caller cannot produce.

The crate's domain logic (durable lifecycle, taint-before-drain, RAII leases,
cancel-safety, the credential-epoch reconcile) is mature and correct. The cost
is carried entirely by the form: every acquire-pipeline change is six edits,
every reader re-reads the same invariant nine times, and the Manager keeps
growing toward a sixth topology that never arrived.

---

## Actors

- A1. Resource author: implements a topology trait for a concrete resource
  (`impl Pooled/Resident/Service/Transport/Exclusive for X`). 8 impls of the
  three folded traits exist workspace-wide (verified by grep); ~all are test
  fixtures. 39 `Pooled`/`Resident` impls are untouched by the collapse.
- A2. Engine resource path: acquires resources at node dispatch. Production
  acquire goes through `Manager::acquire_erased` (verified:
  `crates/engine/src/resource_accessor.rs`) — topology-agnostic, so the
  collapse does not rewrite engine acquire call sites.
- A3. Manager API consumers (engine registrar, `nebula-api`, tests, examples):
  call the `register_*` family directly — this is the surface that breaks.
- A4. Maintainer of `nebula-resource`: the party currently paying the
  six-edits-per-change and nine-reads-per-invariant tax.

---

## Requirements

**Topology model collapse (root — review sin 1)**

- R1. `Pooled` and `Resident` remain distinct topologies with their own
  runtime modules — they are genuinely different (N-instance
  checkout/recycle/warmup/fingerprint-evict vs lazy single-create + clone +
  credential-epoch reconcile) and are not duplicative.
- R2. `Service`, `Transport`, `Exclusive` fold into one parameterized runtime
  governed by a concurrency cap (1 / N / unbounded) and a per-acquire
  hook pair. The fold MUST preserve every existing capability: Service
  `Cloned` vs `Tracked` release mode, Transport `open_session`/
  `close_session`/`keepalive`, Exclusive `reset`-on-release with the
  permit-held-until-reset ordering.
- R3. A single object-safe topology seam (one `acquire`-shaped dispatch point)
  replaces the five-arm `TopologyRuntime` match scattered across the Manager.
- R4. One unified author-facing trait replaces the three folded traits
  (`Transport`/`Service`/`Exclusive`). `Pooled`/`Resident` author traits are
  unchanged.

**Manager surface reduction (review sins 2, 3, 4)**

- R5. One generic `run_acquire` pipeline replaces the five `run_*_acquire`
  bodies. The dead `_ => Err("expected X topology")` arms disappear with the
  per-topology match.
- R6. The three-name passthrough indirection chain is reduced to a single
  layer (the redundant identical-body wrappers are deleted).
- R7. A `RegisterRequest`-style parameter aggregate replaces the multi-arg
  `register`/`register_with_identity`/`register_row_with_acquire`/
  `register_from_value` signatures. No `#[allow(clippy::too_many_arguments)]`
  remains in the Manager.
- R8. All acquire routes through the existing erased seam
  (`acquire_erased`). The typed per-topology `acquire_*` / `acquire_*_for` /
  `acquire_*_at_scope` family collapses to the minimum the typed callers
  actually need; per-topology entry points that exist only to fan out are
  removed.

**Invariant + registry hygiene (review sins 5, 6)**

- R9. The sync-taint / lazy-future-timeout two-phase-revoke rationale lives in
  exactly one canonical location (a crate module doc). The other eight sites
  carry a one-line reference, not a copy.
- R10. The registry single-pinned lookup (`get_for`) returns a type with no
  `Ambiguous` variant, so the impossible-state branch is unrepresentable
  rather than handled with a fabricated error. The identity-agnostic public
  paths keep their fail-closed `Ambiguous` deny (that case is real there).

**Behavior preservation + migration discipline**

- R11. Externally observable semantics are unchanged: acquire/release/taint/
  drain/revoke/rotation outcomes, `Cloned` vs `Tracked` handling, keepalive,
  reset-on-release ordering, credential-epoch reconcile, and the
  revoke-vs-acquire TOCTOU close all behave exactly as before.
- R12. Expand-contract migration: the whole workspace builds and its tests
  pass at every commit. Old surface is deleted last, after all callers move.
  No shim, adapter, or bridge at any point (replace the wrong thing directly).
- R13. All workspace impls of the folded traits and all affected tests/
  examples are migrated in-tree to the unified trait in the same body of work
  — not left compiling against a compatibility layer.
- R14. The topology-model rationale (why 5 → 3, the two real axes, why no ADR)
  is recorded in this requirements doc and in the canonical crate module doc
  from R9. No new ADR is created; no existing ADR is superseded (verified: no
  ADR binds the 5-topology model).

---

## Acceptance Examples

- AE1. **Covers R2, R11.** Given a resource that was `Service` + `Cloned`,
  when acquired, it returns an owned handle with no release callback —
  identical to today.
- AE2. **Covers R2, R11.** Given a resource that was `Service` + `Tracked`,
  when its handle drops, the release hook runs via the release queue —
  identical to today.
- AE3. **Covers R2, R11.** Given a resource that was `Exclusive`, when one
  caller holds the lease and a second acquires, the second unblocks only
  after the first's reset completes and the permit is returned.
- AE4. **Covers R8, R11.** Given an engine node acquiring a resource that was
  `Transport` via `acquire_erased`, the session open/close + permit semantics
  match the pre-collapse behavior.
- AE5. **Covers R12.** Given any single commit in the migration series, when
  the workspace is built and tested, it is green (per-crate clippy + nextest)
  — there is no red intermediate commit.
- AE6. **Covers R10, R11.** Given a multi-tenant `(key, scope)` with no
  resolved slot identity, the identity-agnostic acquire still fails closed
  with the `Ambiguous` deny; the identity-pinned lookup cannot express
  `Ambiguous` at the type level.

---

## Success Criteria

- `Manager` drops from ~3118 lines toward ~800; the count of public
  `register_*`/`acquire_*` methods drops from ~35 to a small constant; zero
  `run_*_acquire` clones remain.
- A maintainer adds a step to the acquire pipeline by editing one function,
  not six; reads the revoke invariant once, not nine times.
- All six review findings are closed and cannot regress structurally (the
  duplication is gone, not merely de-duplicated by discipline).
- ce-plan can sequence the migration without inventing product behavior:
  the seam, the unified trait's capability set, and the
  preserve-behavior contract are all specified here.
- Whole-workspace green at every commit; no shim ever introduced.

---

## Scope Boundaries

- bind-population / graduating `nebula-resource` out of `frontier`
  (ADR-0067 §M12.4 — no production credential→slot resolver) is **out**. The
  crate stays `frontier`. This was the considered higher-upside challenger and
  was explicitly rejected for this body of work.
- No behavior redesign: taint/drain/TOCTOU/epoch-reconcile semantics are
  preserved, not improved or re-specified.
- `Pooled`/`Resident` runtime internals are not refactored — they are not
  duplicative.
- Issue #589 (release_queue_handle → ArcSwapOption) is unrelated consistency
  work and is not included.
- No performance work; the collapse is shape-only.

---

## Key Decisions

- Full collapse over targeted fixes (review options A/B): A (point fixes) and
  B (de-dup without renaming) both leave the God-object + 5× duplication root
  intact. The user chose the deepest cut that removes the root.
- Keep Pool + Resident, fold only the three thin ones: the duplication is in
  Service/Transport/Exclusive; Pool/Resident earn their separate modules.
- No ADR: the 5-topology model is bound by no ADR (verified by grep over
  `docs/adr/`), and layer boundaries are enforced by `deny.toml`, not ADRs —
  so no ADR is mechanically required and none is superseded. The architectural
  rationale lives in the canonical crate module doc (which R9 creates anyway)
  plus this doc.
- Expand-contract, no shim: mandated by the repo's whole-workspace-green-
  per-commit discipline; expand-contract (add-new / migrate / delete-old-last)
  is the sanctioned path and is not a shim.

---

## Dependencies / Assumptions

- Verified: production acquire flows through `Manager::acquire_erased`
  (`crates/engine/src/resource_accessor.rs`), so the breaking surface is the
  `register_*` API + the folded author trait, not engine acquire call sites.
- Verified: 8 `impl Service/Transport/Exclusive` exist workspace-wide (mostly
  test fixtures); 39 `Pooled`/`Resident` impls are untouched.
- Verified: no ADR binds the 5-topology taxonomy; review sins are not tracked
  by any open issue (#682–#687 closed; only minor #589 open).
- Assumes the three folded capability sets (Cloned/Tracked, keepalive,
  reset-on-release, concurrency cap) can be expressed in one trait without
  losing any current behavior — the unified trait's exact shape is a planning
  decision (see Outstanding).

---

## Outstanding Questions

### Deferred to Planning

- [Affects R2, R4][Technical] Exact shape of the unified author trait — how
  Cloned/Tracked mode, optional keepalive, and reset-on-release compose
  (associated const + default methods vs other factorings).
- [Affects R3][Technical] Seam mechanism: dispatch enum vs `dyn` trait object
  for the object-safe topology seam.
- [Affects R12, R13][Technical] Migration commit ordering under expand-contract
  (which callers move first; where the old surface deletion lands).
