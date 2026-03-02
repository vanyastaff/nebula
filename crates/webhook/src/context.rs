//! Trigger context for webhook operations

use crate::{Environment, TriggerState};
use nebula_resource::{Context, ExecutionId, WorkflowId};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Context for webhook trigger operations
///
/// Provides access to:
/// - Base context (scope, workflow_id, execution_id, cancellation)
/// - Trigger identity (trigger_id, environment-specific UUIDs)
/// - Webhook server (for registration and listening)
/// - Environment isolation (test vs production)
#[derive(Clone)]
pub struct TriggerCtx {
    /// Base resource context
    pub base: Context,

    /// Unique identifier for this trigger within the workflow
    pub trigger_id: String,

    /// Current environment (Test or Production)
    pub env: Environment,

    /// Persistent state with UUIDs for both environments
    pub state: Arc<TriggerState>,

    /// Base URL for the webhook server (e.g., "https://nebula.example.com")
    pub base_url: String,

    /// Path prefix for all webhooks (e.g., "/webhooks")
    pub path_prefix: String,
}

impl TriggerCtx {
    /// Create a new trigger context
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base: Context,
        trigger_id: impl Into<String>,
        env: Environment,
        state: Arc<TriggerState>,
        base_url: impl Into<String>,
        path_prefix: impl Into<String>,
    ) -> Self {
        Self {
            base,
            trigger_id: trigger_id.into(),
            env,
            state,
            base_url: base_url.into(),
            path_prefix: path_prefix.into(),
        }
    }

    /// Get the webhook path for the current environment
    ///
    /// Format: `/{path_prefix}/{env}/{uuid}`
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_webhook::prelude::*;
    /// # use nebula_resource::Context;
    /// # use nebula_resource::Scope;
    /// # use std::sync::Arc;
    /// # let ctx = TriggerCtx::new(
    /// #     Context::new(Scope::Global, nebula_resource::WorkflowId::new(), nebula_resource::ExecutionId::new()),
    /// #     "trigger",
    /// #     Environment::Production,
    /// #     Arc::new(TriggerState::new("trigger")),
    /// #     "https://example.com",
    /// #     "/webhooks",
    /// # );
    /// let path = ctx.webhook_path();
    /// // => "/webhooks/prod/550e8400-e29b-41d4-a716-446655440000"
    /// ```
    pub fn webhook_path(&self) -> String {
        let uuid = self.state.uuid_for_env(&self.env);
        format!("{}/{}/{}", self.path_prefix, self.env.path_prefix(), uuid)
    }

    /// Get the full webhook URL for the current environment
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_webhook::prelude::*;
    /// # use nebula_resource::Context;
    /// # use nebula_resource::Scope;
    /// # use std::sync::Arc;
    /// # let ctx = TriggerCtx::new(
    /// #     Context::new(Scope::Global, nebula_resource::WorkflowId::new(), nebula_resource::ExecutionId::new()),
    /// #     "trigger",
    /// #     Environment::Production,
    /// #     Arc::new(TriggerState::new("trigger")),
    /// #     "https://example.com",
    /// #     "/webhooks",
    /// # );
    /// let url = ctx.webhook_url();
    /// // => "https://example.com/webhooks/prod/550e8400-..."
    /// ```
    pub fn webhook_url(&self) -> String {
        format!("{}{}", self.base_url, self.webhook_path())
    }

    /// Get the UUID for the current environment
    pub fn current_uuid(&self) -> uuid::Uuid {
        self.state.uuid_for_env(&self.env)
    }

    /// Get the cancellation token
    pub fn cancellation(&self) -> &CancellationToken {
        &self.base.cancellation
    }

    /// Check if the operation is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.base.cancellation.is_cancelled()
    }

    /// Get the workflow ID
    pub fn workflow_id(&self) -> &WorkflowId {
        &self.base.workflow_id
    }

    /// Get the execution ID
    pub fn execution_id(&self) -> &ExecutionId {
        &self.base.execution_id
    }

    /// Get metadata value from base context
    pub fn metadata(&self, key: &str) -> Option<&str> {
        self.base.metadata.get(key).map(String::as_str)
    }

    /// Get the tenant ID if present
    pub fn tenant_id(&self) -> Option<&str> {
        self.base.tenant_id.as_deref()
    }

    /// Create a child cancellation token
    pub fn child_cancellation(&self) -> CancellationToken {
        self.base.cancellation.child_token()
    }

    /// Switch to a different environment
    ///
    /// This is primarily used for testing - switching from production
    /// to test environment for the `test()` method.
    pub fn with_environment(&self, env: Environment) -> Self {
        Self {
            env,
            ..self.clone()
        }
    }
}

