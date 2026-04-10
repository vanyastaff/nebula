//! Execution context types and traits.
//!
//! [`Context`] is the base trait for action execution. [`ActionContext`] is the
//! stable context for StatelessAction/StatefulAction/ResourceAction;
//! [`TriggerContext`] is used by TriggerAction.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use nebula_core::AuthScheme;
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

use crate::capability::{
    ActionLogger, ExecutionEmitter, ResourceAccessor, TriggerScheduler, default_action_logger,
    default_execution_emitter, default_resource_accessor, default_trigger_scheduler,
};
use crate::error::ActionError;
use nebula_credential::{
    CredentialAccessor, CredentialGuard, CredentialSnapshot, default_credential_accessor,
};

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
}

/// Shared credential-access API for contexts that hold a
/// [`CredentialAccessor`].
///
/// Implemented for both [`ActionContext`] and [`TriggerContext`].
/// This trait exists purely to eliminate the copy-paste of ~100
/// lines of `credential_by_id` / `credential_typed` / `credential` /
/// `has_credential_id` between the two context types — the method
/// bodies were identical except for the `self` type, and had
/// already diverged in minor ways (one had deprecation attributes,
/// the other did not).
///
/// The single required method [`Self::credentials`] yields the
/// underlying accessor; everything else is a default method
/// implementation using that accessor.
///
/// Callers must have this trait in scope:
///
/// ```rust,ignore
/// use nebula_action::CredentialContextExt;
/// // or, equivalently:
/// use nebula_action::prelude::*;
///
/// let token: CredentialGuard<MyScheme> = ctx.credential::<MyScheme>().await?;
/// ```
///
/// # Implementing
///
/// Only the crate types `ActionContext` and `TriggerContext` are
/// expected to implement this trait. Downstream crates can
/// implement it on their own context types; the crate's forward-
/// compatibility contract is:
///
/// - New credential helpers added to this trait will **always** ship
///   with a default implementation built on top of
///   [`Self::credentials`]. Downstream impls will not need to add
///   new methods to stay compatible with new crate versions.
/// - The required method [`Self::credentials`] will not change
///   signature without a major version bump of `nebula-action`.
/// - The trait itself may gain new default methods in minor versions;
///   those default methods are non-breaking additions for implementors.
pub trait CredentialContextExt {
    /// Access the underlying credential accessor.
    ///
    /// Implementors return a reference to their `credentials` field.
    fn credentials(&self) -> &Arc<dyn CredentialAccessor>;

    /// Retrieve a credential snapshot by id through the configured accessor.
    fn credential_by_id(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<CredentialSnapshot, ActionError>> + Send
    where
        Self: Sync,
    {
        async move { self.credentials().get(id).await.map_err(ActionError::from) }
    }

    /// Retrieve a credential and project it to the concrete [`AuthScheme`] type.
    ///
    /// Fetches the snapshot and consumes it via
    /// [`CredentialSnapshot::into_project`], returning the concrete
    /// scheme type.
    ///
    /// # Errors
    ///
    /// - Returns [`ActionError::Fatal`] if the credential does not exist or the
    ///   accessor is not configured.
    /// - Returns [`ActionError::Fatal`] if the stored scheme type does not match
    ///   `S` (scheme mismatch).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let token: SecretToken = ctx.credential_typed::<SecretToken>("api_key").await?;
    /// ```
    fn credential_typed<S: AuthScheme>(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<S, ActionError>> + Send
    where
        Self: Sync,
    {
        async move {
            let snapshot = self
                .credentials()
                .get(id)
                .await
                .map_err(ActionError::from)?;
            snapshot
                .into_project::<S>()
                .map_err(|e| ActionError::fatal(format!("credential `{id}`: {e}")))
        }
    }

    /// Retrieve a typed credential by [`AuthScheme`] type.
    ///
    /// Type IS the key — no string identifier needed. Returns
    /// [`CredentialGuard<S>`] that derefs to `S`, zeroizes on drop,
    /// and cannot be serialized.
    ///
    /// # Errors
    ///
    /// - [`ActionError::Fatal`] if no credential of type `S` is configured
    /// - [`ActionError::Fatal`] if the stored scheme does not match `S`
    fn credential<S>(
        &self,
    ) -> impl std::future::Future<Output = Result<CredentialGuard<S>, ActionError>> + Send
    where
        S: AuthScheme + zeroize::Zeroize,
        Self: Sync,
    {
        async move {
            let type_id = std::any::TypeId::of::<S>();
            let type_name = std::any::type_name::<S>();
            let snapshot = self
                .credentials()
                .get_by_type(type_id, type_name)
                .await
                .map_err(ActionError::from)?;
            let scheme = snapshot.into_project::<S>().map_err(|e| {
                ActionError::fatal(format!("credential type mismatch for `{type_name}`: {e}"))
            })?;
            Ok(CredentialGuard::new(scheme))
        }
    }

    /// Check whether a credential exists by id.
    fn has_credential_id(&self, id: &str) -> impl std::future::Future<Output = bool> + Send
    where
        Self: Sync,
    {
        async move { self.credentials().has(id).await }
    }
}

impl CredentialContextExt for ActionContext {
    fn credentials(&self) -> &Arc<dyn CredentialAccessor> {
        &self.credentials
    }
}

impl CredentialContextExt for TriggerContext {
    fn credentials(&self) -> &Arc<dyn CredentialAccessor> {
        &self.credentials
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
    use nebula_credential::{
        CredentialAccessError, CredentialMetadata, CredentialSnapshot, SecretString, SecretToken,
        scheme::ConnectionUri,
    };

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
        assert!(!ctx.has_credential_id("missing").await);
        assert!(ctx.resource("missing").await.is_err());
        assert!(ctx.credential_by_id("missing").await.is_err());
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
        async fn get(&self, _id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
            Ok(CredentialSnapshot::new(
                "api_key",
                CredentialMetadata::new(),
                SecretToken::new(SecretString::new("test-token")),
            ))
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
        assert!(ctx.has_credential_id("cred").await);
        assert!(ctx.credential_by_id("cred").await.is_ok());
    }

    #[tokio::test]
    async fn action_context_credential_typed_returns_projected_scheme() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor));

        let token: SecretToken = ctx
            .credential_typed::<SecretToken>("api_key")
            .await
            .unwrap();
        token.token().expose_secret(|t| assert_eq!(t, "test-token"));
    }

