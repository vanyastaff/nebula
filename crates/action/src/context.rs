//! Execution context contracts and runtime implementations.
//!
//! Spec 23/27 make [`ActionContext`] and [`TriggerContext`] **umbrella marker
//! traits**: any type satisfying [`nebula_core::Context`] plus the capability
//! supertraits is an action/trigger context. The concrete runtime types live
//! in this module as [`ActionRuntimeContext`] / [`TriggerRuntimeContext`] —
//! they embed [`nebula_core::BaseContext`] for identity and compose
//! capability accessors as `Arc<dyn ...>` fields.
//!
//! Action authors never write `ActionRuntimeContext` in their code — they
//! write `fn execute(ctx: &(impl ActionContext + ?Sized))` and receive any
//! type the runtime chooses to supply (engine runtime, test harness,
//! sandbox wrapper, ...).

use std::{any::Any, fmt, future::Future, sync::Arc};

use nebula_core::{
    BaseContext, CredentialKey, NodeKey, ResourceKey,
    accessor::{Clock, CredentialAccessor, EventEmitter, Logger, MetricsEmitter, ResourceAccessor},
    context::{
        Context as CoreContext, HasCredentials, HasEventBus, HasLogger, HasMetrics, HasResources,
    },
    id::{ExecutionId, WorkflowId},
    obs::{SpanId, TraceId},
    scope::{Principal, Scope},
};
use nebula_credential::{AuthScheme, CredentialGuard, CredentialSnapshot};
use tokio_util::sync::CancellationToken;

use crate::{
    capability::{
        ExecutionEmitter, TriggerHealth, TriggerScheduler, default_action_logger,
        default_credential_accessor, default_event_emitter, default_execution_emitter,
        default_metrics_emitter, default_resource_accessor, default_trigger_scheduler,
    },
    error::ActionError,
};

// ── Action-specific capability traits ──────────────────────────────────────

/// Capability: node identity within a workflow graph.
///
/// Action-specific — triggers live outside an execution and use
/// [`HasTriggerScheduling`] instead. Returns already-typed IDs (not `Option`)
/// because an action that is executing always has all three.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not expose action node identity",
    note = "provide execution_id / node_key / workflow_id via HasNodeIdentity — the runtime \
            populates these per dispatch; tests use TestContextBuilder"
)]
pub trait HasNodeIdentity: CoreContext {
    /// The execution this action is running in.
    fn execution_id(&self) -> ExecutionId;

    /// The node this action corresponds to in the workflow graph.
    fn node_key(&self) -> &NodeKey;

    /// The workflow definition that owns the node.
    fn workflow_id(&self) -> WorkflowId;
}

/// Capability: trigger-lifecycle wiring (scheduling + execution emission +
/// health atomics). Present only on trigger contexts.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not expose trigger-scheduling capabilities",
    note = "provide scheduler / emitter / health via HasTriggerScheduling — the runtime wires \
            these at trigger activation time; tests use TestContextBuilder::build_trigger"
)]
pub trait HasTriggerScheduling: CoreContext {
    /// Scheduler handle — ask the runtime to invoke the trigger after a delay.
    fn scheduler(&self) -> &dyn TriggerScheduler;

    /// Emitter handle — start a new workflow execution from the trigger.
    fn emitter(&self) -> &dyn ExecutionEmitter;

    /// Shared health atomics (adapter writes, runtime reads for dashboards).
    fn health(&self) -> &TriggerHealth;

    /// Webhook endpoint provider wired by the HTTP transport at trigger
    /// activation time.
    ///
    /// Returns `None` for non-webhook triggers (poll, cron, ...) and for
    /// webhook triggers that have not yet had their endpoint resolved —
    /// webhook action authors can treat `None` as a hard fatal, since the
    /// transport is responsible for populating it before `on_activate`
    /// runs. Built into the scheduling trait so a single `impl
    /// HasTriggerScheduling` covers every trigger shape without a
    /// secondary capability trait.
    fn webhook_endpoint(&self) -> Option<&Arc<dyn crate::webhook::WebhookEndpointProvider>> {
        None
    }
}

// ── Umbrella marker traits ─────────────────────────────────────────────────

