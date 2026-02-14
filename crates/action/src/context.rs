use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use nebula_core::scope::ScopeLevel;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::error::ActionError;

/// A string that redacts its contents in Debug and Display.
///
/// Used for credential values to prevent accidental logging.
#[derive(Clone)]
pub struct SecureString {
    inner: String,
}

impl SecureString {
    /// Create a new secure string.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// Access the underlying value.
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl fmt::Debug for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecureString(***)")
    }
}

impl fmt::Display for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Port trait for providing credentials to actions.
///
/// Implemented by the runtime to inject credential resolution into actions
/// without coupling them to the credential storage backend.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Retrieve a credential value by key.
    async fn get(&self, key: &str) -> Result<SecureString, ActionError>;
}

/// Port trait for action-level logging.
///
/// Actions use this to emit structured log messages that are captured
/// by the runtime's logging infrastructure.
pub trait ActionLogger: Send + Sync {
    /// Log a debug message.
    fn debug(&self, message: &str);
    /// Log an info message.
    fn info(&self, message: &str);
    /// Log a warning.
    fn warn(&self, message: &str);
    /// Log an error.
    fn error(&self, message: &str);
}

/// Port trait for action-level metrics.
///
/// Actions use this to emit custom metrics (counters, histograms)
/// that are collected by the runtime's metrics infrastructure.
pub trait ActionMetrics: Send + Sync {
    /// Increment a counter by 1.
    fn counter(&self, name: &str, value: u64);
    /// Record a histogram observation.
    fn histogram(&self, name: &str, value: f64);
}

/// Runtime context provided to every action during execution.
///
/// Constructed by the engine before invoking an action. Provides identity
/// information (which execution, workflow, and node this is), workflow-scoped
/// variables, and a cancellation token.
///
/// Actions **must** periodically call [`check_cancelled`](Self::check_cancelled)
/// in long-running loops to support cooperative cancellation.
#[non_exhaustive]
pub struct ActionContext {
    /// Unique execution run identifier.
    pub execution_id: ExecutionId,
    /// Node in the workflow graph being executed.
    pub node_id: NodeId,
    /// Workflow this execution belongs to.
    pub workflow_id: WorkflowId,
    /// Scope level for resource access control.
    pub scope: ScopeLevel,
    /// Cancellation signal — checked cooperatively by actions.
    pub cancellation: CancellationToken,
    /// Shared workflow-scoped variables.
    variables: Arc<RwLock<serde_json::Map<String, serde_json::Value>>>,
    /// Optional credential provider for accessing secrets.
    credentials: Option<Arc<dyn CredentialProvider>>,
    /// Optional logger for structured action logging.
    logger: Option<Arc<dyn ActionLogger>>,
    /// Optional metrics emitter for custom action metrics.
    metrics: Option<Arc<dyn ActionMetrics>>,
}

