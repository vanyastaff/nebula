//! Core [`StatefulAction`] trait and DX convenience patterns.
//!
//! Stateful actions maintain persistent state across iterations, enabling
//! pagination, long-running loops, and multi-step processing. The engine
//! calls [`StatefulAction::execute`] repeatedly — return
//! [`ActionResult::Continue`] for another iteration or [`ActionResult::Break`]
//! when done.

use std::{fmt, future::Future, pin::Pin};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    action::Action,
    context::ActionContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    result::ActionResult,
};

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
/// Cancellation is enforced by the runtime (same as
/// [`StatelessAction`](crate::stateless::StatelessAction)).
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StatefulAction",
    note = "implement `init_state` and `execute` methods with matching Input/Output/State types"
)]
pub trait StatefulAction: Action {
    /// Input type for each iteration.
    ///
    /// Must implement [`HasSchema`](nebula_schema::HasSchema) so the action
    /// metadata can auto-derive its parameter schema from the input type.
    type Input: nebula_schema::HasSchema + Send + Sync;
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

    /// Attempt to migrate state from an older serialized format.
    ///
    /// Called when the engine fails to deserialize a checkpoint into
    /// [`Self::State`]. Return `Some(migrated)` to continue execution with
    /// the migrated state, or `None` to propagate the original
    /// deserialization error as [`ActionError::Validation`].
    fn migrate_state(&self, _old: Value) -> Option<Self::State> {
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
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}

// ── PaginatedAction ─────────────────────────────────────────────────────────

/// Result of fetching a single page.
#[derive(Debug, Clone)]
pub struct PageResult<T, C> {
    /// Data returned by this page.
    pub data: T,
    /// Cursor for the next page, or `None` if this was the last page.
    pub next_cursor: Option<C>,
}

/// Persistent state for pagination (managed by `impl_paginated_action!`).
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
/// `impl_paginated_action!` to generate the [`StatefulAction`] impl.
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
///     async fn fetch_page(&self, input: &Value, cursor: Option<&String>, ctx: &(impl ActionContext + ?Sized))
///         -> Result<PageResult<Vec<Repo>, String>, ActionError> { todo!() }
/// }
/// nebula_action::impl_paginated_action!(ListRepos);
/// ```
pub trait PaginatedAction: Action {
    /// Input type for the paginated request.
    ///
    /// Must implement [`HasSchema`](nebula_schema::HasSchema) so the action
    /// metadata can auto-derive its parameter schema from the input type.
    type Input: nebula_schema::HasSchema + Send + Sync;
    /// Output type produced per page.
    type Output: Send + Sync;
    /// Cursor type for tracking pagination position.
    type Cursor: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Maximum pages before forcing a break. Default: 100.
    ///
    /// Must be `>= 1` — the generated `StatefulAction::execute` body
    /// `debug_assert!`s this at runtime and silently coerces to 1 in
    /// release mode. Returning 0 means "fetch zero pages", which is
    /// almost certainly a bug; use a `StatelessAction` if you do not
    /// want to iterate.
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
        ctx: &(impl ActionContext + ?Sized),
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
                ctx: &(impl $crate::context::ActionContext + ?Sized),
            ) -> ::core::result::Result<
                $crate::result::ActionResult<Self::Output>,
                $crate::error::ActionError,
            > {
                use $crate::stateful::PaginatedAction as _;
                let result = self.fetch_page(&input, state.cursor.as_ref(), ctx).await?;
                state.cursor.clone_from(&result.next_cursor);
                state.pages_fetched = state.pages_fetched.saturating_add(1);

                // Contract: max_pages() must return >= 1. Debug builds
                // trip an assertion so the bug is visible in tests;
                // release builds coerce to 1 to avoid an infinite loop.
                debug_assert!(
                    self.max_pages() >= 1,
                    "PaginatedAction::max_pages() must return >= 1, got 0"
                );
                let max = self.max_pages().max(1);
                if result.next_cursor.is_some() && state.pages_fetched < max {
                    let progress = state.pages_fetched as f64 / max as f64;
                    Ok($crate::result::ActionResult::continue_with(
                        result.data,
                        Some(progress),
                    ))
                } else if result.next_cursor.is_some() {
                    // Hit max_pages limit — truncated
                    Ok($crate::result::ActionResult::break_with_reason(
                        result.data,
                        $crate::result::BreakReason::MaxIterations,
                    ))
                } else {
                    // No more pages — natural completion
                    Ok($crate::result::ActionResult::break_completed(result.data))
                }
            }
        }
    };
}

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