/// Umbrella context trait for [`StatelessAction`](crate::stateless::StatelessAction),
/// [`StatefulAction`](crate::stateful::StatefulAction),
/// [`ResourceAction`](crate::resource::ResourceAction), and
/// [`ControlAction`](crate::control::ControlAction).
///
/// Any type implementing the core [`Context`](nebula_core::Context) trait
/// plus every listed capability IS an `ActionContext` — the blanket impl
/// means nothing in nebula-action needs to name concrete runtime types.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is missing capabilities required by ActionContext",
    note = "ActionContext requires: Context + HasResources + HasCredentials + HasLogger + \
            HasMetrics + HasEventBus + HasNodeIdentity (see spec 23)"
)]
pub trait ActionContext:
    CoreContext + HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasNodeIdentity
{
}

impl<T> ActionContext for T where
    T: ?Sized
        + CoreContext
        + HasResources
        + HasCredentials
        + HasLogger
        + HasMetrics
        + HasEventBus
        + HasNodeIdentity
{
}

/// Umbrella context trait for [`TriggerAction`](crate::trigger::TriggerAction)
/// and its specializations ([`WebhookAction`](crate::webhook::WebhookAction),
/// [`PollAction`](crate::poll::PollAction)).
#[diagnostic::on_unimplemented(
    message = "`{Self}` is missing capabilities required by TriggerContext",
    note = "TriggerContext requires: Context + HasResources + HasCredentials + HasLogger + \
            HasMetrics + HasEventBus + HasTriggerScheduling (see spec 23)"
)]
pub trait TriggerContext:
    CoreContext
    + HasResources
    + HasCredentials
    + HasLogger
    + HasMetrics
    + HasEventBus
    + HasTriggerScheduling
{
}

impl<T> TriggerContext for T where
    T: ?Sized
        + CoreContext
        + HasResources
        + HasCredentials
        + HasLogger
        + HasMetrics
        + HasEventBus
        + HasTriggerScheduling
{
}

// ── Concrete runtime types ─────────────────────────────────────────────────

/// Concrete context supplied to actions at dispatch time.
///
/// Implements [`ActionContext`] via the blanket impl. The runtime constructs
/// one per dispatch, wiring real resource/credential/logger/metrics/eventbus
/// accessors; tests go through [`TestContextBuilder`](crate::testing::TestContextBuilder).
///
/// This type lives in nebula-action today; spec 28 relocates it to
/// `nebula-engine::context::ActionRuntimeContext`. Call sites that use the
/// [`ActionContext`] trait will migrate without change.
#[derive(Clone)]
pub struct ActionRuntimeContext {
    base: Arc<BaseContext>,
    execution_id: ExecutionId,
    node_key: NodeKey,
    workflow_id: WorkflowId,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn Logger>,
    metrics: Arc<dyn MetricsEmitter>,
    eventbus: Arc<dyn EventEmitter>,
}

