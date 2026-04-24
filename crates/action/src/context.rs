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

use std::{any::Any, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{
    AttemptId, BaseContext, CredentialKey, NodeKey, ResourceKey,
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
/// [`HasTriggerScheduling`] instead. The concrete identity surface is the
/// pair `(node_key, attempt_id)`; execution id and workflow id live on the
/// context [`Scope`] and are reached via `Context::scope()`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not expose action node identity",
    note = "provide node_key / attempt_id via HasNodeIdentity — the runtime \
            populates these per dispatch; tests use TestContextBuilder"
)]
pub trait HasNodeIdentity: CoreContext {
    /// Node key currently executing.
    fn node_key(&self) -> &NodeKey;
    /// Current execution attempt identifier.
    fn attempt_id(&self) -> &AttemptId;
}

/// Capability: trigger scheduling + execution emission.
pub trait HasTriggerScheduling: CoreContext {
    /// Scheduler used by triggers for delayed re-runs.
    fn scheduler(&self) -> &dyn TriggerScheduler;
    /// Emitter used to start workflow executions from trigger events.
    fn emitter(&self) -> &dyn ExecutionEmitter;
    /// Shared trigger health state.
    fn health(&self) -> &TriggerHealth;
}

/// Optional webhook endpoint capability for trigger contexts.
pub trait HasWebhookEndpoint: CoreContext {
    /// Endpoint provider when present (webhook triggers), otherwise `None`.
    fn webhook_endpoint(&self) -> Option<&Arc<dyn crate::webhook::WebhookEndpointProvider>>;
}

/// Umbrella trait for execution-time action contexts.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement ActionContext",
    note = "ActionContext requires core::Context + resources + credentials + logger + metrics + event bus + node identity"
)]
pub trait ActionContext:
    CoreContext
    + HasResources
    + HasCredentials
    + HasLogger
    + HasMetrics
    + HasEventBus
    + HasNodeIdentity
{
}

impl<T> ActionContext for T where
    T: CoreContext
        + HasResources
        + HasCredentials
        + HasLogger
        + HasMetrics
        + HasEventBus
        + HasNodeIdentity
        + ?Sized
{
}

/// Umbrella trait for trigger-dispatch contexts.
///
/// `HasWebhookEndpoint` is a supertrait so webhook-specific trigger actions
/// can pull the endpoint URL off any `TriggerContext` without having to
/// thread an extra bound through every `on_activate` impl. Non-webhook
/// trigger shapes return `None` from `webhook_endpoint()`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement TriggerContext",
    note = "TriggerContext requires core::Context + credentials + logger + metrics + event bus + trigger scheduling + webhook endpoint (may be None)"
)]
pub trait TriggerContext:
    CoreContext
    + HasCredentials
    + HasLogger
    + HasMetrics
    + HasEventBus
    + HasTriggerScheduling
    + HasWebhookEndpoint
{
}

impl<T> TriggerContext for T where
    T: CoreContext
        + HasCredentials
        + HasLogger
        + HasMetrics
        + HasEventBus
        + HasTriggerScheduling
        + HasWebhookEndpoint
        + ?Sized
{
}

// ── Concrete runtime contexts ──────────────────────────────────────────────

/// Concrete context supplied to actions at dispatch time.
///
/// Implements [`ActionContext`] via the blanket impl. The runtime constructs
/// one per dispatch, wiring real resource/credential/logger/metrics/eventbus
/// accessors; tests go through [`TestContextBuilder`](crate::testing::TestContextBuilder).
///
/// Lives in `nebula-action` as the canonical runtime context. Spec 28
/// schedules a physical relocation to `nebula-engine::context` once the
/// engine surface is stable — the umbrella [`ActionContext`] trait makes
/// that move non-breaking for action authors.
#[derive(Clone)]
pub struct ActionRuntimeContext {
    base: Arc<BaseContext>,
    scope: Scope,
    node_key: NodeKey,
    attempt_id: AttemptId,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn Logger>,
    metrics: Arc<dyn MetricsEmitter>,
    eventbus: Arc<dyn EventEmitter>,
}

impl ActionRuntimeContext {
    /// Build a runtime context from a shared [`BaseContext`] plus action identity.
    ///
    /// The incoming `base` provides cancellation/clock/observability; the
    /// identity fields (`execution_id`, `node_key`, `workflow_id`,
    /// `attempt_id`) are written into the returned context's [`Scope`] so
    /// `Context::scope()` exposes the complete identity tuple.
    #[must_use]
    pub fn new(
        base: Arc<BaseContext>,
        execution_id: ExecutionId,
        node_key: NodeKey,
        workflow_id: WorkflowId,
    ) -> Self {
        let attempt_id = AttemptId::new();
        let scope = Scope {
            execution_id: Some(execution_id),
            node_key: Some(node_key.clone()),
            workflow_id: Some(workflow_id),
            attempt_id: Some(attempt_id),
            ..base.scope().clone()
        };
        Self {
            base,
            scope,
            node_key,
            attempt_id,
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
        &self.scope
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
    fn node_key(&self) -> &NodeKey {
        &self.node_key
    }

    fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }
}

impl fmt::Debug for ActionRuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRuntimeContext")
            .field("node_key", &self.node_key)
            .field("attempt_id", &self.attempt_id)
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
    scope: Scope,
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
        let scope = Scope {
            workflow_id: Some(workflow_id),
            node_key: Some(trigger_id.clone()),
            ..base.scope().clone()
        };
        Self {
            base,
            scope,
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
}

impl CoreContext for TriggerRuntimeContext {
    fn scope(&self) -> &Scope {
        &self.scope
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
}

impl HasWebhookEndpoint for TriggerRuntimeContext {
    fn webhook_endpoint(&self) -> Option<&Arc<dyn crate::webhook::WebhookEndpointProvider>> {
        self.webhook.as_ref()
    }
}

impl fmt::Debug for TriggerRuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerRuntimeContext")
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
    ) -> Pin<Box<dyn Future<Output = Result<CredentialSnapshot, ActionError>> + Send + '_>>
    where
        Self: Sync,
    {
        let id = id.to_owned();
        Box::pin(async move {
            let key = CredentialKey::new(&id)
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
        })
    }

    /// Retrieve a credential and project it to the concrete [`AuthScheme`] type.
    fn credential_typed<'a, S: AuthScheme + 'a>(
        &'a self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<S, ActionError>> + Send + 'a>>
    where
        Self: Sync,
    {
        let id = id.to_owned();
        Box::pin(async move {
            let key = CredentialKey::new(&id)
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
        })
    }

    /// Retrieve a typed credential by [`AuthScheme`] type. Returns a
    /// zeroizing [`CredentialGuard<S>`].
    fn credential<'a, S>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<CredentialGuard<S>, ActionError>> + Send + 'a>>
    where
        S: AuthScheme + zeroize::Zeroize + 'a,
        Self: Sync,
    {
        Box::pin(async move {
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
        })
    }

    /// Check whether a credential exists by id.
    fn has_credential_id<'a>(
        &'a self,
        id: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>
    where
        Self: Sync,
    {
        Box::pin(async move {
            let Ok(key) = CredentialKey::new(id) else {
                return false;
            };
            self.credentials().has(&key)
        })
    }
}

/// Blanket impl — any type carrying `HasCredentials` gets the helpers.
impl<T: ?Sized + HasCredentials> CredentialContextExt for T {}

