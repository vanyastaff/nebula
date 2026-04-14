//! Testing utilities for Nebula SDK.
//!
//! This module provides helpers for testing actions and workflows.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_sdk::{
//!     prelude::*,
//!     testing::{ActionTester, TestContext},
//! };
//!
//! #[tokio::test]
//! async fn test_my_action() {
//!     let tester = ActionTester::new(MyAction::default());
//!     let result = tester
//!         .execute(MyInput {
//!             name: "test".into(),
//!         })
//!         .await;
//!     assert!(is_success(&result));
//! }
//! ```

use std::collections::HashMap;

/// Test context for action execution.
///
/// Provides a mock execution context for testing.
#[derive(Debug, Clone, Default)]
pub struct TestContext {
    logs: Vec<String>,
    metrics: HashMap<String, f64>,
    variables: HashMap<String, serde_json::Value>,
}

impl TestContext {
    /// Create a new test context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a log entry.
    pub fn log(&mut self, message: impl Into<String>) {
        self.logs.push(message.into());
    }

    /// Get all log entries.
    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    /// Record a metric.
    pub fn record_metric(&mut self, name: impl Into<String>, value: f64) {
        self.metrics.insert(name.into(), value);
    }

    /// Get a recorded metric.
    pub fn metric(&self, name: &str) -> Option<f64> {
        self.metrics.get(name).copied()
    }

    /// Set a variable.
    pub fn set_variable(&mut self, name: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.variables.insert(name.into(), value.into());
    }

    /// Get a variable.
    pub fn variable(&self, name: &str) -> Option<&serde_json::Value> {
        self.variables.get(name)
    }
}

/// Helper for testing actions.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::testing::ActionTester;
///
/// let tester = ActionTester::new(my_action);
/// let result = tester.execute(input).await;
/// ```
pub struct ActionTester<A> {
    #[allow(dead_code)] // used in execute() when A: Action
    action: A,
    context: TestContext,
}

impl<A> ActionTester<A> {
    /// Create a new action tester.
    pub fn new(action: A) -> Self {
        Self {
            action,
            context: TestContext::new(),
        }
    }

    /// Get a reference to the test context.
    pub fn context(&self) -> &TestContext {
        &self.context
    }

    /// Get a mutable reference to the test context.
    pub fn context_mut(&mut self) -> &mut TestContext {
        &mut self.context
    }
}

// TODO: ProcessAction temporarily disabled
// impl<A, I, O> ActionTester<A>
// where
// A: nebula_action::ProcessAction<Input = I, Output = O>,
// I: serde::de::DeserializeOwned + Send + Sync,
// O: serde::Serialize + Send + Sync,
// {
// pub async fn execute(
// &self,
// input: I,
// ) -> Result<nebula_action::ActionResult<O>, nebula_action::ActionError> {
// use nebula_action::ActionContext;
// use nebula_core::{ExecutionId, NodeId, WorkflowId};
//
// let workflow_id = WorkflowId::new();
// let ctx = ActionContext::new(
// ExecutionId::new(),
// NodeId::new(),
// workflow_id,
// tokio_util::sync::CancellationToken::new(),
// );
// self.action.execute(input, &ctx).await
// }
// }

/// Check if an action result is successful.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::testing::is_success;
///
/// let result = action.execute(input, &ctx).await;
/// assert!(is_success(&result));
/// ```
pub fn is_success<T>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) -> bool {
    matches!(result, Ok(nebula_action::ActionResult::Success { .. }))
}

/// Check if an action result is a failure.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::testing::is_failure;
///
/// let result = action.execute(bad_input, &ctx).await;
/// assert!(is_failure(&result));
/// ```
pub fn is_failure<T>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) -> bool {
    !is_success(result)
}

/// Assert that an action result is successful.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::testing::assert_success;
///
/// let result = action.execute(input, &ctx).await;
/// assert_success(&result);
/// ```
pub fn assert_success<T: std::fmt::Debug>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) {
    assert!(is_success(result), "Expected success, got: {:?}", result);
}

/// Assert that an action result is a failure.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::testing::assert_failure;
///
/// let result = action.execute(bad_input, &ctx).await;
/// assert_failure(&result);
/// ```
pub fn assert_failure<T: std::fmt::Debug>(
    result: &Result<nebula_action::ActionResult<T>, nebula_action::ActionError>,
) {
    assert!(is_failure(result), "Expected failure, got: {:?}", result);
}

/// Test fixtures for common scenarios.
pub mod fixtures {
    /// Create a test workflow ID.
    pub fn workflow_id() -> String {
        "test-workflow-001".to_string()
    }

    /// Create a test execution ID.
    pub fn execution_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Create a test timestamp.
    pub fn timestamp() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use super::{TestContext, fixtures};

    #[test]
    fn test_test_context() {
        let mut ctx = TestContext::new();

        ctx.log("Test message");
        assert_eq!(ctx.logs().len(), 1);

        ctx.record_metric("duration", 100.0);
        assert_eq!(ctx.metric("duration"), Some(100.0));

        ctx.set_variable("key", "value");
        assert!(ctx.variable("key").is_some());
    }

    #[test]
    fn test_fixtures() {
        let wf_id = fixtures::workflow_id();
        assert!(!wf_id.is_empty());

        let exec_id = fixtures::execution_id();
        assert!(!exec_id.is_empty());

        let ts = fixtures::timestamp();
        assert!(ts.timestamp() > 0);
    }
}