impl ActionRuntimeContext {
    /// Build a runtime context from a shared [`BaseContext`] plus action identity.
    #[must_use]
    pub fn new(
        base: Arc<BaseContext>,
        execution_id: ExecutionId,
        node_key: NodeKey,
        workflow_id: WorkflowId,
    ) -> Self {
        Self {
            base,
            execution_id,
            node_key,
            workflow_id,
            resources: default_resource_accessor(),
            credentials: default_credential_accessor(),
            logger: default_action_logger(),
            metrics: default_metrics_emitter(),
            eventbus: default_event_emitter(),
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

    /// Inject a logger capability.
    #[must_use]
    pub fn with_logger(mut self, logger: Arc<dyn Logger>) -> Self {
        self.logger = logger;
        self
    }

    /// Inject a metrics emitter capability.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<dyn MetricsEmitter>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Inject an event emitter capability.
    #[must_use]
    pub fn with_eventbus(mut self, eventbus: Arc<dyn EventEmitter>) -> Self {
        self.eventbus = eventbus;
        self
    }

    /// Acquire a resource by string key through the configured accessor.
    ///
    /// Invalid keys surface as fatal [`ActionError`].
    pub async fn resource(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        let rk = ResourceKey::new(key)
            .map_err(|e| ActionError::fatal(format!("invalid resource key `{key}`: {e}")))?;
        self.resources
            .acquire_any(&rk)
            .await
            .map_err(ActionError::from)
    }

    /// Check whether a resource exists under the given string key.
    pub async fn has_resource(&self, key: &str) -> bool {
        let Ok(rk) = ResourceKey::new(key) else {
            return false;
        };
        self.resources.has(&rk)
    }
}

impl CoreContext for ActionRuntimeContext {
    fn scope(&self) -> &Scope {
        self.base.scope()
    }

    fn principal(&self) -> &Principal {
        self.base.principal()
    }

    fn cancellation(&self) -> &CancellationToken {
        self.base.cancellation()
    }

    fn clock(&self) -> &dyn Clock {
        self.base.clock()
    }

    fn trace_id(&self) -> Option<TraceId> {
        self.base.trace_id()
    }

    fn span_id(&self) -> Option<SpanId> {
        self.base.span_id()
    }
}

impl HasResources for ActionRuntimeContext {
    fn resources(&self) -> &dyn ResourceAccessor {
        &*self.resources
    }
}

impl HasCredentials for ActionRuntimeContext {
    fn credentials(&self) -> &dyn CredentialAccessor {
        &*self.credentials
    }
}

impl HasLogger for ActionRuntimeContext {
    fn logger(&self) -> &dyn Logger {
        &*self.logger
    }
}

impl HasMetrics for ActionRuntimeContext {
    fn metrics(&self) -> &dyn MetricsEmitter {
        &*self.metrics
    }
}

impl HasEventBus for ActionRuntimeContext {
    fn eventbus(&self) -> &dyn EventEmitter {
        &*self.eventbus
    }
}

impl HasNodeIdentity for ActionRuntimeContext {
    fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }

    fn node_key(&self) -> &NodeKey {
        &self.node_key
    }

    fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
}

impl fmt::Debug for ActionRuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRuntimeContext")
            .field("execution_id", &self.execution_id)
            .field("node_key", &self.node_key)
            .field("workflow_id", &self.workflow_id)
            .field("resources", &"<dyn ResourceAccessor>")
            .field("credentials", &"<dyn CredentialAccessor>")
            .field("logger", &"<dyn Logger>")
            .field("metrics", &"<dyn MetricsEmitter>")
            .field("eventbus", &"<dyn EventEmitter>")
            .finish()
    }
}

/// Concrete context supplied to triggers at activation / loop time.
///
/// Implements [`TriggerContext`] via the blanket impl. Same migration note
/// as [`ActionRuntimeContext`] — relocates to `nebula-engine` per spec 28.
#[derive(Clone)]
pub struct TriggerRuntimeContext {
    base: Arc<BaseContext>,
    workflow_id: WorkflowId,
    trigger_id: NodeKey,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn Logger>,
    metrics: Arc<dyn MetricsEmitter>,
    eventbus: Arc<dyn EventEmitter>,
    scheduler: Arc<dyn TriggerScheduler>,
    emitter: Arc<dyn ExecutionEmitter>,
    health: Arc<TriggerHealth>,
    /// Webhook endpoint capability — populated by the HTTP transport at
    /// activation time so `WebhookAction::on_activate` can read the public
    /// URL and register it with the external provider. `None` for poll
    /// triggers and any shape that does not own an HTTP endpoint.
    pub webhook: Option<Arc<dyn crate::webhook::WebhookEndpointProvider>>,
}

