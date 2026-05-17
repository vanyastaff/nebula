# nebula-action v4 — pain-point write-up (input for ADR-0052)

> Captured 2026-05-13 during `/aif-improve` research pass. Honest scope:
> the working tree on `main` started rolling back the v4 surface without
> any documented rationale. This write-up enumerates what we *can* observe
> as evidence of strain plus the explicit gap (no user-stated cause).

## Origin

- M6/§M11 v4 surface shipped 2026-04-29 (per ROADMAP `Status Snapshot`).
- ADRs landing the v4 shape: 0042 (node binding), 0043 (dependency DX),
  0044 (resource/credential singular), 0045 (EventTrigger deferral).
- Working tree on `main` (uncommitted, observed 2026-05-13) drops
  `pub mod erased`, `pub mod factory`, `pub mod from_workflow_node`,
  `pub mod resource_produces`, `pub mod idempotency` from
  `crates/action/src/lib.rs` and reduces `Action` to an instance
  `metadata(&self)` form.
- No commit message, no ADR, no plan, no comment in source explains
  *why*. The closest pre-existing record is `b728e7aa revert: undo
  resource integration stack + context updates` whose body says only
  "Reverting commits made from another machine."

## Evidence of strain (observed, not user-confirmed)

### 1. Surface re-export bloat

`crates/action/src/lib.rs` (HEAD) `pub use` block exported ~50 symbols
across `Action`, sub-traits, handler trait family, factory generics,
erased trait family, output types, port types, webhook stack, validation
errors, test harnesses, plus a `prelude` module. Working tree trims this
to ~30 by removing the factory + erased re-exports. The trim is real,
but no user has said the re-export count was the bottleneck.

### 2. Two-struct authoring per action

ADR-0043 §Negative explicitly notes "Two structs per action (Self +
Input) — verbose by single-line metric". Working tree introduces
`FnStatelessAction` / `FnStatelessCtxAction` adapters that collapse the
trivial case to a single function — strong indirect evidence that
single-struct DX was wanted for slot-less actions. Hybrid (ADR-0052)
keeps both paths.

### 3. Macro surface growth

ADR-0043 §Negative quotes "5 macros + 1 attribute macro to ship +
maintain". The working tree does not consolidate these; macro burden is
unaddressed by the rollback. If macro complexity was the driver, the
rollback chose the wrong tool.

### 4. `async_trait` lingering in `trigger/mod.rs`

`crates/action/src/trigger/mod.rs:359, 485` still use `#[async_trait]`,
inconsistent with the rest of the crate (and inconsistent with the
working-tree intent — the lib.rs trim assumes it's gone). This is
strong evidence that one driver was modernization to native AFIT —
which hybrid preserves while keeping the dispatch indirection.

### 5. Latent FnCtx bug

`crates/action/src/stateless.rs:247-254` — `FnStatelessCtxAction::execute`
ignores `_ctx`. The struct name and docs promise context propagation; the
implementation does not. Discovered during research; bug is in the
working tree, not in v4. Suggests the rewrite was in-progress and not
audited.

### 6. Static `Action::metadata` was awkward

The v4 `Action: Sized` shape required a static `metadata()` method,
which forced engine call sites to either monomorphize per type or hold
the type via the factory. Working tree's instance `metadata(&self)`
makes `Arc<dyn Action>` a viable registry storage shape directly. This
is a clean improvement — hybrid keeps it.

## What is NOT evidence of strain

- "Factory layer became a YAGNI middleman" (the original /aif-plan v1
  task description) is not substantiated. The factory is the documented
  per-execution boundary for slot binding resolution, JSON marshaling,
  and per-node instantiation; engine deep dive confirms it is actively
  used at every dispatch (`runtime/runtime.rs:517-525`). Removing it
  would require relocating those responsibilities, not eliminating them.
- "Dispatch complexity" — Rust 2026 industry consensus
  (DataFusion, Temporal, axum, erased-serde, Bevy) converges on the
  same shape: typed-author trait + erased adapter + per-variant registry.
  Hybrid follows the consensus; full rollback would diverge.

## Explicit gap

No document in the repository says:

- which v4 capability turned out to be unused;
- which slot-binding pattern caused the most authoring friction;
- which downstream caller couldn't tolerate `<A as Action>::metadata()`
  static dispatch;
- whether the macro count (5+1) caused build-time pain;
- whether the per-execution `Box::new(ErasedXxxImpl::new(action))`
  allocation appeared in any profile.

If any of these are the real driver, capture them in this file and
amend ADR-0052 accordingly. Until then, the hybrid in ADR-0052 is the
minimum-risk reconciliation: keeps every documented v4 capability,
adopts every working-tree improvement that has architectural merit,
fixes the bug introduced during the rewrite.

## Recommended next steps for evidence collection

- If macro complexity is the real driver → open a separate proc-macro
  consolidation initiative (out of scope for ADR-0052).
- If the per-execution alloc cost is the real driver → benchmark
  `cargo bench -p nebula-action -- factory_dispatch` (does not exist
  yet; would need to land in `crates/action/benches/`); measure
  baseline; decide whether to optimize via factory caching or by
  changing `instantiate` to return a borrow.
- If authoring DX is the driver → the function adapters
  (`stateless_fn`, `stateless_ctx_fn`) plus `FromWorkflowNode` default
  no-op already cover the common slot-less case.
