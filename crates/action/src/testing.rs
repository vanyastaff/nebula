//! Test utilities for action authors.
//!
//! Provides [`TestContextBuilder`] for constructing [`ActionContext`] in tests
//! without needing real credential/resource providers.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use nebula_credential::CredentialSnapshot;
use tokio_util::sync::CancellationToken;

use crate::capability::{ActionLogLevel, ActionLogger, CredentialAccessor};
use crate::context::ActionContext;
use crate::error::ActionError;

/// Builder for creating test [`ActionContext`] instances.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::testing::TestContextBuilder;
///
/// let ctx = TestContextBuilder::new()
///     .with_credential_snapshot("api_key", snapshot)
///     .build();
/// ```
pub struct TestContextBuilder {
    credentials: HashMap<String, CredentialSnapshot>,
    logs: Arc<SpyLogger>,
}

impl TestContextBuilder {
    /// Create a new test context builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
            logs: Arc::new(SpyLogger::new()),
        }
    }

    /// Add a typed credential snapshot for testing.
    ///
    /// The credential is stored as a [`CredentialSnapshot`] and returned
    /// by the test credential accessor when requested by `key`.
    #[must_use]
    pub fn with_credential_snapshot(
        mut self,
        key: impl Into<String>,
        snapshot: CredentialSnapshot,
    ) -> Self {
        self.credentials.insert(key.into(), snapshot);
        self
    }

    /// Get the spy logger for checking logged messages after execution.
    #[must_use]
    pub fn spy_logger(&self) -> Arc<SpyLogger> {
        Arc::clone(&self.logs)
    }

    /// Build the test context.
    #[must_use]
    pub fn build(self) -> ActionContext {
        ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor {
            credentials: self.credentials,
        }))
        .with_logger(self.logs)
    }
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Logger that captures log entries for test assertions.
pub struct SpyLogger {
    entries: parking_lot::Mutex<Vec<(ActionLogLevel, String)>>,
}

impl SpyLogger {
    /// Create a new spy logger.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get all logged messages (level and text).
    #[must_use]
    pub fn entries(&self) -> Vec<(ActionLogLevel, String)> {
        self.entries.lock().clone()
    }

    /// Get only the message strings.
    #[must_use]
    pub fn messages(&self) -> Vec<String> {
        self.entries.lock().iter().map(|(_, m)| m.clone()).collect()
    }

    /// Check if any entry contains the given substring.
    #[must_use]
    pub fn contains(&self, substring: &str) -> bool {
        self.entries
            .lock()
            .iter()
            .any(|(_, m)| m.contains(substring))
    }

    /// Number of log entries.
    #[must_use]
    pub fn count(&self) -> usize {
        self.entries.lock().len()
    }
}

impl Default for SpyLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SpyLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpyLogger")
            .field("count", &self.count())
            .finish()
    }
}

impl ActionLogger for SpyLogger {
    fn log(&self, level: ActionLogLevel, message: &str) {
        self.entries.lock().push((level, message.to_owned()));
    }
}

struct TestCredentialAccessor {
    credentials: HashMap<String, CredentialSnapshot>,
}

#[async_trait]
impl CredentialAccessor for TestCredentialAccessor {
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
        self.credentials.get(id).cloned().ok_or_else(|| {
            ActionError::fatal(format!("credential `{id}` not found in test context"))
        })
    }

    async fn has(&self, id: &str) -> bool {
        self.credentials.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::SecretString;
    use nebula_credential::{BearerToken, CredentialMetadata};

    use super::*;

    #[test]
    fn test_context_builder_defaults() {
        let builder = TestContextBuilder::new();
        let ctx = builder.build();
        // Should create a valid context with noop-like capabilities.
        let _ = ctx.execution_id;
        let _ = ctx.node_id;
    }

    #[tokio::test]
    async fn test_context_builder_with_credential_snapshot() {
        let snapshot = CredentialSnapshot::new(
            "api_key",
            CredentialMetadata::new(),
            BearerToken::new(SecretString::new("test-secret")),
        );

        let ctx = TestContextBuilder::new()
            .with_credential_snapshot("my_cred", snapshot)
            .build();

        assert!(ctx.has_credential("my_cred").await);
        assert!(!ctx.has_credential("other").await);

        let snap = ctx.credential("my_cred").await.unwrap();
        assert_eq!(snap.scheme_kind(), "bearer");
    }

    #[tokio::test]
    async fn test_context_builder_missing_credential_returns_error() {
        let ctx = TestContextBuilder::new().build();
        let result = ctx.credential("missing").await;
        assert!(result.is_err());
    }

    #[test]
    fn spy_logger_captures_messages() {
        let logger = SpyLogger::new();
        logger.log(ActionLogLevel::Info, "hello world");
        logger.log(ActionLogLevel::Error, "something failed");

        assert_eq!(logger.count(), 2);
        assert!(logger.contains("hello"));
        assert!(!logger.contains("missing"));

        let messages = logger.messages();
        assert_eq!(messages, vec!["hello world", "something failed"]);
    }

    #[test]
    fn spy_logger_shared_via_builder() {
        let builder = TestContextBuilder::new();
        let spy = builder.spy_logger();
        let ctx = builder.build();

        ctx.logger.log(ActionLogLevel::Info, "from action");

        assert_eq!(spy.count(), 1);
        assert!(spy.contains("from action"));
    }
}