impl TriggerRuntimeContext {
    /// Build a trigger runtime context from a shared [`BaseContext`] plus
    /// trigger identity.
    #[must_use]
    pub fn new(base: Arc<BaseContext>, workflow_id: WorkflowId, trigger_id: NodeKey) -> Self {
        Self {
            base,
            workflow_id,
            trigger_id,
            resources: default_resource_accessor(),
            credentials: default_credential_accessor(),
            logger: default_action_logger(),
            metrics: default_metrics_emitter(),
            eventbus: default_event_emitter(),
            scheduler: default_trigger_scheduler(),
            emitter: default_execution_emitter(),
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

    /// Inject a logger capability.
    #[must_use]
    pub fn with_logger(mut self, logger: Arc<dyn Logger>) -> Self {
        self.logger = logger;
        self
    }

    /// Inject a metrics emitter capability.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<dyn MetricsEmitter>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Inject an event emitter capability.
    #[must_use]
    pub fn with_eventbus(mut self, eventbus: Arc<dyn EventEmitter>) -> Self {
        self.eventbus = eventbus;
        self
    }

    /// Inject a shared health state (runtime keeps its own Arc clone).
    #[must_use]
    pub fn with_health(mut self, health: Arc<TriggerHealth>) -> Self {
        self.health = health;
        self
    }

    /// Inject a webhook endpoint provider (webhook triggers only).
    ///
    /// The HTTP transport layer calls this at trigger activation time,
    /// after it has generated the `(trigger_uuid, nonce)` path and built
    /// the full public URL.
    #[must_use]
    pub fn with_webhook_endpoint(
        mut self,
        provider: Arc<dyn crate::webhook::WebhookEndpointProvider>,
    ) -> Self {
        self.webhook = Some(provider);
        self
    }

    /// Schedule the next trigger run after `delay`.
    pub async fn schedule_after(&self, delay: std::time::Duration) -> Result<(), ActionError> {
        self.scheduler.schedule_after(delay).await
    }

    /// Emit a new execution request for the trigger's workflow.
    pub async fn emit_execution(
        &self,
        input: serde_json::Value,
    ) -> Result<ExecutionId, ActionError> {
        self.emitter.emit(input).await
    }

    /// Trigger (node) identity.
    #[must_use]
    pub fn trigger_id(&self) -> &NodeKey {
        &self.trigger_id
    }

    /// Workflow identity (triggers have no execution yet).
    #[must_use]
    pub fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
}

impl CoreContext for TriggerRuntimeContext {
    fn scope(&self) -> &Scope {
        self.base.scope()
    }

    fn principal(&self) -> &Principal {
        self.base.principal()
    }

    fn cancellation(&self) -> &CancellationToken {
        self.base.cancellation()
    }

    fn clock(&self) -> &dyn Clock {
        self.base.clock()
    }

    fn trace_id(&self) -> Option<TraceId> {
        self.base.trace_id()
    }

    fn span_id(&self) -> Option<SpanId> {
        self.base.span_id()
    }
}

impl HasResources for TriggerRuntimeContext {
    fn resources(&self) -> &dyn ResourceAccessor {
        &*self.resources
    }
}

impl HasCredentials for TriggerRuntimeContext {
    fn credentials(&self) -> &dyn CredentialAccessor {
        &*self.credentials
    }
}

impl HasLogger for TriggerRuntimeContext {
    fn logger(&self) -> &dyn Logger {
        &*self.logger
    }
}

impl HasMetrics for TriggerRuntimeContext {
    fn metrics(&self) -> &dyn MetricsEmitter {
        &*self.metrics
    }
}

impl HasEventBus for TriggerRuntimeContext {
    fn eventbus(&self) -> &dyn EventEmitter {
        &*self.eventbus
    }
}

impl HasTriggerScheduling for TriggerRuntimeContext {
    fn scheduler(&self) -> &dyn TriggerScheduler {
        &*self.scheduler
    }

    fn emitter(&self) -> &dyn ExecutionEmitter {
        &*self.emitter
    }

    fn health(&self) -> &TriggerHealth {
        &self.health
    }

    fn webhook_endpoint(&self) -> Option<&Arc<dyn crate::webhook::WebhookEndpointProvider>> {
        self.webhook.as_ref()
    }
}

impl fmt::Debug for TriggerRuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerRuntimeContext")
            .field("workflow_id", &self.workflow_id)
            .field("trigger_id", &self.trigger_id)
            .field("scheduler", &"<dyn TriggerScheduler>")
            .field("emitter", &"<dyn ExecutionEmitter>")
            .field("credentials", &"<dyn CredentialAccessor>")
            .field("logger", &"<dyn Logger>")
            .field("metrics", &"<dyn MetricsEmitter>")
            .field("eventbus", &"<dyn EventEmitter>")
            .field("health", &self.health)
            .finish()
    }
}

