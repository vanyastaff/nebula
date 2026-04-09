# Phase 6: DX Types — StatefulAction Family

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move `StatefulAction` from `execution.rs` into `stateful.rs`, add DX convenience traits (`PaginatedAction`, `BatchAction`, `TransactionalAction`) with `macro_rules!` macros that generate `impl StatefulAction` for concrete types (no blanket impls — Rust coherence forbids them), plus `migrate_state()` for state schema evolution.

**Architecture:** New `crates/action/src/stateful.rs` — core `StatefulAction` trait (moved from `execution.rs`) + 3 DX traits + their state types. Three `#[macro_export]` macros (`impl_paginated_action!`, `impl_batch_action!`, `impl_transactional_action!`) generate `impl StatefulAction for $ty` by delegating to DX trait methods. Engine only sees `StatefulAction` — DX types are invisible. `execution.rs` re-exports `StatefulAction` for backward compat.

**Tech Stack:** Rust 1.94, `serde`/`serde_json`, `macro_rules!`, existing `ActionResult`/`ActionOutput`/`ActionError`

**Prerequisites:** Phase 3 done (StatefulHandler, StatefulActionAdapter). Phase 5 done (StatefulTestHarness).

**Spec reference:** `docs/plans/2026-04-08-action-v2-spec.md` sections 3.2 (DX layer), C3 (migrate_state)

**Why macro_rules!, not blanket impls:** Rust's coherence checker rejects multiple blanket impls of the same trait (`impl<A: PaginatedAction> StatefulAction for A` + `impl<A: BatchAction> StatefulAction for A` = compile error). `macro_rules!` generates a concrete impl per type — no coherence issues, `cargo expand` for debugging, scales to Phase 7 trigger DX types.

---

### Task 1: Move `StatefulAction` to `stateful.rs` and re-export

**Files:**
- Create: `crates/action/src/stateful.rs`
- Modify: `crates/action/src/execution.rs`
- Modify: `crates/action/src/lib.rs`

**Step 1: Create `stateful.rs` with the core trait moved from `execution.rs`**

Cut the `StatefulAction` trait (lines 71–109 of `execution.rs`, including doc comment) and place it in `stateful.rs`:

```rust
//! Stateful action trait and DX convenience patterns.
//!
//! Core:
//! - [`StatefulAction`] — iterative execution with persistent state (`Continue`/`Break`).
//!
//! DX convenience traits (use corresponding `impl_*_action!` macro to generate `StatefulAction`):
//! - [`PaginatedAction`] + [`impl_paginated_action!`]
//! - [`BatchAction`] + [`impl_batch_action!`]
//! - [`TransactionalAction`] + [`impl_transactional_action!`]

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::action::Action;
use crate::context::Context;
use crate::error::ActionError;
use crate::result::ActionResult;

/// Stateful action: iterative execution with persistent state.
///
/// The engine calls `execute` repeatedly. Return [`ActionResult::Continue`] to
/// request another iteration (state is saved); return [`ActionResult::Break`]
/// when done. Use for pagination, long-running loops, or multi-step processing.
///
/// State must be serializable (`Serialize + DeserializeOwned`) so the engine can
/// checkpoint it between iterations, and `Clone` so it can snapshot before
/// executing (rollback on failure).
///
/// Cancellation is enforced by the runtime (same as [`StatelessAction`](crate::execution::StatelessAction)).
pub trait StatefulAction: Action {
    /// Input type for each iteration.
    type Input: Send + Sync;
    /// Output type (wrapped in [`ActionResult`]); `Continue` and `Break` carry output.
    type Output: Send + Sync;
    /// Persistent state type (saved between iterations by the engine).
    ///
    /// Must be serializable for engine checkpointing and cloneable for
    /// pre-execution snapshots.
    type State: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Create initial state for the first iteration.
    ///
    /// Called once when the engine starts executing this action. Subsequent
    /// iterations receive the state mutated by the previous `execute` call.
    fn init_state(&self) -> Self::State;

    /// Migrate state from a previous action version.
    ///
    /// Called by the engine when direct deserialization of persisted state into
    /// `Self::State` fails (e.g., after an action version upgrade changed the
    /// state schema). Return `Some(migrated)` if migration succeeds, `None`
    /// to propagate the original deserialization error.
    ///
    /// Default: no migration (returns `None`).
    fn migrate_state(&self, _old: serde_json::Value) -> Option<Self::State> {
        None
    }

    /// Execute one iteration with the given input, mutable state, and context.
    ///
    /// Return `Continue { output, progress, delay }` for another iteration,
    /// or `Break { output, reason }` when finished.
    fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
```

**Step 2: In `execution.rs`, remove `StatefulAction` trait and re-export**

Remove the `StatefulAction` trait definition (lines 71–109). Add at the top, after the existing imports:

```rust
// Re-export StatefulAction from its dedicated module.
pub use crate::stateful::StatefulAction;
```

**Step 3: Add `stateful` module to `lib.rs`**

Add after `pub mod scoped;`:

```rust
/// Stateful action trait and DX convenience patterns (paginated, batch, transactional).
pub mod stateful;
```

**Step 4: Run check + tests**