/// Persistent state for batch processing (managed by `impl_batch_action!`).
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
/// `impl_batch_action!` to generate the [`StatefulAction`] impl.
///
/// [`ActionError::Fatal`] from `process_item` aborts the entire batch.
/// Other errors are captured as [`BatchItemResult::Failed`].
pub trait BatchAction: Action {
    /// Input type containing the items to process.
    ///
    /// Must implement [`HasSchema`](nebula_schema::HasSchema) so the action
    /// metadata can auto-derive its parameter schema from the input type.
    type Input: nebula_schema::HasSchema + Send + Sync;
    /// Individual work item type.
    type Item: Serialize + DeserializeOwned + Clone + Send + Sync;
    /// Output type per item and as the final merged result.
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Items per iteration. Default: 50.
    fn batch_size(&self) -> usize {
        50
    }

    /// Extract work items from input. Called once on the first iteration.
    fn extract_items(&self, input: &Self::Input) -> Vec<Self::Item>;

    /// Process a single item.
    ///
    /// [`ActionError::Fatal`] aborts the batch. Other errors captured per-item.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] for item-level failures.
    fn process_item(
        &self,
        item: Self::Item,
        ctx: &(impl ActionContext + ?Sized),
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
                ctx: &(impl $crate::context::ActionContext + ?Sized),
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
                        Ok(output) => state
                            .results
                            .push($crate::stateful::BatchItemResult::Ok { output }),
                        Err(e) => {
                            if e.is_fatal() {
                                return Err(e);
                            }
                            state
                                .results
                                .push($crate::stateful::BatchItemResult::Failed {
                                    error: e.to_string(),
                                });
                        },
                    }
                }
                state.chunks_processed = state.chunks_processed.saturating_add(1);

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

// `TransactionalAction` + `TransactionPhase` + `TransactionState` +
// `impl_transactional_action!` were removed on 2026-04-10 (M1). The
// three-phase saga state machine was unreachable past the first
// phase: `runtime::execute_stateful` only loops on
// `ActionResult::Continue`, and the Pending arm returned
// `break_completed(...)` — so the Executed / Compensated branches
// were dead code under normal engine dispatch. The doc acknowledged
// "engine-level saga orchestration is post-v1" but the trait looked
// like a working three-phase saga to readers.
//
// Real saga orchestration (rollback triggers, saga state store,
// compensation DAG) will land as an engine-level feature post-v1.
// When it does, it will ship its own trait shape that cooperates
// with the orchestrator, not a re-imagined `TransactionalAction`.
// See `.claude/decisions.md` for the rationale note.

// ── StatefulHandler trait ───────────────────────────────────────────────────