// ── CredentialContextExt ───────────────────────────────────────────────────

/// Ergonomic credential-access helpers for any context that carries a
/// [`HasCredentials`] capability.
///
/// The methods here used to be typed helpers on the concrete contexts —
/// since the contexts became marker traits, these moved into a blanket
/// extension trait keyed on [`HasCredentials`]. Callers bring the trait
/// into scope via `use nebula_action::CredentialContextExt;` or via the
/// prelude.
pub trait CredentialContextExt: HasCredentials {
    /// Retrieve a credential snapshot by id through the configured accessor.
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

    /// Retrieve a typed credential by [`AuthScheme`] type. Returns a
    /// zeroizing [`CredentialGuard<S>`].
    fn credential<S>(&self) -> impl Future<Output = Result<CredentialGuard<S>, ActionError>> + Send
    where
        S: AuthScheme + zeroize::Zeroize,
        Self: Sync,
    {
        async move {
            let type_name = std::any::type_name::<S>();
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

/// Blanket impl — any type carrying `HasCredentials` gets the helpers.
impl<T: ?Sized + HasCredentials> CredentialContextExt for T {}

#[cfg(test)]
mod tests {
    use std::{any::Any, time::Duration};

    use nebula_core::{
        BaseContext, CoreError, CredentialKey,
        accessor::{CredentialAccessor, LogLevel, Logger},
        id::{ExecutionId, WorkflowId},
        node_key,
    };
    use nebula_credential::{
        CredentialRecord, CredentialSnapshot, SecretString, SecretToken, scheme::ConnectionUri,
    };

    use super::*;

    /// Type alias for dyn-safe async return (for test impls).
    type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    fn make_base() -> Arc<BaseContext> {
        Arc::new(BaseContext::builder().build())
    }

    #[tokio::test]
    async fn action_context_defaults_to_noop_capabilities() {
        let ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        );

        assert!(!ctx.has_resource("missing").await);
        assert!(!ctx.has_credential_id("missing").await);
        assert!(ctx.resource("missing").await.is_err());
        assert!(ctx.credential_by_id("missing").await.is_err());
    }

    struct TestScheduler;

    impl TriggerScheduler for TestScheduler {
        fn schedule_after(&self, _delay: Duration) -> BoxFuture<'_, Result<(), ActionError>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct TestEmitter;

    impl ExecutionEmitter for TestEmitter {
        fn emit(
            &self,
            _input: serde_json::Value,
        ) -> BoxFuture<'_, Result<ExecutionId, ActionError>> {
            Box::pin(async { Ok(ExecutionId::new()) })
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
        let ctx = TriggerRuntimeContext::new(make_base(), WorkflowId::new(), node_key!("test"))
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
        let ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
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
        let ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor));

        let result = ctx.credential_typed::<ConnectionUri>("api_key").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
        assert!(err.to_string().contains("scheme mismatch"));
    }

    // ── Type-based credential access tests ──────────────────────────────

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
        let ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        )
        .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn trigger_context_credential_returns_guard() {
        let ctx = TriggerRuntimeContext::new(make_base(), WorkflowId::new(), node_key!("test"))
            .with_credentials(Arc::new(TypedCredentialAccessor));

        let guard = ctx.credential::<ZeroizableToken>().await.unwrap();
        assert_eq!(guard.value, "secret-42");
    }

    #[tokio::test]
    async fn credential_noop_accessor_returns_error() {
        let ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        );
        let result = ctx.credential::<ZeroizableToken>().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_fatal());
    }

    #[test]
    fn blanket_impl_compiles_for_runtime_contexts() {
        // Compile-time check that the blanket impls work — any type
        // implementing all the capabilities satisfies the umbrella trait.
        fn assert_action<T: ActionContext>(_: &T) {}
        fn assert_trigger<T: TriggerContext>(_: &T) {}

        let action_ctx = ActionRuntimeContext::new(
            make_base(),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        );
        assert_action(&action_ctx);

        let trigger_ctx =
            TriggerRuntimeContext::new(make_base(), WorkflowId::new(), node_key!("test"));
        assert_trigger(&trigger_ctx);
    }
}
