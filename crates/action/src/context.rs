//! Execution context types and traits.
//!
//! [`Context`] is the base trait for action execution. [`ActionContext`] is the
//! stable context for StatelessAction/StatefulAction/ResourceAction;
//! [`TriggerContext`] is used by TriggerAction.

use std::{any::Any, fmt, sync::Arc};

use nebula_core::{
    CredentialKey, NodeKey, ResourceKey,
    accessor::{CredentialAccessor, Logger, ResourceAccessor},
    id::{ExecutionId, WorkflowId},
};
use nebula_credential::{AuthScheme, CredentialGuard, CredentialSnapshot};
use tokio_util::sync::CancellationToken;

use crate::{
    capability::{
        ExecutionEmitter, TriggerHealth, TriggerScheduler, default_action_logger,
        default_credential_accessor, default_execution_emitter, default_resource_accessor,
        default_trigger_scheduler,
    },
    error::ActionError,
};

/// Stable execution context for StatelessAction, StatefulAction, and ResourceAction.
///
/// Concrete struct (not a trait). Capability fields (resources, credentials,
/// logger) can be swapped by the runtime/sandbox via the `with_*` builder
/// methods without changing this crate's API.
#[derive(Clone)]
pub struct ActionContext {
    /// Execution identity.
    pub execution_id: ExecutionId,
    /// Node identity within the workflow.
    pub node_key: NodeKey,
    /// Workflow identity.
    pub workflow_id: WorkflowId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
    /// Resource access capability.
    pub resources: Arc<dyn ResourceAccessor>,
    /// Credential access capability.
    pub credentials: Arc<dyn CredentialAccessor>,
    /// Action-scoped logger capability.
    pub logger: Arc<dyn Logger>,
}

impl ActionContext {
    /// Create a new action context.
    #[must_use]
    pub fn new(
        execution_id: ExecutionId,
        node_key: NodeKey,
        workflow_id: WorkflowId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            execution_id,
            node_key,
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
    pub fn with_logger(mut self, logger: Arc<dyn Logger>) -> Self {
        self.logger = logger;
        self
    }

    /// Acquire a resource by key through the configured accessor.
    ///
    /// The string `key` is parsed into a [`ResourceKey`]; invalid keys are
    /// surfaced as a fatal [`ActionError`].
    pub async fn resource(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        let rk = ResourceKey::new(key)
            .map_err(|e| ActionError::fatal(format!("invalid resource key `{key}`: {e}")))?;
        self.resources
            .acquire_any(&rk)
            .await
            .map_err(ActionError::from)
    }

    /// Check whether a resource exists.
    pub async fn has_resource(&self, key: &str) -> bool {
        let Ok(rk) = ResourceKey::new(key) else {
            return false;
        };
        self.resources.has(&rk)
    }

    /// Cancellation token; actions may check it before or during work.
    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    /// Execution identity.
    #[must_use]
    pub fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }

    /// Node identity within the workflow.
    #[must_use]
    pub fn node_key(&self) -> &NodeKey {
        &self.node_key
    }

    /// Workflow identity.
    #[must_use]
    pub fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }

    /// Action-scoped logger handle.
    #[must_use]
    pub fn logger(&self) -> &dyn Logger {
        &*self.logger
    }

    /// Build a [`nebula_core::Scope`] snapshot carrying the execution and
    /// node identity. Used by code that treats the action identity as a
    /// generic `Scope` (tracing, metrics, cache keys).
    #[must_use]
    pub fn scope(&self) -> nebula_core::Scope {
        nebula_core::Scope {
            execution_id: Some(self.execution_id),
            workflow_id: Some(self.workflow_id),
            node_key: Some(self.node_key.clone()),
            ..nebula_core::Scope::default()
        }
    }
}

