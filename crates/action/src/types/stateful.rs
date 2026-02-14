use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::result::ActionResult;

/// Iterative action with persistent state between executions.
///
/// Used for paginated API scraping, accumulation, polling with cursors,
/// and any workflow node that must remember where it left off.
///
/// The engine manages the state lifecycle:
/// 1. First call: `initialize_state` → creates initial `State`
/// 2. Each iteration: `execute_with_state` → returns `Continue` or `Break`
/// 3. On `Continue`: engine persists state, optionally waits, re-invokes
/// 4. On `Break`: engine finalizes state, passes output downstream
///
/// # State Versioning
///
/// When the state schema changes, implement [`state_version`](Self::state_version)
/// and [`migrate_state`](Self::migrate_state) to handle old persisted states.
///
/// # Type Parameters
///
/// - `State`: persisted between iterations, must be serializable.
/// - `Input`: data received from upstream (typically same each iteration).
/// - `Output`: data produced each iteration (intermediate or final).
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
/// use async_trait::async_trait;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct PaginationState {
///     cursor: Option<String>,
///     pages_fetched: u32,
/// }
///
/// struct PaginatedFetcher { meta: ActionMetadata }
///
/// #[async_trait]
/// impl StatefulAction for PaginatedFetcher {
///     type State = PaginationState;
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn initialize_state(
///         &self, _input: &Self::Input, _ctx: &ActionContext,
///     ) -> Result<Self::State, ActionError> {
///         Ok(PaginationState { cursor: None, pages_fetched: 0 })
///     }
///
///     async fn execute_with_state(
///         &self, input: Self::Input, state: &mut Self::State, ctx: &ActionContext,
///     ) -> Result<ActionResult<Self::Output>, ActionError> {
///         ctx.check_cancelled()?;
///         // ... fetch page using state.cursor ...
///         state.pages_fetched += 1;
///         if state.cursor.is_none() {
///             Ok(ActionResult::Break {
///                 output: input,
///                 reason: BreakReason::Completed,
///             })
///         } else {
///             Ok(ActionResult::Continue {
///                 output: input,
///                 progress: None,
///                 delay: Some(std::time::Duration::from_millis(500)),
///             })
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait StatefulAction: Action {
    /// Persistent state type — serialized between iterations.
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;
    /// Input data received from upstream nodes.
    type Input: Send + Sync + 'static;
    /// Output data produced each iteration.
    type Output: Send + Sync + 'static;

    /// Execute one iteration with mutable access to persistent state.
    ///
    /// Return [`ActionResult::Continue`] to request another iteration,
    /// or [`ActionResult::Break`] to finalize.
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    /// Create the initial state for the first iteration.
    ///
    /// Called once when the engine first executes this action
    /// (no persisted state exists yet).
    async fn initialize_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError>;

    /// Current state schema version.
    ///
    /// Increment when the `State` type changes. The engine compares this
    /// against the version stored with persisted state to decide whether
    /// migration is needed.
    fn state_version(&self) -> u32 {
        1
    }

    /// Migrate state from an older schema version.
    ///
    /// Called when persisted state has a version lower than [`state_version`](Self::state_version).
    /// Default implementation attempts direct deserialization (works if only
    /// fields were added with `#[serde(default)]`).
    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        _old_version: u32,
    ) -> Result<Self::State, ActionError> {
        serde_json::from_value(old_state)
            .map_err(|e| ActionError::fatal(format!("state migration failed: {e}")))
    }
}
