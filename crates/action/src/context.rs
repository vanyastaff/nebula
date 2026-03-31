//! Execution context types and traits.
//!
//! [`Context`] is the base trait for action execution. [`ActionContext`] is the
//! stable context for StatelessAction/StatefulAction/ResourceAction;
//! [`TriggerContext`] is used by TriggerAction.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

use crate::capability::{
    ActionLogger, CredentialAccessor, ExecutionEmitter, ResourceAccessor, TriggerScheduler,
    default_action_logger, default_credential_accessor, default_execution_emitter,
    default_resource_accessor, default_trigger_scheduler,
};
use crate::error::ActionError;
use nebula_credential::CredentialSnapshot;

/// Base trait for action execution contexts.
///
/// Engine/runtime/sandbox provide concrete implementations; actions receive `&impl Context`.
pub trait Context: Send + Sync {
    /// Execution identity.
    fn execution_id(&self) -> ExecutionId;
    /// Node identity within the workflow.
    fn node_id(&self) -> NodeId;
    /// Workflow identity.
    fn workflow_id(&self) -> WorkflowId;
    /// Cancellation token; action may check before or during work.
    fn cancellation(&self) -> &CancellationToken;
}

/// Stable execution context for StatelessAction, StatefulAction, and ResourceAction.
///
/// Composes execution identity and cancellation. Capability modules (resources,
/// credentials, logger) can be added as fields by the runtime/sandbox without
/// changing this crate's API.
#[derive(Clone)]
pub struct ActionContext {
    /// Execution identity.
    pub execution_id: ExecutionId,
    /// Node identity within the workflow.
    pub node_id: NodeId,
    /// Workflow identity.
    pub workflow_id: WorkflowId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
    /// Resource access capability.
    pub resources: Arc<dyn ResourceAccessor>,
    /// Credential access capability.
    pub credentials: Arc<dyn CredentialAccessor>,
    /// Action-scoped logger capability.
    pub logger: Arc<dyn ActionLogger>,
}

impl Context for ActionContext {
    fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    fn node_id(&self) -> NodeId {
        self.node_id
    }
    fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl ActionContext {
    /// Create a new action context.
    #[must_use]
    pub fn new(
        execution_id: ExecutionId,
        node_id: NodeId,
        workflow_id: WorkflowId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            cancellation,
            resources: default_resource_accessor(),
            credentials: default_credential_accessor(),
            logger: default_action_logger(),
        }
    }

    /// Inject a resource accessor capability.
    #[must_use]
    pub fn with_resources(mut self, resources: Arc<dyn ResourceAccessor>) -> Self {
        self.resources = resources;
        self
    }

    /// Inject a credential accessor capability.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<dyn CredentialAccessor>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Inject an action logger capability.
    #[must_use]
    pub fn with_logger(mut self, logger: Arc<dyn ActionLogger>) -> Self {
        self.logger = logger;
        self
    }

    /// Acquire a resource by key through the configured accessor.
    pub async fn resource(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        self.resources.acquire(key).await
    }

    /// Check whether a resource exists.
    pub async fn has_resource(&self, key: &str) -> bool {
        self.resources.exists(key).await
    }

    /// Retrieve a credential snapshot by id through the configured accessor.
    pub async fn credential(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
        self.credentials.get(id).await
    }

    /// Check whether a credential exists.
    pub async fn has_credential(&self, id: &str) -> bool {
        self.credentials.has(id).await
    }
}

impl fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionContext")
            .field("execution_id", &self.execution_id)
            .field("node_id", &self.node_id)
            .field("workflow_id", &self.workflow_id)
            .field("cancellation", &self.cancellation)
            .field("resources", &"<dyn ResourceAccessor>")
            .field("credentials", &"<dyn CredentialAccessor>")
            .field("logger", &"<dyn ActionLogger>")
            .finish()
    }
}

/// Context for TriggerAction (workflow starters).
///
/// Triggers live outside a specific execution; they start new executions.
/// Composes workflow/trigger identity and cancellation; scheduler and emitter
/// are provided by runtime.
#[derive(Clone)]
pub struct TriggerContext {
    /// Workflow this trigger belongs to.
    pub workflow_id: WorkflowId,
    /// Trigger (node) identity.
    pub trigger_id: NodeId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
    /// Trigger scheduling capability.
    pub scheduler: Arc<dyn TriggerScheduler>,
    /// Execution emission capability.
    pub emitter: Arc<dyn ExecutionEmitter>,
    /// Credential access capability.
    pub credentials: Arc<dyn CredentialAccessor>,
    /// Trigger-scoped logger capability.
    pub logger: Arc<dyn ActionLogger>,
}