    #[tokio::test]
    async fn action_context_credential_typed_mismatch_returns_fatal() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor));

        let result = ctx.credential_typed::<ConnectionUri>("api_key").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
        assert!(err.to_string().contains("scheme mismatch"));
    }

    #[tokio::test]
    async fn trigger_context_credential_typed_returns_projected_scheme() {
        let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new())
            .with_credentials(Arc::new(TestCredentialAccessor));

        let token: SecretToken = ctx
            .credential_typed::<SecretToken>("api_key")
            .await
            .unwrap();
        token.token().expose_secret(|t| assert_eq!(t, "test-token"));
    }

    #[tokio::test]
    async fn trigger_context_credential_typed_mismatch_returns_fatal() {
        let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new())
            .with_credentials(Arc::new(TestCredentialAccessor));

        let result = ctx.credential_typed::<ConnectionUri>("api_key").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
        assert!(err.to_string().contains("scheme mismatch"));
    }

    // ── Type-based credential access tests ──────────────────────────────

    /// Test credential type implementing both AuthScheme and Zeroize.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct ZeroizableToken {
        value: String,
    }

    impl nebula_core::AuthScheme for ZeroizableToken {
        fn pattern() -> nebula_core::AuthPattern {
            nebula_core::AuthPattern::SecretToken
        }
    }

    impl zeroize::Zeroize for ZeroizableToken {
        fn zeroize(&mut self) {
            self.value.zeroize();
        }
    }

    /// Accessor that supports `get_by_type` for `ZeroizableToken`.
    struct TypedCredentialAccessor;

    #[async_trait]
    impl CredentialAccessor for TypedCredentialAccessor {
        async fn get(&self, _id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
            Err(CredentialAccessError::NotConfigured(
                "use get_by_type".to_owned(),
            ))
        }

        async fn has(&self, _id: &str) -> bool {
            false
        }

        async fn get_by_type(
            &self,
            type_id: std::any::TypeId,
            type_name: &str,
        ) -> Result<CredentialSnapshot, CredentialAccessError> {
            if type_id == std::any::TypeId::of::<ZeroizableToken>() {
                Ok(CredentialSnapshot::new(
                    "typed",
                    CredentialMetadata::new(),
                    ZeroizableToken {
                        value: "secret-42".to_owned(),
                    },
                ))
            } else {
                Err(CredentialAccessError::NotFound(format!(
                    "no credential for `{type_name}`"
                )))
            }
        }
    }

    #[tokio::test]
    async fn action_context_credential_returns_guard() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn trigger_context_credential_returns_guard() {
        let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new())
            .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn credential_noop_accessor_returns_not_supported() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        );
        let result = ctx.credential::<ZeroizableToken>().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
        assert!(err.to_string().contains("not supported"));
    }
}