/// Stateful action handler — JSON in, mutable JSON state, JSON out.
///
/// The engine calls `execute` repeatedly. State is persisted as JSON between
/// iterations for checkpointing. Return [`ActionResult::Continue`] for another
/// iteration or [`ActionResult::Break`] when done.
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
pub trait StatefulHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Create initial state as JSON for the first iteration.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if the initial state cannot be produced
    /// (e.g., serialization failure in an adapter).
    fn init_state(&self) -> Result<Value, ActionError>;

    /// Attempt to migrate state from a previous version.
    ///
    /// Called when state deserialization fails during `execute`. Returns
    /// migrated state as JSON, or `None` to propagate the error.
    fn migrate_state(&self, old: Value) -> Option<Value> {
        let _ = old;
        None
    }

    /// Execute one iteration with mutable JSON state.
    ///
    /// # State checkpointing
    ///
    /// Implementations MUST flush any state mutations back to `state` before
    /// returning, regardless of whether the iteration succeeded. Returning
    /// `Err(Retryable)` after mutating internal typed state without
    /// checkpointing causes the engine to re-run the iteration against a
    /// stale snapshot — partial work is replayed and external side effects
    /// (API calls, DB writes, emits) are duplicated.
    ///
    /// The only exception is when state deserialization fails at the start
    /// of the iteration — in that case no mutations could have occurred and
    /// there is nothing to flush.
    ///
    /// # Cancellation (cancel-on-drop contract)
    ///
    /// The runtime races this future against `ctx.cancellation().cancelled()`
    /// via `tokio::select!`. When the cancellation token fires mid-`await`,
    /// the runtime **drops the execute future**, which cancels all nested
    /// futures at their next `.await` point. Implementations whose
    /// mid-`await` state cannot safely be dropped (in-flight DB transactions,
    /// outgoing HTTP requests that must be completed, …) must either:
    ///
    /// 1. Guard the critical section behind a `tokio::select!` with a longer grace period and a
    ///    compensating rollback, or
    /// 2. Use `tokio::task::spawn` for the critical section and await the join handle — dropping
    ///    the spawned task still leaks it, but the caller can wait for it to settle.
    ///
    /// The runtime will NOT poll the future to completion after
    /// cancellation fires — a stuck handler cannot stall cancellation
    /// (#304).
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    fn execute<'life0, 'life1, 'life2, 'life3, 'a>(
        &'life0 self,
        input: &'life1 Value,
        state: &'life2 mut Value,
        ctx: &'life3 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
        'life2: 'a,
        'life3: 'a;
}

// ── StatefulActionAdapter ───────────────────────────────────────────────────

/// Wraps a [`StatefulAction`] as a [`dyn StatefulHandler`].
///
/// Handles JSON (de)serialization of input, output, and state so the runtime
/// works with untyped JSON while action authors write strongly-typed Rust.
///
/// State is serialized to/from `serde_json::Value` between iterations for
/// engine checkpointing.
pub struct StatefulActionAdapter<A> {
    action: A,
}

impl<A> StatefulActionAdapter<A> {
    /// Wrap a typed stateful action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

impl<A> StatefulHandler for StatefulActionAdapter<A>
where
    A: StatefulAction + Send + Sync + 'static,
    A::Input: DeserializeOwned + Send + Sync,
    A::Output: Serialize + Send + Sync,
    A::State: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn init_state(&self) -> Result<Value, ActionError> {
        serde_json::to_value(self.action.init_state())
            .map_err(|e| ActionError::fatal(format!("init_state serialization failed: {e}")))
    }

    fn migrate_state(&self, old: Value) -> Option<Value> {
        // If the migrated state can't be serialized back to JSON, treat as migration failure.
        // This is acceptable because the alternative (a different error) would be more confusing.
        self.action
            .migrate_state(old)
            .and_then(|state| serde_json::to_value(state).ok())
    }