impl TriggerContext {
    /// Create a new trigger context.
    #[must_use]
    pub fn new(
        workflow_id: WorkflowId,
        trigger_id: NodeId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            workflow_id,
            trigger_id,
            cancellation,
            scheduler: default_trigger_scheduler(),
            emitter: default_execution_emitter(),
            credentials: default_credential_accessor(),
            logger: default_action_logger(),
        }
    }

    /// Inject a trigger scheduler capability.
    #[must_use]
    pub fn with_scheduler(mut self, scheduler: Arc<dyn TriggerScheduler>) -> Self {
        self.scheduler = scheduler;
        self
    }

    /// Inject an execution emitter capability.
    #[must_use]
    pub fn with_emitter(mut self, emitter: Arc<dyn ExecutionEmitter>) -> Self {
        self.emitter = emitter;
        self
    }

    /// Inject a credential accessor capability.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<dyn CredentialAccessor>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Inject a trigger logger capability.
    #[must_use]
    pub fn with_logger(mut self, logger: Arc<dyn ActionLogger>) -> Self {
        self.logger = logger;
        self
    }

    /// Schedule the next trigger run.
    pub async fn schedule_after(&self, delay: std::time::Duration) -> Result<(), ActionError> {
        self.scheduler.schedule_after(delay).await
    }

    /// Emit a new execution request for this workflow.
    pub async fn emit_execution(
        &self,
        input: serde_json::Value,
    ) -> Result<ExecutionId, ActionError> {
        self.emitter.emit(input).await
    }

    /// Retrieve a credential snapshot by id through the configured accessor.
    pub async fn credential(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
        self.credentials.get(id).await
    }

    /// Check whether a credential exists.
    pub async fn has_credential(&self, id: &str) -> bool {
        self.credentials.has(id).await
    }
}

impl fmt::Debug for TriggerContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerContext")
            .field("workflow_id", &self.workflow_id)
            .field("trigger_id", &self.trigger_id)
            .field("cancellation", &self.cancellation)
            .field("scheduler", &"<dyn TriggerScheduler>")
            .field("emitter", &"<dyn ExecutionEmitter>")
            .field("credentials", &"<dyn CredentialAccessor>")
            .field("logger", &"<dyn ActionLogger>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use async_trait::async_trait;
    use nebula_credential::CredentialMetadata;
    use nebula_credential::CredentialSnapshot;

    use crate::capability::{ActionLogLevel, ActionLogger, ExecutionEmitter, TriggerScheduler};

    struct MockContext {
        token: CancellationToken,
    }
    impl Default for MockContext {
        fn default() -> Self {
            Self {
                token: CancellationToken::new(),
            }
        }
    }
    impl Context for MockContext {
        fn execution_id(&self) -> ExecutionId {
            ExecutionId::nil()
        }
        fn node_id(&self) -> NodeId {
            NodeId::nil()
        }
        fn workflow_id(&self) -> WorkflowId {
            WorkflowId::nil()
        }
        fn cancellation(&self) -> &CancellationToken {
            &self.token
        }
    }

    #[test]
    fn context_trait_object_safety() {
        let ctx = MockContext::default();
        let _: &dyn Context = &ctx;
    }

    #[tokio::test]
    async fn action_context_defaults_to_noop_capabilities() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        );

        assert!(!ctx.has_resource("missing").await);
        assert!(!ctx.has_credential("missing").await);
        assert!(ctx.resource("missing").await.is_err());
        assert!(ctx.credential("missing").await.is_err());
    }

    struct TestScheduler;

    #[async_trait]
    impl TriggerScheduler for TestScheduler {
        async fn schedule_after(&self, _delay: Duration) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestEmitter;

    #[async_trait]
    impl ExecutionEmitter for TestEmitter {
        async fn emit(&self, _input: serde_json::Value) -> Result<ExecutionId, ActionError> {
            Ok(ExecutionId::new())
        }
    }

    struct TestCredentialAccessor;

    #[async_trait]
    impl CredentialAccessor for TestCredentialAccessor {
        async fn get(&self, _id: &str) -> Result<CredentialSnapshot, ActionError> {
            Ok(CredentialSnapshot {
                kind: "api_key".to_string(),
                state: serde_json::json!({"token": "test-token"}),
                metadata: CredentialMetadata::new(),
            })
        }

        async fn has(&self, _id: &str) -> bool {
            true
        }
    }

    struct TestLogger;

    impl ActionLogger for TestLogger {
        fn log(&self, _level: ActionLogLevel, _message: &str) {}
    }

    #[tokio::test]
    async fn trigger_context_with_capabilities_can_schedule_and_emit() {
        let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new())
            .with_scheduler(Arc::new(TestScheduler))
            .with_emitter(Arc::new(TestEmitter))
            .with_credentials(Arc::new(TestCredentialAccessor))
            .with_logger(Arc::new(TestLogger));

        assert!(ctx.schedule_after(Duration::from_millis(5)).await.is_ok());
        assert!(
            ctx.emit_execution(serde_json::json!({"event":"tick"}))
                .await
                .is_ok()
        );
        assert!(ctx.has_credential("cred").await);
        assert!(ctx.credential("cred").await.is_ok());
    }
}
