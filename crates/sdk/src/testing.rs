//! Testing utilities for action authors.
//!
//! Drives `ActionResult` assertions and produces lightweight identifier
//! fixtures. The canonical end-to-end harness is
//! [`TestRuntime`](crate::runtime::TestRuntime) — it builds an
//! `ActionContext` via [`TestContextBuilder`](nebula_action::testing::TestContextBuilder)
//! and executes the action through the full lifecycle. Reach for the helpers
//! in this module when you only want to inspect the `Result` shape.
//!
//! # Example
//!
//! ```rust,no_run
//! # use nebula_sdk::testing::{assert_success, fixtures};
//! # use nebula_action::{ActionError, ActionResult};
//! # let result: Result<ActionResult<serde_json::Value>, ActionError> =
//! #     Ok(ActionResult::success(serde_json::json!({})));
//! assert_success(&result);
//! let workflow = fixtures::workflow_id();
//! ```

/// Returns `true` when the action result is `Ok(ActionResult::Success { .. })`.
pub fn is_success<T>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) -> bool {
    matches!(result, Ok(nebula_action::ActionResult::Success { .. }))
}

/// Returns `true` when the action result is anything other than success
/// (engine error, validation failure, or non-success `ActionResult` variant).
pub fn is_failure<T>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) -> bool {
    !is_success(result)
}

/// Asserts the action result is success — panics otherwise with the
/// debug-printed result for diagnosis.
pub fn assert_success<T: std::fmt::Debug>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) {
    assert!(is_success(result), "Expected success, got: {result:?}");
}

/// Asserts the action result is a failure — panics otherwise.
pub fn assert_failure<T: std::fmt::Debug>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) {
    assert!(is_failure(result), "Expected failure, got: {result:?}");
}

/// Lightweight identifier fixtures.
///
/// `execution_id()` returns a fresh UUID per call so concurrent tests do not
/// collide. `workflow_id()` is intentionally stable (`"test-workflow-001"`)
/// because tests typically share one workflow across many executions; if your
/// tests need unique workflow IDs, generate them inline.
pub mod fixtures {
    /// Stable workflow identifier shared across test invocations.
    #[must_use]
    pub fn workflow_id() -> String {
        "test-workflow-001".to_string()
    }

    /// Fresh UUID-shaped execution identifier — unique per call.
    #[must_use]
    pub fn execution_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Current timestamp in UTC.
    #[must_use]
    pub fn timestamp() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::{ActionError, ActionResult};
    use serde_json::Value;

    use super::{fixtures, is_failure, is_success};

    #[test]
    fn is_success_matches_success_variant() {
        let ok: Result<ActionResult<Value>, ActionError> = Ok(ActionResult::success(Value::Null));
        assert!(is_success(&ok));
        assert!(!is_failure(&ok));
    }

    #[test]
    fn is_failure_matches_err_and_non_success_variants() {
        let err: Result<ActionResult<Value>, ActionError> = Err(ActionError::fatal("boom"));
        assert!(!is_success(&err));
        assert!(is_failure(&err));
    }

    #[test]
    fn fixtures_produce_non_empty_ids() {
        assert!(!fixtures::workflow_id().is_empty());
        assert!(!fixtures::execution_id().is_empty());
        assert!(fixtures::timestamp().timestamp() > 0);
    }
}
