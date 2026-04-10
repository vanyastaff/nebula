//! Core [`StatefulAction`] trait and DX convenience patterns.
//!
//! Stateful actions maintain persistent state across iterations, enabling
//! pagination, long-running loops, and multi-step processing. The engine
//! calls [`StatefulAction::execute`] repeatedly — return
//! [`ActionResult::Continue`] for another iteration or [`ActionResult::Break`]
//! when done.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
/// Cancellation is enforced by the runtime (same as [`StatelessAction`](crate::stateless::StatelessAction)).
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

    /// Attempt to migrate state from an older serialized format.
    ///
    /// Called when the engine fails to deserialize a checkpoint into
    /// [`Self::State`]. Return `Some(migrated)` to continue execution with
    /// the migrated state, or `None` to propagate the original
    /// deserialization error as [`ActionError::Validation`].
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
                state.pages_fetched = state.pages_fetched.saturating_add(1);

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
    type Input: Send + Sync;
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
                        }
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