Run: `cargo check -p nebula-action && cargo nextest run -p nebula-action`
Expected: compiles, all tests pass (re-export preserves all `use crate::execution::StatefulAction` paths)

**Step 5: Commit**

```
refactor(action): move StatefulAction to stateful.rs
```

---

### Task 2: Add `migrate_state` to `StatefulHandler` + adapter fallback

**Files:**
- Modify: `crates/action/src/handler.rs`

**Step 1: Add `migrate_state` to `StatefulHandler` trait**

In the `StatefulHandler` trait (around line 425), add after `init_state`:

```rust
    /// Attempt to migrate state from a previous version.
    ///
    /// Called when state deserialization fails during `execute`. Returns
    /// migrated state as JSON, or `None` to propagate the error.
    fn migrate_state(&self, old: Value) -> Option<Value> {
        let _ = old;
        None
    }
```

**Step 2: Update `StatefulActionAdapter` to delegate `migrate_state`**

In `impl StatefulHandler for StatefulActionAdapter<A>` (around line 189), add:

```rust
    fn migrate_state(&self, old: Value) -> Option<Value> {
        self.action
            .migrate_state(old)
            .and_then(|state| serde_json::to_value(state).ok())
    }
```

**Step 3: Update adapter `execute` — try migration on state deser failure**

Replace the state deserialization line (around line 220):

Old:
```rust
        let mut typed_state: A::State = serde_json::from_value(state.clone())
            .map_err(|e| ActionError::validation(format!("state deserialization failed: {e}")))?;
```

New:
```rust
        let mut typed_state: A::State = serde_json::from_value(state.clone()).or_else(|e| {
            self.action
                .migrate_state(state.clone())
                .ok_or_else(|| ActionError::validation(format!("state deserialization failed: {e}")))
        })?;
```

**Step 4: Run check + tests**

Run: `cargo check -p nebula-action && cargo nextest run -p nebula-action`
Expected: compiles, all pass

**Step 5: Commit**

```
feat(action): migrate_state fallback in StatefulHandler and adapter
```

---

### Task 3: Tests for `migrate_state`

**Files:**
- Modify: `crates/action/tests/execution_integration.rs`

**Step 1: Add test types and tests**

Append at the end of the file:

```rust
// ── migrate_state tests ─────────────────────────────────────────────────

/// State v2 — has a new `label` field not in v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MigratableState {
    count: u32,
    label: String,
}

struct MigratableAction {
    meta: ActionMetadata,
}

impl ActionDependencies for MigratableAction {}

impl Action for MigratableAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatefulAction for MigratableAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    type State = MigratableState;

    fn init_state(&self) -> MigratableState {
        MigratableState { count: 0, label: "new".into() }
    }

    fn migrate_state(&self, old: serde_json::Value) -> Option<MigratableState> {
        let count = old.get("count")?.as_u64()? as u32;
        Some(MigratableState { count, label: "migrated".into() })
    }

    async fn execute(
        &self,
        _input: Self::Input,
        state: &mut Self::State,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        state.count += 1;
        Ok(ActionResult::Break {
            output: ActionOutput::Value(serde_json::json!({
                "count": state.count, "label": &state.label,
            })),
            reason: BreakReason::Completed,
        })
    }
}

#[tokio::test]
async fn migrate_state_succeeds_from_v1() {
    let action = MigratableAction {
        meta: ActionMetadata::builder("migrate_test", "Migrate Test").build(),
    };
    let adapter = StatefulActionAdapter::new(action);
    let ctx = TestContextBuilder::minimal().build();
    let mut old_state = serde_json::json!({ "count": 5 });

    let result = adapter.execute(serde_json::json!({}), &mut old_state, &ctx).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_break!(result);
}

#[tokio::test]
async fn migrate_state_propagates_error_when_none() {
    let action = CounterAction {
        meta: ActionMetadata::builder("counter", "Counter").build(),
    };
    let adapter = StatefulActionAdapter::new(action);
    let ctx = TestContextBuilder::minimal().build();
    let mut bad_state = serde_json::json!("not_an_object");

    let result = adapter.execute(serde_json::json!({}), &mut bad_state, &ctx).await;
    assert_validation_error!(result);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action migrate_state`
Expected: both PASS

**Step 3: Commit**

```
test(action): migrate_state fallback and error propagation tests
```

---

### Task 4: `ActionResult` convenience constructors for Continue/Break

**Files:**
- Modify: `crates/action/src/result.rs`

**Step 1: Add tests in `#[cfg(test)] mod tests`**

