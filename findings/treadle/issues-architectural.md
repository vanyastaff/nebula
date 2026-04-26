# Treadle — Issues & Architectural Pain Points

## GitHub Issues

The treadle repository has **0 open issues and 0 closed issues** (confirmed by `gh issue list --repo oxur/treadle --state all` returning `[]`). The project is 2.5 months old with 2 GitHub stars and no community issue activity. The "≥3 cited issues" quality gate does not apply because the project has fewer than 100 closed issues.

## Pain Points from Internal Documentation

The following gaps are documented in code comments and design documents:

### 1. Retry semantics are incomplete in v1
**Source:** `src/workflow.rs`, around line 466 (comment in `advance_internal`):
```rust
Ok(crate::StageOutcome::Retry) => {
    // For now, treat retry as needing review
    // Full retry logic will be in Milestone 4.4
}
```
And `handle_outcome` (around line 614):
```rust
crate::StageOutcome::Retry => {
    // For now, mark as paused for retry
    // Full retry logic in Milestone 4.4
    state.mark_paused();
    state.increment_retry();
    ...
}
```
`StageOutcome::Retry` was released in v1 but behaves identically to `NeedsReview` (marks the stage paused, emits `StageRetried` event). Users who return `Retry` expecting automatic re-execution will see the stage stuck in Paused state. This is a documented incomplete feature.

### 2. Fan-out subtasks execute sequentially, not in parallel
**Source:** `src/workflow.rs` `execute_fanout` method (around line 692):
```rust
// Execute each subtask
for (idx, subtask) in subtasks.iter().enumerate() {
    ...
    let result = stage.execute(item, &mut ctx).await;
    ...
}
```
README says "Fan-Out with Per-Subtask Tracking" with implication of concurrent execution; implementation is a sequential for-loop. Not a documentation lie (README says "tracked independently", not "executed in parallel"), but developers coming from other engines that parallelize fan-out will be surprised.

### 3. No typed artefact passing between stages in v1
**Source:** v2 design document (`docs/design/02-under-review/0002-treadle-v2-design-document.md`), "What v1 Cannot Express" section:
> "Stage output capture. Stages return an Outcome enum, not the actual artefacts they produced. There's no built-in way to pass a stage's output to a quality evaluator or to downstream stages."

This is the primary v2 motivation. In v1, downstream stages cannot access upstream stage outputs without managing their own side-channel.

### 4. `advance_internal` recursion depth limit
**Source:** `src/workflow.rs:427`:
```rust
const MAX_DEPTH: usize = 100;
if depth > MAX_DEPTH {
    warn!("maximum recursion depth exceeded");
    return Err(TreadleError::StageExecution("maximum recursion depth exceeded".to_string()));
}
```
Pipelines with more than 100 stages that each complete in a single advance call would hit this limit. For typical usage this is fine, but it is a design smell — a proper task queue would not need a recursion guard.

### 5. No schema migration runner for SQLite
`src/state_store/sqlite.rs` initializes at schema version 1. The v2 design plans to add columns. Without a migration runner, upgrading an existing database will require manual schema changes or a database recreate.