impl std::fmt::Debug for TriggerCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerCtx")
            .field("trigger_id", &self.trigger_id)
            .field("env", &self.env)
            .field("workflow_id", &self.base.workflow_id)
            .field("execution_id", &self.base.execution_id)
            .field("webhook_path", &self.webhook_path())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::Scope;

    fn create_test_ctx(env: Environment) -> TriggerCtx {
        let wf = WorkflowId::parse("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let ex = ExecutionId::parse("550e8400-e29b-41d4-a716-446655440002").unwrap();
        let base = Context::new(Scope::Global, wf, ex);
        let state = Arc::new(TriggerState::new("test-trigger"));

        TriggerCtx::new(
            base,
            "test-trigger",
            env,
            state,
            "https://nebula.example.com",
            "/webhooks",
        )
    }

    #[test]
    fn test_webhook_path() {
        let ctx = create_test_ctx(Environment::Production);
        let path = ctx.webhook_path();

        assert!(path.starts_with("/webhooks/prod/"));
        assert_eq!(path.split('/').count(), 4);
    }

    #[test]
    fn test_webhook_url() {
        let ctx = create_test_ctx(Environment::Production);
        let url = ctx.webhook_url();

        assert!(url.starts_with("https://nebula.example.com/webhooks/prod/"));
    }

    #[test]
    fn test_environment_specific_uuids() {
        let base = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
        let state = Arc::new(TriggerState::new("test-trigger"));

        let test_ctx = TriggerCtx::new(
            base.clone(),
            "test-trigger",
            Environment::Test,
            state.clone(),
            "https://example.com",
            "/webhooks",
        );

        let prod_ctx = TriggerCtx::new(
            base,
            "test-trigger",
            Environment::Production,
            state,
            "https://example.com",
            "/webhooks",
        );

        // Different environments should have different UUIDs
        assert_ne!(test_ctx.current_uuid(), prod_ctx.current_uuid());

        // But they should be stable
        assert_eq!(test_ctx.current_uuid(), test_ctx.state.test_uuid);
        assert_eq!(prod_ctx.current_uuid(), prod_ctx.state.prod_uuid);
    }

    #[test]
    fn test_with_environment() {
        let ctx = create_test_ctx(Environment::Production);
        let test_ctx = ctx.with_environment(Environment::Test);

        assert_eq!(ctx.env, Environment::Production);
        assert_eq!(test_ctx.env, Environment::Test);

        // Should use different UUIDs
        assert_ne!(ctx.current_uuid(), test_ctx.current_uuid());
    }

    #[test]
    fn test_context_accessors() {
        let ctx = create_test_ctx(Environment::Production);
        let wf = WorkflowId::parse("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let ex = ExecutionId::parse("550e8400-e29b-41d4-a716-446655440002").unwrap();

        assert_eq!(ctx.workflow_id(), &wf);
        assert_eq!(ctx.execution_id(), &ex);
        assert_eq!(ctx.trigger_id, "test-trigger");
        assert!(!ctx.is_cancelled());
    }

    #[test]
    fn test_cancellation() {
        let base = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
        base.cancellation.cancel();

        let state = Arc::new(TriggerState::new("test-trigger"));
        let ctx = TriggerCtx::new(
            base,
            "test-trigger",
            Environment::Production,
            state,
            "https://example.com",
            "/webhooks",
        );

        assert!(ctx.is_cancelled());
    }

    #[test]
    fn test_metadata() {
        let base = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_metadata("region", "us-east-1")
            .with_metadata("env", "staging");

        let state = Arc::new(TriggerState::new("test-trigger"));
        let ctx = TriggerCtx::new(
            base,
            "test-trigger",
            Environment::Production,
            state,
            "https://example.com",
            "/webhooks",
        );

        assert_eq!(ctx.metadata("region"), Some("us-east-1"));
        assert_eq!(ctx.metadata("env"), Some("staging"));
        assert_eq!(ctx.metadata("nonexistent"), None);
    }
}