```rust
    #[test]
    fn continue_with_constructor() {
        let r: ActionResult<i32> = ActionResult::continue_with(42, Some(0.5));
        assert!(r.is_continue());
        match r {
            ActionResult::Continue { output, progress, delay } => {
                assert_eq!(output.as_value(), Some(&42));
                assert_eq!(progress, Some(0.5));
                assert!(delay.is_none());
            }
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn continue_with_delay_constructor() {
        let r: ActionResult<i32> =
            ActionResult::continue_with_delay(7, Some(0.8), Duration::from_secs(5));
        match r {
            ActionResult::Continue { output, progress, delay } => {
                assert_eq!(output.as_value(), Some(&7));
                assert_eq!(progress, Some(0.8));
                assert_eq!(delay, Some(Duration::from_secs(5)));
            }
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn break_completed_constructor() {
        let r: ActionResult<String> = ActionResult::break_completed("done".into());
        match r {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value(), Some(&"done".to_string()));
                assert_eq!(reason, BreakReason::Completed);
            }
            _ => panic!("expected Break"),
        }
    }

    #[test]
    fn break_with_reason_constructor() {
        let r: ActionResult<i32> =
            ActionResult::break_with_reason(99, BreakReason::MaxIterations);
        match r {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value(), Some(&99));
                assert_eq!(reason, BreakReason::MaxIterations);
            }
            _ => panic!("expected Break"),
        }
    }
```

**Step 2: Run to verify they fail**

Run: `cargo nextest run -p nebula-action continue_with_constructor`
Expected: FAIL

**Step 3: Add constructors to `impl<T> ActionResult<T>`**

After `skip_with_output` method (around line 264):

```rust
    /// Create a `Continue` result for stateful action iteration.
    ///
    /// Wraps `output` in [`ActionOutput::Value`] with optional progress.
    /// No delay between iterations.
    #[must_use]
    pub fn continue_with(output: T, progress: Option<f64>) -> Self {
        Self::Continue {
            output: ActionOutput::Value(output),
            progress,
            delay: None,
        }
    }

    /// Create a `Continue` result with a delay before the next iteration.
    #[must_use]
    pub fn continue_with_delay(output: T, progress: Option<f64>, delay: Duration) -> Self {
        Self::Continue {
            output: ActionOutput::Value(output),
            progress,
            delay: Some(delay),
        }
    }

    /// Create a `Break` result indicating natural completion.
    #[must_use]
    pub fn break_completed(output: T) -> Self {
        Self::Break {
            output: ActionOutput::Value(output),
            reason: BreakReason::Completed,
        }
    }

    /// Create a `Break` result with a specific reason.
    #[must_use]
    pub fn break_with_reason(output: T, reason: BreakReason) -> Self {
        Self::Break {
            output: ActionOutput::Value(output),
            reason,
        }
    }
```

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-action continue_with`
Run: `cargo nextest run -p nebula-action break_completed`
Expected: all PASS

**Step 5: Commit**

```
feat(action): ActionResult convenience constructors for Continue/Break
```

---

### Task 5: PaginatedAction trait + `impl_paginated_action!` macro

**Files:**
- Modify: `crates/action/src/stateful.rs`

**Step 1: Append PaginatedAction types, trait, and macro**

Add after `StatefulAction` trait in `stateful.rs`:

```rust
// ── PaginatedAction ─────────────────────────────────────────────────────────

/// Result of fetching a single page.
#[derive(Debug, Clone)]
pub struct PageResult<T, C> {
    /// Data returned by this page.
    pub data: T,
    /// Cursor for the next page, or `None` if this was the last page.
    pub next_cursor: Option<C>,
}

/// Persistent state for pagination (managed by [`impl_paginated_action!`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationState<C> {
    /// Current cursor position (`None` = first page).
    pub cursor: Option<C>,
    /// Number of pages fetched so far.
    pub pages_fetched: u32,
}

/// Cursor-driven pagination action.
///
/// Implement [`fetch_page`](PaginatedAction::fetch_page) and invoke
/// [`impl_paginated_action!`] to generate the [`StatefulAction`] impl.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::stateful::{PaginatedAction, PageResult};
///
/// struct ListRepos;
/// impl PaginatedAction for ListRepos {
///     type Input = Value;
///     type Output = Vec<Repo>;
///     type Cursor = String;
///     fn max_pages(&self) -> u32 { 10 }
///     async fn fetch_page(&self, input: &Value, cursor: Option<&String>, ctx: &impl Context)
///         -> Result<PageResult<Vec<Repo>, String>, ActionError> { todo!() }
/// }
/// nebula_action::impl_paginated_action!(ListRepos);
/// ```
pub trait PaginatedAction: Action {
    /// Input type for the paginated request.
    type Input: Send + Sync;
    /// Output type produced per page.
    type Output: Send + Sync;
    /// Cursor type for tracking pagination position.
    type Cursor: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Maximum pages before forcing a break. Default: 100.
    fn max_pages(&self) -> u32 {
        100
    }