    /// Execute one iteration, deserializing input and state from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Validation`] if input or state deserialization fails,
    /// or propagates errors from the underlying action.
    ///
    /// # State checkpointing invariant
    ///
    /// State mutations performed by the typed action are flushed back to
    /// `state` **before** any error from `action.execute()` is propagated.
    /// If the typed action increments a counter or advances a cursor and
    /// then returns [`ActionError::Retryable`], the engine checkpoints the
    /// new state — retries resume from the mutated position instead of
    /// replaying completed work (which would duplicate API calls, double
    /// charges, and double emits).
    ///
    /// The only path that does NOT checkpoint is `Validation` raised while
    /// deserializing input or state — in that case `typed_state` was never
    /// created and cannot have been mutated.
    fn execute<'life0, 'life1, 'life2, 'life3, 'a>(
        &'life0 self,
        input: &'life1 Value,
        state: &'life2 mut Value,
        ctx: &'life3 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
        'life2: 'a,
        'life3: 'a,
    {
        Box::pin(async move {
            // Adapter clones input ONCE per iteration to deserialize into typed A::Input.
            let typed_input: A::Input = serde_json::from_value(input.clone()).map_err(|e| {
                ActionError::validation(
                    "input",
                    ValidationReason::MalformedJson,
                    Some(e.to_string()),
                )
            })?;

            // Happy path: one clone for `from_value`. Migration path (rare,
            // version skew between stored checkpoint and current State schema):
            // a second clone only when the first deserialization fails and
            // `migrate_state` is actually consulted.
            let mut typed_state: A::State = match serde_json::from_value::<A::State>(state.clone())
            {
                Ok(s) => s,
                Err(e) => self.action.migrate_state(state.clone()).ok_or_else(|| {
                    ActionError::validation(
                        "state",
                        ValidationReason::StateDeserialization,
                        Some(e.to_string()),
                    )
                })?,
            };

            // Run the typed action. typed_state may be mutated regardless of
            // Ok/Err — flush it back to JSON before propagating so that a
            // Retryable does not replay completed work on retry.
            let action_result = self
                .action
                .execute(typed_input, &mut typed_state, ctx)
                .await;

            // Flatten the 2D decision (serialize-success × action-result) into
            // a single tuple match — easier to audit than nested arms, which
            // matters on a checkpoint-critical code path.
            match (serde_json::to_value(&typed_state), &action_result) {
                (Ok(new_state), _) => {
                    *state = new_state;
                },
                (Err(ser_err), Ok(_)) => {
                    // Success path: surface the serialization failure as fatal.
                    return Err(ActionError::fatal(format!(
                        "state serialization failed: {ser_err}"
                    )));
                },
                (Err(ser_err), Err(action_err)) => {
                    // Error path: the action error is the actionable signal.
                    // Log the serde failure forensically and let the original
                    // error propagate — masking it would break retry classification.
                    tracing::error!(
                        action = %self.action.metadata().base.key,
                        serialization_error = %ser_err,
                        action_error = %action_err,
                        "stateful adapter: state serialization failed on error path; \
                         checkpoint lost, propagating original action error"
                    );
                },
            }

            let result = action_result?;

            result.try_map_output(|output| {
                serde_json::to_value(output)
                    .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
            })
        })
    }
}

