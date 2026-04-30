# Phase 7 — Deferred work (engine driver + ResourceAction dispatch)

> Status: documented at the Phase 7 commit boundary. The Phase 7
> deliverable lands the **storage + lifecycle primitives** for scoped
> resources (Tasks 7.1, 7.3 storage, 7.4, 7.5). The **engine-driver
> integration** that calls `ResourceAction::configure`/`cleanup` per
> branch on the frontier loop (Task 7.2 wiring) is deferred — see
> "Why deferred" below.

## What landed (Phase 7 — M6.2)

- **`DashScopedResourceMap`** — `dashmap`-backed per-branch storage in
  [`crates/engine/src/scoped_resources.rs`](../crates/engine/src/scoped_resources.rs).
  `register_branch(branch, parent)` / `push(branch, key, payload)` /
  `pop(branch) → Vec<PoppedEntry>` / `lookup_in_ancestors_from(start, key)`.
  Closest-ancestor walk with `MAX_ANCESTOR_DEPTH = 1024` cycle defense.
  No `Arc<Mutex<…>>` per `ARCHITECTURE.md` Anti-Patterns.
- **`BranchId`** — newtype wrapping `NodeKey`. Keeps the door open for
  per-execution / per-iteration namespacing without a rename.
- **Cleanup driver (Task 7.4)** — `run_cleanup{,_with_timeout}` wraps a
  resource-destroy future in `tokio::time::timeout`. Returns a typed
  `CleanupOutcome::{Completed,TimedOut,Failed}` so the engine attribution
  matches the observability triple (typed enum + tracing span + invariant
  check).
- **`ScopedResourceGuard`** — RAII wrapper. On `Drop` without `dismiss`,
  drives `pop` and routes the drained entries to a caller-provided panic
  sink. Cancel-safe by construction (the map state is corrected even on
  panic; the engine still has to drive async cleanup outside Drop).
- **`ExecutionEvent::ScopedResourceCleanupTimeout`** — new event variant
  in [`crates/engine/src/event.rs`](../crates/engine/src/event.rs).
  Carries `execution_id`, `branch_id`, `resource_key`, `budget`, `elapsed`.
  Engine emits this when a cleanup overruns the configured budget.
- **`DEFAULT_CLEANUP_TIMEOUT = 30s`** — public constant exported from
  the engine crate.
- **Phase 7 test suite** — 17 integration tests in
  [`crates/engine/tests/scoped_resources.rs`](../crates/engine/tests/scoped_resources.rs)
  covering Task 7.5's full matrix:
  3-hop nested shadowing, cancellation mid-branch, panic mid-branch,
  cleanup-uses-global, scope conflicts (closest-wins, sibling isolation),
  cleanup timeout typed event, inner-to-outer + LIFO destroy ordering,
  the three "Use cases" from `crates/resource/plans/10-scoped-resources.md`
  (temporary test database, per-tenant pool, ephemeral sandbox), concurrent
  sibling pushes, ancestor-depth bound, layered accessor wiring,
  per-execution credential scope namespacing.

## What is deferred (Task 7.2 wiring)

The frontier-loop integration that calls `ResourceAction::configure` on
node entry, `push`-es the resource, runs the downstream subtree, then
`pop`-s and drives `cleanup` on branch exit is **not** wired in Phase 7.
The Phase 6 wiring point in `engine.rs:2317` still threads
`LayeredResourceAccessor::global_only(global)` — the scoped layer is
empty in the production engine path.

The runtime explicitly **rejects** `ActionHandler::Resource` today:

```rust
// crates/engine/src/runtime/runtime.rs:442
ActionHandler::Resource(_) => {
    self.observe_rejected(dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE);
    return Err(RuntimeError::ResourceNotExecutable {
        key: action_key.to_owned(),
    });
},
```

Downstream consumers can stand up scoped resources programmatically
(constructing `DashScopedResourceMap`, calling
`register_branch`/`push`/`pop` directly, wiring the `LayeredResourceAccessor`
into an `ActionRuntimeContext`), but the engine's frontier loop does
not yet drive the lifecycle.

## Why deferred — architectural blocker

The plan's specification of "branch entry / branch exit" requires
defining what a **branch** is in the DAG. The frontier-driven engine
in `engine.rs` does not have a notion of explicit subgraphs:

1. There is no node kind discriminator. `NodeDefinition` carries an
   `ActionKey`; whether the resolved handler is a `Stateless` /
   `Stateful` / `Trigger` / `Resource` action is not known to the
   frontier loop until the runtime dispatches the node — and the
   frontier ready-queue advances per-node, not per-branch.
2. There is no parent/child branch relationship surface. The
   `DependencyGraph` exposes `incoming_connections` /
   `outgoing_connections`; mapping a `ResourceAction` node to "the
   subtree rooted at this node, minus its siblings" requires either
   (a) a reachability analysis at plan time (every node the
   ResourceAction dominates becomes a "child branch") or (b) explicit
   subgraph notation in `WorkflowDefinition` (the plan's "scope A /
   scope B" tree-style nesting from
   `crates/resource/plans/10-scoped-resources.md`).

Both options are larger architectural decisions than this phase budget
permits — they touch workflow validation, plan generation, the
frontier loop's bookkeeping, and the cancel propagation path. The
plan acknowledges this risk in §"Constraints" with the explicit
permission to ship what's done and document the rest here.

## What unblocks the deferred wiring

A short follow-up scope:

1. **Branch tree from dominator analysis.** At plan time, compute the
   immediate dominator for each `ResourceAction` node; every node in
   that node's dominator subtree is a child branch. Store the result
   on `ExecutionPlan` so the frontier loop reads it instead of
   recomputing. Sibling `ResourceAction`s naturally produce sibling
   subtrees; nested `ResourceAction`s produce nested subtrees.
2. **Frontier-loop hook**. In
   [`crates/engine/src/engine.rs`](../crates/engine/src/engine.rs)
   `spawn_node` (line ~2188): before dispatch, check if the node's
   handler is `ActionHandler::Resource`. If so, build a `BranchId` from
   the node's `NodeKey` (with execution-id namespacing), call
   `scoped.register_branch(branch, parent_branch_from_plan)`, run the
   ResourceAction's `configure(input, ctx)`, `push` the resulting
   `Resource` into the scoped map, and stamp `current_branch` on the
   layered accessor for the dispatched task.
3. **Branch-exit hook.** When the dominator subtree's last node
   completes (or the cancel token fires), `pop` the branch, run
   `Resource::destroy` for each popped entry under the timeout driver,
   then call the original `ResourceAction::cleanup(input, ctx)` (audit
   hook).
4. **Per-execution credential scope.** With execution-id-namespaced
   `BranchId`, scoped credential resolution at
   `ScopeLevel::Execution(execution_id)` falls out for free — the
   credential resolver picks up the execution id from the context's
   `Scope` field.
5. **Eventbus emission.** When the cleanup driver returns
   `CleanupOutcome::TimedOut`, emit
   `ExecutionEvent::ScopedResourceCleanupTimeout` via the engine's
   existing event sender.

## Verification at the Phase 7 boundary

- 24/24 unit tests pass (`cargo test -p nebula-engine --lib scoped_resources`).
- 17/17 integration tests pass (`cargo test -p nebula-engine --test scoped_resources`).
- Workspace `cargo check --all-targets` green.
- `cargo clippy --workspace --all-targets -- -D warnings` green.
- `cargo doc --no-deps --workspace -- -D rustdoc::broken_intra_doc_links` green.