    /// Fetch a single page. `cursor` is `None` for the first page.
    ///
    /// # Errors
    ///
    /// Return [`ActionError::Retryable`] for transient failures,
    /// [`ActionError::Fatal`] for permanent failures.
    fn fetch_page(
        &self,
        input: &Self::Input,
        cursor: Option<&Self::Cursor>,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<PageResult<Self::Output, Self::Cursor>, ActionError>> + Send;
}

/// Generate `impl StatefulAction` for a type that implements [`PaginatedAction`].
///
/// The generated impl manages cursor tracking, progress reporting, and
/// Continue/Break decisions. The engine sees only [`StatefulAction`].
///
/// # Example
///
/// ```rust,ignore
/// impl PaginatedAction for MyAction { /* ... */ }
/// nebula_action::impl_paginated_action!(MyAction);
/// registry.register_stateful(MyAction);
/// ```
#[macro_export]
macro_rules! impl_paginated_action {
    ($ty:ty) => {
        impl $crate::stateful::StatefulAction for $ty {
            type Input = <$ty as $crate::stateful::PaginatedAction>::Input;
            type Output = <$ty as $crate::stateful::PaginatedAction>::Output;
            type State = $crate::stateful::PaginationState<
                <$ty as $crate::stateful::PaginatedAction>::Cursor,
            >;

            fn init_state(&self) -> Self::State {
                $crate::stateful::PaginationState {
                    cursor: None,
                    pages_fetched: 0,
                }
            }

            async fn execute(
                &self,
                input: Self::Input,
                state: &mut Self::State,
                ctx: &impl $crate::context::Context,
            ) -> ::core::result::Result<
                $crate::result::ActionResult<Self::Output>,
                $crate::error::ActionError,
            > {
                use $crate::stateful::PaginatedAction as _;
                let result = self.fetch_page(&input, state.cursor.as_ref(), ctx).await?;
                state.cursor.clone_from(&result.next_cursor);
                state.pages_fetched += 1;

                if result.next_cursor.is_some() && state.pages_fetched < self.max_pages() {
                    let progress = state.pages_fetched as f64 / self.max_pages() as f64;
                    Ok($crate::ActionResult::continue_with(result.data, Some(progress)))
                } else {
                    Ok($crate::ActionResult::break_completed(result.data))
                }
            }
        }
    };
}
```

**Step 2: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 3: Commit**

```
feat(action): PaginatedAction trait and impl_paginated_action! macro
```

---

### Task 6: Tests for PaginatedAction

**Files:**
- Create: `crates/action/tests/dx_paginated.rs`

**Step 1: Write the tests**

```rust
//! Integration tests for PaginatedAction DX trait.

use nebula_action::action::Action;
use nebula_action::context::Context;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::stateful::{PageResult, PaginatedAction};
use nebula_action::testing::{StatefulTestHarness, TestContextBuilder};

struct NumberPaginator {
    meta: ActionMetadata,
    total_pages: u32,
}

impl ActionDependencies for NumberPaginator {}
impl Action for NumberPaginator {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl PaginatedAction for NumberPaginator {
    type Input = serde_json::Value;
    type Output = Vec<i32>;
    type Cursor = u32;

    fn max_pages(&self) -> u32 { self.total_pages + 1 }