impl fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionContext")
            .field("execution_id", &self.execution_id)
            .field("node_key", &self.node_key)
            .field("workflow_id", &self.workflow_id)
            .field("cancellation", &self.cancellation)
            .field("resources", &"<dyn ResourceAccessor>")
            .field("credentials", &"<dyn CredentialAccessor>")
            .field("logger", &"<dyn Logger>")
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
    pub trigger_id: NodeKey,
    /// Cancellation token.
    pub cancellation: CancellationToken,
    /// Trigger scheduling capability.
    pub scheduler: Arc<dyn TriggerScheduler>,
    /// Execution emission capability.
    pub emitter: Arc<dyn ExecutionEmitter>,
    /// Credential access capability.
    pub credentials: Arc<dyn CredentialAccessor>,
    /// Trigger-scoped logger capability.
    pub logger: Arc<dyn Logger>,
    /// Shared health state — adapter writes, runtime reads.
    pub health: Arc<TriggerHealth>,
    /// Webhook endpoint capability — `Some` only for webhook
    /// triggers, populated by the HTTP transport at activation time
    /// so `WebhookAction::on_activate` can read the public URL and
    /// register it with the external provider.
    ///
    /// `None` for poll triggers and any shape that does not own an
    /// HTTP endpoint.
    pub webhook: Option<Arc<dyn crate::webhook::WebhookEndpointProvider>>,
}