impl<A: Action> fmt::Debug for StatefulActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StatefulActionAdapter")
            .field("action", &self.action.metadata().base.key)
            .finish_non_exhaustive()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::DeclaresDependencies;

    use super::*;
    use crate::{
        output::ActionOutput,
        result::BreakReason,
        testing::{TestActionContext, TestContextBuilder},
    };

    fn make_ctx() -> TestActionContext {
        TestContextBuilder::new().build()
    }

    // ── StatefulActionAdapter tests ───────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CounterState {
        count: u32,
    }

    struct CounterAction {
        meta: ActionMetadata,
    }

    impl CounterAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.counter"),
                    "Counter",
                    "Counts up to 3",
                ),
            }
        }
    }

    impl DeclaresDependencies for CounterAction {}

    impl Action for CounterAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatefulAction for CounterAction {
        type Input = Value;
        type Output = Value;
        type State = CounterState;

        fn init_state(&self) -> CounterState {
            CounterState { count: 0 }
        }

        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count += 1;
            if state.count >= 3 {
                Ok(ActionResult::Break {
                    output: ActionOutput::Value(serde_json::json!({"final": state.count})),
                    reason: BreakReason::Completed,
                })
            } else {
                Ok(ActionResult::Continue {
                    output: ActionOutput::Value(serde_json::json!({"current": state.count})),
                    progress: Some(state.count as f64 / 3.0),
                    delay: None,
                })
            }
        }
    }

    #[test]
    fn stateful_adapter_is_dyn_compatible() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let _: Arc<dyn StatefulHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateful_adapter_init_state_serializes() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let state = adapter.init_state().unwrap();
        let cs: CounterState = serde_json::from_value(state).unwrap();
        assert_eq!(cs.count, 0);
    }

    #[tokio::test]
    async fn stateful_adapter_iterates_with_state() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let handler: Arc<dyn StatefulHandler> = Arc::new(adapter);
        let ctx = make_ctx();
        let mut state = handler.init_state().unwrap();

        // Iteration 1: count goes 0 → 1, Continue
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Continue { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 1);

        // Iteration 2: count goes 1 → 2, Continue
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Continue { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 2);

        // Iteration 3: count goes 2 → 3, Break
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Break { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 3);
    }

    #[tokio::test]
    async fn stateful_adapter_returns_validation_error_on_bad_state() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let ctx = make_ctx();
        let mut bad_state = serde_json::json!("not a counter state");

        let err = adapter
            .execute(&serde_json::json!({}), &mut bad_state, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation { .. }));
    }

    /// Action that mutates state to `mark` then fails with the configured error.
    ///
    /// Used to prove that `StatefulActionAdapter::execute` flushes state
    /// back to JSON before propagating errors — critical for avoiding
    /// duplicated side effects on retry.
    struct MutateThenFailAction {
        meta: ActionMetadata,
        fail_with: ActionError,
        mark: u32,
    }

    impl DeclaresDependencies for MutateThenFailAction {}

    impl Action for MutateThenFailAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatefulAction for MutateThenFailAction {
        type Input = Value;
        type Output = Value;
        type State = CounterState;

        fn init_state(&self) -> CounterState {
            CounterState { count: 0 }
        }

        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count = self.mark;
            Err(self.fail_with.clone())
        }
    }

    fn mutate_fail(fail_with: ActionError, mark: u32) -> MutateThenFailAction {
        MutateThenFailAction {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.mutate_fail"),
                "MutateFail",
                "Mutates state then fails",
            ),
            fail_with,
            mark,
        }
    }

    #[tokio::test]
    async fn stateful_adapter_checkpoints_state_on_retryable_error() {
        // Prove: an action that advances cursor/counter state and then returns
        // Retryable must have its mutations flushed to the JSON state so the
        // engine checkpoints the new position. Otherwise retry replays
        // completed work.
        let adapter = StatefulActionAdapter::new(mutate_fail(
            ActionError::retryable("transient upstream error"),
            42,
        ));
        let ctx = make_ctx();
        let mut state = serde_json::json!({ "count": 0 });

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(err.is_retryable(), "error must still be retryable");
        assert_eq!(
            state,
            serde_json::json!({ "count": 42 }),
            "state mutations before Err must be checkpointed"
        );
    }

    #[tokio::test]
    async fn stateful_adapter_checkpoints_state_on_fatal_error() {
        // Symmetric invariant: even fatal errors must checkpoint state, so
        // an operator debugging the failure can see the position at which
        // the action gave up.
        let adapter =
            StatefulActionAdapter::new(mutate_fail(ActionError::fatal("schema mismatch"), 7));
        let ctx = make_ctx();
        let mut state = serde_json::json!({ "count": 0 });

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(err.is_fatal());
        assert_eq!(state, serde_json::json!({ "count": 7 }));
    }

    #[tokio::test]
    async fn stateful_adapter_preserves_state_on_validation_error() {
        // Deserialization failure happens BEFORE the typed action runs, so
        // typed_state never existed and nothing should be written back. The
        // input JSON must remain verbatim so the engine can decide how to
        // recover (e.g., schema migration path outside the adapter).
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let ctx = make_ctx();
        let bad = serde_json::json!("not a counter state");
        let mut state = bad.clone();

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(matches!(err, ActionError::Validation { .. }));
        assert_eq!(state, bad, "state must be untouched on deser failure");
    }

    #[test]
    fn stateful_adapter_into_inner_returns_action() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().base.key,
            nebula_core::action_key!("test.counter")
        );
    }
}