    async fn fetch_page(
        &self,
        _input: &serde_json::Value,
        cursor: Option<&u32>,
        _ctx: &impl Context,
    ) -> Result<PageResult<Vec<i32>, u32>, ActionError> {
        let page = cursor.copied().unwrap_or(0);
        let data: Vec<i32> = ((page * 10)..((page + 1) * 10)).map(|i| i as i32).collect();
        let next = if page + 1 < self.total_pages { Some(page + 1) } else { None };
        Ok(PageResult { data, next_cursor: next })
    }
}

nebula_action::impl_paginated_action!(NumberPaginator);

#[tokio::test]
async fn paginated_fetches_all_pages() {
    let action = NumberPaginator {
        meta: ActionMetadata::builder("paginator", "Paginator").build(),
        total_pages: 3,
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let input = serde_json::json!({});
    let r1 = harness.step(input.clone()).await.expect("step 1");
    assert!(r1.is_continue());
    let r2 = harness.step(input.clone()).await.expect("step 2");
    assert!(r2.is_continue());
    let r3 = harness.step(input).await.expect("step 3");
    assert!(matches!(r3, nebula_action::ActionResult::Break { .. }));
    assert_eq!(harness.iterations(), 3);
}

#[tokio::test]
async fn paginated_respects_max_pages() {
    struct LimitedPaginator(NumberPaginator);
    impl ActionDependencies for LimitedPaginator {}
    impl Action for LimitedPaginator {
        fn metadata(&self) -> &ActionMetadata { self.0.metadata() }
    }
    impl PaginatedAction for LimitedPaginator {
        type Input = serde_json::Value;
        type Output = Vec<i32>;
        type Cursor = u32;
        fn max_pages(&self) -> u32 { 2 }
        async fn fetch_page(
            &self, input: &serde_json::Value, cursor: Option<&u32>, ctx: &impl Context,
        ) -> Result<PageResult<Vec<i32>, u32>, ActionError> {
            self.0.fetch_page(input, cursor, ctx).await
        }
    }
    nebula_action::impl_paginated_action!(LimitedPaginator);

    let action = LimitedPaginator(NumberPaginator {
        meta: ActionMetadata::builder("limited", "Limited").build(),
        total_pages: 100,
    });
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let r1 = harness.step(serde_json::json!({})).await.expect("step 1");
    assert!(r1.is_continue());
    let r2 = harness.step(serde_json::json!({})).await.expect("step 2");
    assert!(matches!(r2, nebula_action::ActionResult::Break { .. }));
}

#[tokio::test]
async fn paginated_single_page() {
    let action = NumberPaginator {
        meta: ActionMetadata::builder("single", "Single").build(),
        total_pages: 1,
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let r = harness.step(serde_json::json!({})).await.expect("step 1");
    assert!(matches!(r, nebula_action::ActionResult::Break { .. }));
    assert_eq!(harness.iterations(), 1);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action dx_paginated`
Expected: all 3 PASS

**Step 3: Commit**

```
test(action): PaginatedAction integration tests
```

---

### Task 7: BatchAction trait + `impl_batch_action!` macro

**Files:**
- Modify: `crates/action/src/stateful.rs`

**Step 1: Append BatchAction types, trait, and macro**

```rust
// ── BatchAction ─────────────────────────────────────────────────────────────

/// Result of processing a single batch item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum BatchItemResult<T> {
    /// Item processed successfully.
    Ok {
        /// Output for this item.
        output: T,
    },
    /// Item processing failed (non-fatal — batch continues).
    Failed {
        /// Error message for this item.
        error: String,
    },
}

/// Persistent state for batch processing (managed by [`impl_batch_action!`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchState<I, T> {
    /// Remaining items to process.
    pub remaining: Vec<I>,
    /// Results accumulated so far.
    pub results: Vec<BatchItemResult<T>>,
    /// Number of chunks processed.
    pub chunks_processed: u32,
}

/// Batch processing action — fixed-size chunks with per-item error handling.
///
/// Implement [`extract_items`](BatchAction::extract_items),
/// [`process_item`](BatchAction::process_item), and
/// [`merge_results`](BatchAction::merge_results), then invoke
/// [`impl_batch_action!`] to generate the [`StatefulAction`] impl.
///
/// [`ActionError::Fatal`] from `process_item` aborts the entire batch.
/// Other errors are captured as [`BatchItemResult::Failed`].
pub trait BatchAction: Action {
    /// Input type containing the items to process.
    type Input: Send + Sync;
    /// Individual work item type.
    type Item: Serialize + DeserializeOwned + Clone + Send + Sync;
    /// Output type per item and as the final merged result.
    type Output: Serialize + DeserializeOwned + Send + Sync;

    /// Items per iteration. Default: 50.
    fn batch_size(&self) -> usize { 50 }

    /// Extract work items from input. Called once on the first iteration.
    fn extract_items(&self, input: &Self::Input) -> Vec<Self::Item>;

    /// Process a single item.
    ///
    /// # Errors
    ///
    /// [`ActionError::Fatal`] aborts the batch. Other errors are captured per-item.
    fn process_item(
        &self,
        item: Self::Item,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<Self::Output, ActionError>> + Send;

    /// Merge per-item results into the final output.
    fn merge_results(&self, results: Vec<BatchItemResult<Self::Output>>) -> Self::Output;
}

/// Generate `impl StatefulAction` for a type that implements [`BatchAction`].
#[macro_export]
macro_rules! impl_batch_action {
    ($ty:ty) => {
        impl $crate::stateful::StatefulAction for $ty {
            type Input = <$ty as $crate::stateful::BatchAction>::Input;
            type Output = <$ty as $crate::stateful::BatchAction>::Output;
            type State = $crate::stateful::BatchState<
                <$ty as $crate::stateful::BatchAction>::Item,
                <$ty as $crate::stateful::BatchAction>::Output,
            >;

            fn init_state(&self) -> Self::State {
                $crate::stateful::BatchState {
                    remaining: Vec::new(),
                    results: Vec::new(),
                    chunks_processed: 0,
                }
            }

            async fn execute(
                &self,
                input: Self::Input,
                state: &mut Self::State,
                ctx: &impl $crate::context::Context,
            ) -> ::core::result::Result<
                $crate::result::ActionResult<Self::Output>,
                $crate::error::ActionError,
            > {
                use $crate::stateful::BatchAction as _;

                if state.chunks_processed == 0 {
                    state.remaining = self.extract_items(&input);
                }

                let batch_size = self.batch_size().max(1);
                let chunk_end = batch_size.min(state.remaining.len());
                let chunk: Vec<_> = state.remaining.drain(..chunk_end).collect();

                for item in chunk {
                    match self.process_item(item, ctx).await {
                        Ok(output) => state.results.push(
                            $crate::stateful::BatchItemResult::Ok { output },
                        ),
                        Err(e) => {
                            if e.is_fatal() {
                                return Err(e);
                            }
                            state.results.push(
                                $crate::stateful::BatchItemResult::Failed {
                                    error: e.to_string(),
                                },
                            );
                        }
                    }
                }
                state.chunks_processed += 1;

                if state.remaining.is_empty() {
                    let results = ::std::mem::take(&mut state.results);
                    let merged = self.merge_results(results);
                    Ok($crate::ActionResult::break_completed(merged))
                } else {
                    let total = state.results.len() + state.remaining.len();
                    let done = state.results.len();
                    let progress = done as f64 / total as f64;
                    let partial = self.merge_results(state.results.clone());
                    Ok($crate::ActionResult::continue_with(partial, Some(progress)))
                }
            }
        }
    };
}
```

**Step 2: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 3: Commit**

```
feat(action): BatchAction trait and impl_batch_action! macro
```

---

### Task 8: Tests for BatchAction

**Files:**
- Create: `crates/action/tests/dx_batch.rs`

**Step 1: Write the tests**

```rust
//! Integration tests for BatchAction DX trait.

use nebula_action::action::Action;
use nebula_action::context::Context;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::stateful::{BatchAction, BatchItemResult};
use nebula_action::testing::{StatefulTestHarness, TestContextBuilder};
use serde::{Deserialize, Serialize};

struct DoublerBatch { meta: ActionMetadata }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NumberList { numbers: Vec<i32> }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BatchOutput { processed: Vec<i32>, errors: usize }

impl ActionDependencies for DoublerBatch {}
impl Action for DoublerBatch {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl BatchAction for DoublerBatch {
    type Input = NumberList;
    type Output = BatchOutput;
    type Item = i32;

    fn batch_size(&self) -> usize { 3 }

    fn extract_items(&self, input: &NumberList) -> Vec<i32> { input.numbers.clone() }

    async fn process_item(&self, item: i32, _ctx: &impl Context)
        -> Result<BatchOutput, ActionError>
    {
        if item < 0 {
            return Err(ActionError::retryable(format!("negative: {item}")));
        }
        Ok(BatchOutput { processed: vec![item * 2], errors: 0 })
    }

    fn merge_results(&self, results: Vec<BatchItemResult<BatchOutput>>) -> BatchOutput {
        let mut processed = Vec::new();
        let mut errors = 0;
        for r in results {
            match r {
                BatchItemResult::Ok { output } => processed.extend(output.processed),
                BatchItemResult::Failed { .. } => errors += 1,
            }
        }
        BatchOutput { processed, errors }
    }
}

nebula_action::impl_batch_action!(DoublerBatch);

#[tokio::test]
async fn batch_processes_in_chunks() {
    let action = DoublerBatch {
        meta: ActionMetadata::builder("doubler", "Doubler").build(),
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let input = serde_json::to_value(NumberList {
        numbers: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    }).unwrap();

    // batch_size=3, 10 items -> 4 chunks (3+3+3+1)
    let r1 = harness.step(input.clone()).await.expect("step 1");
    assert!(r1.is_continue());
    let r2 = harness.step(input.clone()).await.expect("step 2");
    assert!(r2.is_continue());
    let r3 = harness.step(input.clone()).await.expect("step 3");
    assert!(r3.is_continue());
    let r4 = harness.step(input).await.expect("step 4");
    assert!(matches!(r4, nebula_action::ActionResult::Break { .. }));
}

#[tokio::test]
async fn batch_handles_per_item_errors() {
    let action = DoublerBatch {
        meta: ActionMetadata::builder("doubler", "Doubler").build(),
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let input = serde_json::to_value(NumberList {
        numbers: vec![1, -1, 2],
    }).unwrap();

    let r = harness.step(input).await.expect("step 1");
    match r {
        nebula_action::ActionResult::Break { output, .. } => {
            let result: BatchOutput =
                serde_json::from_value(output.into_value().unwrap()).unwrap();
            assert_eq!(result.processed, vec![2, 4]);
            assert_eq!(result.errors, 1);
        }
        other => panic!("expected Break, got {:?}", other),
    }
}

#[tokio::test]
async fn batch_single_chunk() {
    let action = DoublerBatch {
        meta: ActionMetadata::builder("doubler", "Doubler").build(),
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let input = serde_json::to_value(NumberList { numbers: vec![5] }).unwrap();
    let r = harness.step(input).await.expect("step 1");
    assert!(matches!(r, nebula_action::ActionResult::Break { .. }));
    assert_eq!(harness.iterations(), 1);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action dx_batch`
Expected: all 3 PASS

**Step 3: Commit**

```
test(action): BatchAction integration tests
```

---

### Task 9: TransactionalAction trait + `impl_transactional_action!` macro

**Files:**
- Modify: `crates/action/src/stateful.rs`

**Step 1: Append TransactionalAction types, trait, and macro**

```rust
// ── TransactionalAction ─────────────────────────────────────────────────────

/// Phase of the transactional action state machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionPhase {
    /// Forward execution not yet started.
    Pending,
    /// Forward execution completed; compensation data stored.
    Executed,
    /// Compensation completed.
    Compensated,
}

/// Persistent state for transactional actions (managed by [`impl_transactional_action!`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionState<T, C> {
    /// Current phase.
    pub phase: TransactionPhase,
    /// Output from forward execution.
    pub output: Option<T>,
    /// Compensation data from forward execution.
    pub compensation_data: Option<C>,
}

/// Saga-pattern transactional action.
///
/// Implement [`execute_tx`](TransactionalAction::execute_tx) and
/// [`compensate`](TransactionalAction::compensate), then invoke
/// [`impl_transactional_action!`].
///
/// **Note:** Engine-level saga orchestration is post-v1. Compensation
/// must be triggered explicitly by the caller.
pub trait TransactionalAction: Action {
    /// Input for forward execution.
    type Input: Send + Sync;
    /// Output from forward execution.
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync;
    /// Data needed to compensate (undo) the forward execution.
    type CompensationData: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Forward operation. Returns output + compensation data.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if the forward operation fails.
    fn execute_tx(
        &self,
        input: Self::Input,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<(Self::Output, Self::CompensationData), ActionError>> + Send;

    /// Compensate (undo) the forward execution.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if compensation fails.
    fn compensate(
        &self,
        data: Self::CompensationData,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}

/// Generate `impl StatefulAction` for a type that implements [`TransactionalAction`].
#[macro_export]
macro_rules! impl_transactional_action {
    ($ty:ty) => {
        impl $crate::stateful::StatefulAction for $ty {
            type Input = <$ty as $crate::stateful::TransactionalAction>::Input;
            type Output = <$ty as $crate::stateful::TransactionalAction>::Output;
            type State = $crate::stateful::TransactionState<
                <$ty as $crate::stateful::TransactionalAction>::Output,
                <$ty as $crate::stateful::TransactionalAction>::CompensationData,
            >;

            fn init_state(&self) -> Self::State {
                $crate::stateful::TransactionState {
                    phase: $crate::stateful::TransactionPhase::Pending,
                    output: None,
                    compensation_data: None,
                }
            }

            async fn execute(
                &self,
                input: Self::Input,
                state: &mut Self::State,
                ctx: &impl $crate::context::Context,
            ) -> ::core::result::Result<
                $crate::result::ActionResult<Self::Output>,
                $crate::error::ActionError,
            > {
                use $crate::stateful::TransactionalAction as _;
                match state.phase {
                    $crate::stateful::TransactionPhase::Pending => {
                        let (output, comp) = self.execute_tx(input, ctx).await?;
                        state.phase = $crate::stateful::TransactionPhase::Executed;
                        state.output = Some(output.clone());
                        state.compensation_data = Some(comp);
                        Ok($crate::ActionResult::break_completed(output))
                    }
                    $crate::stateful::TransactionPhase::Executed => {
                        let data = state.compensation_data.take().ok_or_else(|| {
                            $crate::error::ActionError::fatal(
                                "compensation data missing in Executed phase",
                            )
                        })?;
                        self.compensate(data, ctx).await?;
                        state.phase = $crate::stateful::TransactionPhase::Compensated;
                        let output = state.output.clone().ok_or_else(|| {
                            $crate::error::ActionError::fatal(
                                "output missing in Executed phase",
                            )
                        })?;
                        Ok($crate::ActionResult::break_with_reason(
                            output,
                            $crate::result::BreakReason::Custom("compensated".into()),
                        ))
                    }
                    $crate::stateful::TransactionPhase::Compensated => {
                        Err($crate::error::ActionError::fatal(
                            "transactional action already compensated",
                        ))
                    }
                }
            }
        }
    };
}
```

**Step 2: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 3: Commit**

```
feat(action): TransactionalAction trait and impl_transactional_action! macro
```

---

### Task 10: Tests for TransactionalAction

**Files:**
- Create: `crates/action/tests/dx_transactional.rs`

**Step 1: Write the tests**

```rust
//! Integration tests for TransactionalAction DX trait.

use nebula_action::action::Action;
use nebula_action::context::Context;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::result::BreakReason;
use nebula_action::stateful::TransactionalAction;
use nebula_action::testing::{StatefulTestHarness, TestContextBuilder};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Confirmation { tx_id: String, amount: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefundInfo { tx_id: String }

struct PaymentAction {
    meta: ActionMetadata,
    compensated: Arc<AtomicBool>,
}

impl ActionDependencies for PaymentAction {}
impl Action for PaymentAction {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl TransactionalAction for PaymentAction {
    type Input = serde_json::Value;
    type Output = Confirmation;
    type CompensationData = RefundInfo;

    async fn execute_tx(&self, _input: serde_json::Value, _ctx: &impl Context)
        -> Result<(Confirmation, RefundInfo), ActionError>
    {
        Ok((
            Confirmation { tx_id: "tx_123".into(), amount: 1000 },
            RefundInfo { tx_id: "tx_123".into() },
        ))
    }

    async fn compensate(&self, _data: RefundInfo, _ctx: &impl Context)
        -> Result<(), ActionError>
    {
        self.compensated.store(true, Ordering::Relaxed);
        Ok(())
    }
}

nebula_action::impl_transactional_action!(PaymentAction);

#[tokio::test]
async fn transactional_forward_execution() {
    let compensated = Arc::new(AtomicBool::new(false));
    let action = PaymentAction {
        meta: ActionMetadata::builder("payment", "Payment").build(),
        compensated: compensated.clone(),
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let r = harness.step(serde_json::json!({})).await.expect("execute_tx");
    match r {
        nebula_action::ActionResult::Break { output, reason } => {
            let conf: Confirmation =
                serde_json::from_value(output.into_value().unwrap()).unwrap();
            assert_eq!(conf.tx_id, "tx_123");
            assert_eq!(conf.amount, 1000);
            assert_eq!(reason, BreakReason::Completed);
        }
        other => panic!("expected Break, got {:?}", other),
    }
    assert!(!compensated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn transactional_compensation() {
    let compensated = Arc::new(AtomicBool::new(false));
    let action = PaymentAction {
        meta: ActionMetadata::builder("payment", "Payment").build(),
        compensated: compensated.clone(),
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let _ = harness.step(serde_json::json!({})).await.expect("execute_tx");
    let r2 = harness.step(serde_json::json!({})).await.expect("compensate");
    match r2 {
        nebula_action::ActionResult::Break { reason, .. } => {
            assert_eq!(reason, BreakReason::Custom("compensated".into()));
        }
        other => panic!("expected Break(compensated), got {:?}", other),
    }
    assert!(compensated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn transactional_double_compensation_fails() {
    let compensated = Arc::new(AtomicBool::new(false));
    let action = PaymentAction {
        meta: ActionMetadata::builder("payment", "Payment").build(),
        compensated,
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx);

    let _ = harness.step(serde_json::json!({})).await.unwrap();
    let _ = harness.step(serde_json::json!({})).await.unwrap();
    let r3 = harness.step(serde_json::json!({})).await;
    nebula_action::assert_fatal!(r3);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action dx_transactional`
Expected: all 3 PASS

**Step 3: Commit**

```
test(action): TransactionalAction integration tests
```

---

### Task 11: Wire up exports and prelude

**Files:**
- Modify: `crates/action/src/lib.rs`
- Modify: `crates/action/src/prelude.rs`

**Step 1: Add `stateful` re-exports to `lib.rs`**

In the public re-exports section, add:

```rust
pub use stateful::{
    BatchAction, BatchItemResult, BatchState, PageResult, PaginatedAction, PaginationState,
    StatefulAction, TransactionPhase, TransactionState, TransactionalAction,
};
```

Keep the existing `pub use execution::StatefulAction` line (both re-export the same type via `execution` re-exporting from `stateful`).

**Step 2: Add to prelude**

```rust
pub use crate::stateful::{
    BatchAction, BatchItemResult, PageResult, PaginatedAction, TransactionalAction,
};
```

**Step 3: Run full check + clippy + tests**

Run: `cargo fmt && cargo clippy -p nebula-action -- -D warnings && cargo nextest run -p nebula-action`
Expected: all pass

**Step 4: Commit**

```
feat(action): wire up stateful DX types in exports and prelude
```

---

### Task 12: Workspace validation + context docs

**Files:**
- Modify: `.claude/crates/action.md`

**Step 1: Run workspace check**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`
Expected: all pass

**Step 2: Update `.claude/crates/action.md`**

Add to "Key Decisions":

```
- `stateful.rs`: core `StatefulAction` (moved from `execution.rs`) + DX traits (`PaginatedAction`, `BatchAction`, `TransactionalAction`) + `macro_rules!` macros (`impl_paginated_action!`, `impl_batch_action!`, `impl_transactional_action!`). Macros generate `impl StatefulAction for $ty` — no blanket impls (Rust coherence forbids multiple). Engine never sees DX types. `execution.rs` re-exports `StatefulAction` for backward compat.
- `migrate_state(old: Value) -> Option<Self::State>` — default method on `StatefulAction`. Adapter calls on state deser failure. Returns `None` by default (error propagated).
- `ActionResult::continue_with()`, `break_completed()`, `break_with_reason()`, `continue_with_delay()` — convenience constructors for stateful iteration results.
```

Add to "Traps":

```
- Must call `impl_paginated_action!(MyType)` after `impl PaginatedAction for MyType` — the macro generates the `StatefulAction` impl. Forgetting the macro call means the type won't work with `register_stateful()`.
- A type cannot use two DX macros (e.g., both `impl_paginated_action!` and `impl_batch_action!`) — they both generate `StatefulAction` impl, causing a duplicate impl error. Choose one pattern per type.
- `BatchAction::process_item` returning `ActionError::Fatal` aborts the entire batch immediately. Use `ActionError::Retryable` for per-item errors that should be captured and continued.
```

**Step 3: Commit**

```
docs(action): update context docs for Phase 6 stateful DX types
```

---

## Exit Criteria Verification

| Criterion                                       | Test                                                    |
|-------------------------------------------------|---------------------------------------------------------|
| PaginatedAction with 3-page test passes         | `dx_paginated::paginated_fetches_all_pages`             |
| BatchAction processes items in chunks            | `dx_batch::batch_processes_in_chunks`                   |
| TransactionalAction stores/retrieves comp. data | `dx_transactional::transactional_compensation`          |
| migrate_state round-trip: v1 -> v2 migration    | `execution_integration::migrate_state_succeeds_from_v1` |
| All 3 DX types compile to core traits           | `StatefulTestHarness` usage in all tests                |

## Design Notes

**Why `macro_rules!` not blanket impls:** Rust's coherence checker rejects `impl<A: TraitA> Target for A` + `impl<A: TraitB> Target for A` — it can't prove no type implements both. Each `macro_rules!` invocation generates a concrete impl for a specific type, avoiding coherence entirely.

**Phase 7 precedent:** The same `macro_rules!` pattern applies to trigger DX types: `impl_webhook_trigger!`, `impl_poll_trigger!`, etc. generating `impl TriggerAction for $ty`.

**Escape hatch:** Action authors who need custom behavior can skip the DX trait entirely and implement `StatefulAction` directly — the macros are convenience, not mandatory.