impl TriggerContext {
    /// Create a new trigger context.
    #[must_use]
    pub fn new(
        workflow_id: WorkflowId,
        trigger_id: NodeKey,
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
            health: Arc::new(TriggerHealth::new()),
            webhook: None,
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
    pub fn with_logger(mut self, logger: Arc<dyn Logger>) -> Self {
        self.logger = logger;
        self
    }

    /// Inject a shared health state (runtime keeps its own Arc clone).
    #[must_use]
    pub fn with_health(mut self, health: Arc<TriggerHealth>) -> Self {
        self.health = health;
        self
    }

    /// Inject a webhook endpoint provider.
    ///
    /// The HTTP transport layer calls this at trigger activation
    /// time, after it has generated the `(trigger_uuid, nonce)` path
    /// and built the full public URL. Called on a per-activation
    /// clone of the context template — do NOT call on a shared
    /// context.
    #[must_use]
    pub fn with_webhook_endpoint(
        mut self,
        provider: Arc<dyn crate::webhook::WebhookEndpointProvider>,
    ) -> Self {
        self.webhook = Some(provider);
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

    /// Cancellation token; trigger loops should honor it between cycles.
    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    /// Trigger-scoped logger handle.
    #[must_use]
    pub fn logger(&self) -> &dyn Logger {
        &*self.logger
    }

    /// Execution emitter handle — used by trigger loops to start workflow runs.
    #[must_use]
    pub fn emitter(&self) -> &dyn ExecutionEmitter {
        &*self.emitter
    }

    /// Shared trigger health atomics (runtime keeps an Arc clone).
    #[must_use]
    pub fn health(&self) -> &TriggerHealth {
        &self.health
    }

    /// Build a [`nebula_core::Scope`] snapshot carrying the trigger's
    /// workflow and node identity. Used by code that treats the trigger
    /// identity as a generic `Scope` (e.g. jitter-seed hashing).
    #[must_use]
    pub fn scope(&self) -> nebula_core::Scope {
        nebula_core::Scope {
            workflow_id: Some(self.workflow_id),
            node_key: Some(self.trigger_id.clone()),
            ..nebula_core::Scope::default()
        }
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
/// - New credential helpers added to this trait will **always** ship with a default implementation
///   built on top of [`Self::credentials`]. Downstream impls will not need to add new methods to
///   stay compatible with new crate versions.
/// - The required method [`Self::credentials`] will not change signature without a major version
///   bump of `nebula-action`.
/// - The trait itself may gain new default methods in minor versions; those default methods are
///   non-breaking additions for implementors.
pub trait CredentialContextExt {
    /// Access the underlying credential accessor.
    ///
    /// Implementors return a reference to their `credentials` field.
    fn credentials(&self) -> &Arc<dyn CredentialAccessor>;

    /// Retrieve a credential snapshot by id through the configured accessor.
    ///
    /// Constructs a [`CredentialKey`] from `id`, calls `resolve_any`, and
    /// downcasts the result to [`CredentialSnapshot`].
    fn credential_by_id(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<CredentialSnapshot, ActionError>> + Send
    where
        Self: Sync,
    {
        async move {
            let key = CredentialKey::new(id)
                .map_err(|e| ActionError::fatal(format!("invalid credential key `{id}`: {e}")))?;
            let boxed = self
                .credentials()
                .resolve_any(&key)
                .await
                .map_err(ActionError::from)?;
            boxed
                .downcast::<CredentialSnapshot>()
                .map(|b| *b)
                .map_err(|_| {
                    ActionError::fatal(format!(
                        "credential `{id}`: resolve_any returned unexpected type (expected CredentialSnapshot)"
                    ))
                })
        }
    }

    /// Retrieve a credential and project it to the concrete [`AuthScheme`] type.
    ///
    /// Fetches the snapshot and consumes it via
    /// [`CredentialSnapshot::into_project`], returning the concrete
    /// scheme type.
    ///
    /// # Errors
    ///
    /// - Returns [`ActionError::Fatal`] if the credential does not exist or the accessor is not
    ///   configured.
    /// - Returns [`ActionError::Fatal`] if the stored scheme type does not match `S` (scheme
    ///   mismatch).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let token: SecretToken = ctx.credential_typed::<SecretToken>("api_key").await?;
    /// ```
    fn credential_typed<S: AuthScheme>(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<S, ActionError>> + Send
    where
        Self: Sync,
    {
        async move {
            let key = CredentialKey::new(id)
                .map_err(|e| ActionError::fatal(format!("invalid credential key `{id}`: {e}")))?;
            let boxed = self
                .credentials()
                .resolve_any(&key)
                .await
                .map_err(ActionError::from)?;
            let snapshot = boxed
                .downcast::<CredentialSnapshot>()
                .map(|b| *b)
                .map_err(|_| {
                    ActionError::fatal(format!(
                        "credential `{id}`: resolve_any returned unexpected type"
                    ))
                })?;
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
    fn credential<S>(&self) -> impl Future<Output = Result<CredentialGuard<S>, ActionError>> + Send
    where
        S: AuthScheme + zeroize::Zeroize,
        Self: Sync,
    {
        async move {
            // Type-based credential access: construct a key from the type name.
            // The core trait uses CredentialKey, so we derive a key from the
            // type name in snake_case-ish form. For now, use a synthetic key
            // based on type_name. Callers should prefer credential_typed() with
            // an explicit key.
            let type_name = std::any::type_name::<S>();
            // Use a synthetic key approach: try to create a CredentialKey from
            // the short type name. If that fails, use a fallback.
            let short_name = type_name.rsplit("::").next().unwrap_or(type_name);
            let key_str = short_name.to_lowercase();
            let key = CredentialKey::new(&key_str).map_err(|_| {
                ActionError::fatal(format!(
                    "type-based credential access not supported for `{type_name}` (could not derive valid key)"
                ))
            })?;
            let boxed = self
                .credentials()
                .resolve_any(&key)
                .await
                .map_err(ActionError::from)?;
            let snapshot = boxed
                .downcast::<CredentialSnapshot>()
                .map(|b| *b)
                .map_err(|_| {
                    ActionError::fatal(format!(
                        "credential type mismatch for `{type_name}`: resolve_any returned unexpected type"
                    ))
                })?;
            let scheme = snapshot.into_project::<S>().map_err(|e| {
                ActionError::fatal(format!("credential type mismatch for `{type_name}`: {e}"))
            })?;
            Ok(CredentialGuard::new(scheme))
        }
    }

    /// Check whether a credential exists by id.
    fn has_credential_id(&self, id: &str) -> impl Future<Output = bool> + Send
    where
        Self: Sync,
    {
        async move {
            let Ok(key) = CredentialKey::new(id) else {
                return false;
            };
            self.credentials().has(&key)
        }
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
            .field("logger", &"<dyn Logger>")
            .field("health", &self.health)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_core::{
        CoreError, CredentialKey,
        accessor::{LogLevel, Logger},
        node_key,
    };
    use nebula_credential::{
        CredentialRecord, CredentialSnapshot, SecretString, SecretToken, scheme::ConnectionUri,
    };

    use super::*;
    use crate::capability::{ExecutionEmitter, TriggerScheduler};

    /// Type alias for dyn-safe async return (for test impls).
    type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[tokio::test]
    async fn action_context_defaults_to_noop_capabilities() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
            CancellationToken::new(),
        );

        assert!(!ctx.has_resource("missing").await);
        assert!(!ctx.has_credential_id("missing").await);
        assert!(ctx.resource("missing").await.is_err());
        assert!(ctx.credential_by_id("missing").await.is_err());
    }

    struct TestScheduler;

    #[async_trait::async_trait]
    impl TriggerScheduler for TestScheduler {
        async fn schedule_after(&self, _delay: Duration) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestEmitter;

    #[async_trait::async_trait]
    impl ExecutionEmitter for TestEmitter {
        async fn emit(&self, _input: serde_json::Value) -> Result<ExecutionId, ActionError> {
            Ok(ExecutionId::new())
        }
    }

    struct TestCredentialAccessor;

    impl CredentialAccessor for TestCredentialAccessor {
        fn has(&self, _key: &CredentialKey) -> bool {
            true
        }

        fn resolve_any(
            &self,
            _key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
            Box::pin(async {
                let snapshot = CredentialSnapshot::new(
                    "api_key",
                    CredentialRecord::new(),
                    SecretToken::new(SecretString::new("test-token")),
                );
                Ok(Box::new(snapshot) as Box<dyn Any + Send + Sync>)
            })
        }

        fn try_resolve_any(
            &self,
            _key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
            Box::pin(async move {
                let snapshot = CredentialSnapshot::new(
                    "api_key",
                    CredentialRecord::new(),
                    SecretToken::new(SecretString::new("test-token")),
                );
                Ok(Some(Box::new(snapshot) as Box<dyn Any + Send + Sync>))
            })
        }
    }

    struct TestLogger;

    impl Logger for TestLogger {
        fn log(&self, _level: LogLevel, _message: &str) {}
        fn log_with_fields(&self, _level: LogLevel, _message: &str, _fields: &[(&str, &str)]) {}
    }

    #[tokio::test]
    async fn trigger_context_with_capabilities_can_schedule_and_emit() {
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            CancellationToken::new(),
        )
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
            node_key!("test"),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor));

        let token: SecretToken = ctx
            .credential_typed::<SecretToken>("api_key")
            .await
            .unwrap();
        assert_eq!(token.token().expose_secret(), "test-token");
    }

    #[tokio::test]
    async fn action_context_credential_typed_mismatch_returns_fatal() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            node_key!("test"),
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
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor));

        let token: SecretToken = ctx
            .credential_typed::<SecretToken>("api_key")
            .await
            .unwrap();
        assert_eq!(token.token().expose_secret(), "test-token");
    }

    #[tokio::test]
    async fn trigger_context_credential_typed_mismatch_returns_fatal() {
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            CancellationToken::new(),
        )
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

    impl AuthScheme for ZeroizableToken {
        fn pattern() -> nebula_credential::AuthPattern {
            nebula_credential::AuthPattern::SecretToken
        }
    }

    impl zeroize::Zeroize for ZeroizableToken {
        fn zeroize(&mut self) {
            self.value.zeroize();
        }
    }

    /// Accessor that supports type-based credential access.
    ///
    /// Maps credential keys to snapshots — the `credential<S>()` method
    /// derives a key from the type name (lowercased).
    struct TypedCredentialAccessor;

    impl CredentialAccessor for TypedCredentialAccessor {
        fn has(&self, key: &CredentialKey) -> bool {
            key.as_str() == "zeroizabletoken"
        }

        fn resolve_any(
            &self,
            key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
            let key_str = key.as_str().to_owned();
            Box::pin(async move {
                if key_str == "zeroizabletoken" {
                    let snapshot = CredentialSnapshot::new(
                        "typed",
                        CredentialRecord::new(),
                        ZeroizableToken {
                            value: "secret-42".to_owned(),
                        },
                    );
                    Ok(Box::new(snapshot) as Box<dyn Any + Send + Sync>)
                } else {
                    Err(CoreError::CredentialNotFound { key: key_str })
                }
            })
        }

        fn try_resolve_any(
            &self,
            key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
            let key_str = key.as_str().to_owned();
            Box::pin(async move {
                if key_str == "zeroizabletoken" {
                    let snapshot = CredentialSnapshot::new(
                        "typed",
                        CredentialRecord::new(),
                        ZeroizableToken {
                            value: "secret-42".to_owned(),
                        },
                    );
                    Ok(Some(Box::new(snapshot) as Box<dyn Any + Send + Sync>))
                } else {
                    Ok(None)
                }
            })
        }
    }

    #[tokio::test]
    async fn action_context_credential_returns_guard() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn trigger_context_credential_returns_guard() {
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn credential_noop_accessor_returns_error() {
        let ctx = ActionContext::new(
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
            CancellationToken::new(),
        );
        let result = ctx.credential::<ZeroizableToken>().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
    }
}