impl ActionContext {
    /// Create a new context with the given identifiers.
    pub fn new(
        execution_id: ExecutionId,
        node_id: NodeId,
        workflow_id: WorkflowId,
        scope: ScopeLevel,
    ) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            scope,
            cancellation: CancellationToken::new(),
            variables: Arc::new(RwLock::new(serde_json::Map::new())),
            credentials: None,
            logger: None,
            metrics: None,
        }
    }

    /// Create a context with a pre-existing cancellation token.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Create a context with pre-populated variables.
    pub fn with_variables(mut self, vars: serde_json::Map<String, serde_json::Value>) -> Self {
        self.variables = Arc::new(RwLock::new(vars));
        self
    }

    /// Read a variable from the workflow scope.
    ///
    /// Returns `None` if the variable does not exist.
    pub fn get_variable(&self, key: &str) -> Option<serde_json::Value> {
        self.variables.read().get(key).cloned()
    }

    /// Write a variable to the workflow scope.
    ///
    /// Overwrites any existing variable with the same key.
    pub fn set_variable(&self, key: &str, value: serde_json::Value) {
        self.variables.write().insert(key.to_owned(), value);
    }

    /// Remove a variable from the workflow scope.
    ///
    /// Returns the previous value, if any.
    pub fn remove_variable(&self, key: &str) -> Option<serde_json::Value> {
        self.variables.write().remove(key)
    }

    /// Check whether execution has been cancelled.
    ///
    /// Actions **should** call this in loops and before expensive operations
    /// to support cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Cancelled`] if the token has been triggered.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Attach a credential provider.
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.credentials = Some(provider);
        self
    }

    /// Attach a logger.
    pub fn with_logger(mut self, logger: Arc<dyn ActionLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Attach a metrics emitter.
    pub fn with_metrics(mut self, metrics: Arc<dyn ActionMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Retrieve a credential value by key.
    ///
    /// Returns an error if no credential provider is configured.
    pub async fn credential(&self, key: &str) -> Result<SecureString, ActionError> {
        match &self.credentials {
            Some(provider) => provider.get(key).await,
            None => Err(ActionError::fatal("no credential provider configured")),
        }
    }

    /// Log a debug message. No-op if no logger is attached.
    pub fn log_debug(&self, message: &str) {
        if let Some(logger) = &self.logger {
            logger.debug(message);
        }
    }

    /// Log an info message. No-op if no logger is attached.
    pub fn log_info(&self, message: &str) {
        if let Some(logger) = &self.logger {
            logger.info(message);
        }
    }

    /// Log a warning. No-op if no logger is attached.
    pub fn log_warn(&self, message: &str) {
        if let Some(logger) = &self.logger {
            logger.warn(message);
        }
    }

    /// Log an error. No-op if no logger is attached.
    pub fn log_error(&self, message: &str) {
        if let Some(logger) = &self.logger {
            logger.error(message);
        }
    }

    /// Record a counter metric. No-op if no metrics emitter is attached.
    pub fn record_counter(&self, name: &str, value: u64) {
        if let Some(metrics) = &self.metrics {
            metrics.counter(name, value);
        }
    }

    /// Record a histogram metric. No-op if no metrics emitter is attached.
    pub fn record_histogram(&self, name: &str, value: f64) {
        if let Some(metrics) = &self.metrics {
            metrics.histogram(name, value);
        }
    }
}

impl std::fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionContext")
            .field("execution_id", &self.execution_id)
            .field("node_id", &self.node_id)
            .field("workflow_id", &self.workflow_id)
            .field("scope", &self.scope)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> ActionContext {
        ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        )
    }

    #[test]
    fn get_set_variable() {
        let ctx = test_context();
        assert!(ctx.get_variable("count").is_none());

        ctx.set_variable("count", serde_json::json!(42));
        assert_eq!(ctx.get_variable("count"), Some(serde_json::json!(42)));
    }

    #[test]
    fn overwrite_variable() {
        let ctx = test_context();
        ctx.set_variable("name", serde_json::json!("alice"));
        ctx.set_variable("name", serde_json::json!("bob"));
        assert_eq!(ctx.get_variable("name"), Some(serde_json::json!("bob")));
    }

    #[test]
    fn remove_variable() {
        let ctx = test_context();
        ctx.set_variable("temp", serde_json::json!(true));
        let old = ctx.remove_variable("temp");
        assert_eq!(old, Some(serde_json::json!(true)));
        assert!(ctx.get_variable("temp").is_none());
    }

    #[test]
    fn check_cancelled_ok() {
        let ctx = test_context();
        assert!(ctx.check_cancelled().is_ok());
    }

    #[test]
    fn check_cancelled_after_cancel() {
        let ctx = test_context();
        ctx.cancellation.cancel();
        let err = ctx.check_cancelled().unwrap_err();
        assert!(matches!(err, ActionError::Cancelled));
    }

    #[test]
    fn with_cancellation_token() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = test_context().with_cancellation(child);
        assert!(ctx.check_cancelled().is_ok());
        token.cancel();
        assert!(ctx.check_cancelled().is_err());
    }

    #[test]
    fn with_variables() {
        let mut vars = serde_json::Map::new();
        vars.insert("preset".into(), serde_json::json!("value"));
        let ctx = test_context().with_variables(vars);
        assert_eq!(ctx.get_variable("preset"), Some(serde_json::json!("value")));
    }

    #[test]
    fn debug_format() {
        let ctx = test_context();
        let debug = format!("{ctx:?}");
        assert!(debug.contains("ActionContext"));
        assert!(debug.contains("execution_id"));
    }

    #[test]
    fn secure_string_redacts_debug() {
        let s = SecureString::new("secret123");
        assert_eq!(format!("{s:?}"), "SecureString(***)");
        assert_eq!(format!("{s}"), "***");
        assert_eq!(s.expose(), "secret123");
    }

    #[test]
    fn log_methods_noop_without_logger() {
        let ctx = test_context();
        // These should not panic even without a logger.
        ctx.log_debug("debug");
        ctx.log_info("info");
        ctx.log_warn("warn");
        ctx.log_error("error");
    }

    #[test]
    fn metrics_methods_noop_without_metrics() {
        let ctx = test_context();
        // These should not panic even without metrics.
        ctx.record_counter("requests", 1);
        ctx.record_histogram("latency", 0.5);
    }
}
