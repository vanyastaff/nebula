//! Application State
//!
//! Shared state for all handlers via Arc.
//! Contains only ports (traits) — independent of concrete implementations.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, WorkspaceId, WorkspaceRole, id::ExecutionId, scope::Principal};
use nebula_credential::PendingToken;
use nebula_engine::ActionRegistry;
use nebula_metrics::MetricsRegistry;
use nebula_plugin::PluginRegistry;
use nebula_storage::{
    ExecutionRepo, WorkflowRepo,
    credential::{InMemoryPendingStore, InMemoryStore},
    repos::{ControlQueueRepo, WebhookActivationRepo},
};
use nebula_storage_port::store::{ControlQueue, ExecutionStore, WorkflowVersionStore};
use tokio::sync::RwLock;

use crate::{
    auth::AuthBackend, config::JwtSecret, errors::ApiError, middleware::IdempotencyStore,
    services::webhook::WebhookTransport,
};

// ── Port traits ──────────────────────────────────────────────────────────────

/// Resolves org identifiers (slug or ULID) to [`OrgId`].
#[async_trait]
pub trait OrgResolver: Send + Sync {
    /// Look up an org by its human-readable slug.
    async fn resolve_by_slug(&self, slug: &str) -> Result<OrgId, ApiError>;
}

/// Resolves workspace identifiers (slug or ULID) within an org to [`WorkspaceId`].
#[async_trait]
pub trait WorkspaceResolver: Send + Sync {
    /// Look up a workspace by its slug within the given org.
    async fn resolve_by_slug(&self, org_id: OrgId, slug: &str) -> Result<WorkspaceId, ApiError>;
}

/// Loads membership roles for RBAC middleware.
#[async_trait]
pub trait MembershipStore: Send + Sync {
    /// Return the caller's org-level role, if they are an org member.
    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError>;

    /// Return the caller's workspace-level role, if they are a workspace member.
    async fn get_workspace_role(
        &self,
        workspace_id: WorkspaceId,
        principal: &Principal,
    ) -> Result<Option<WorkspaceRole>, ApiError>;
}

/// Application state passed through `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    /// JWT secret used to validate Bearer tokens.
    ///
    /// Wrapped in [`JwtSecret`] so construction enforces a
    /// 32-byte minimum length and rejects the well-known development
    /// placeholder. The middleware calls `as_bytes()` — same call
    /// shape as the previous `Arc<str>`.
    pub jwt_secret: JwtSecret,

    /// Static API keys accepted via `X-API-Key` header.
    ///
    /// Each key must use the `nbl_sk_` prefix. Compared in constant time.
    /// An empty `Vec` means API key auth is disabled for this route group.
    pub api_keys: Arc<Vec<String>>,

    /// Workflow Repository (port/trait)
    pub workflow_repo: Arc<dyn WorkflowRepo>,

    /// Execution Repository (port/trait)
    pub execution_repo: Arc<dyn ExecutionRepo>,

    /// Execution control queue (durable outbox — canon §12.2).
    ///
    /// Every cancel signal is enqueued here in the same logical operation as
    /// the corresponding state transition. The engine dispatcher drains this
    /// queue to deliver signals to running executions.
    pub control_queue_repo: Arc<dyn ControlQueueRepo>,

    /// Optional metrics registry for Prometheus export.
    /// When `None`, the `GET /metrics` endpoint returns 503.
    pub metrics_registry: Option<Arc<MetricsRegistry>>,

    /// Optional action registry for the action catalog endpoints.
    /// When `None`, the `GET /actions` endpoints return 503.
    pub action_registry: Option<Arc<ActionRegistry>>,

    /// Optional plugin registry for the plugin catalog endpoints.
    /// When `None`, the `GET /plugins` endpoints return 503.
    pub plugin_registry: Option<Arc<RwLock<PluginRegistry>>>,

    /// Optional webhook HTTP transport. When `None`, no `/webhooks/*`
    /// routes are mounted on the app; webhook-style `WebhookAction`
    /// triggers registered via `ActionRegistry::register_webhook`
    /// will never fire until the transport is attached.
    pub webhook_transport: Option<WebhookTransport>,

    /// OAuth pending state store (ADR-0031 §4.2 — TTL ≤ 10 min, single-use).
    pub oauth_pending_store: Arc<InMemoryPendingStore>,

    /// Maps signed state -> pending token so callback can consume pending data.
    pub oauth_state_tokens: Arc<RwLock<HashMap<String, PendingToken>>>,

    /// Credential state store used by OAuth callback completion.
    pub oauth_credential_store: Arc<InMemoryStore>,

    /// Optional org-slug → [`OrgId`] resolver.
    pub org_resolver: Option<Arc<dyn OrgResolver>>,

    /// Optional workspace-slug → [`WorkspaceId`] resolver.
    pub workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,

    /// Optional Plane-A authentication backend.
    ///
    /// When `Some`, the auth middleware resolves session cookies and PATs
    /// through this single contract. When `None`, only JWT and `X-API-Key`
    /// authentication paths are available.
    ///
    /// See [`crate::auth::AuthBackend`] for the trait surface and
    /// [`crate::auth::InMemoryAuthBackend`] for the default impl.
    pub auth_backend: Option<Arc<dyn AuthBackend>>,

    /// Optional membership store for RBAC role lookups.
    pub membership_store: Option<Arc<dyn MembershipStore>>,

    /// Optional idempotency store backing [`crate::middleware::IdempotencyLayer`].
    ///
    /// When `Some`, `build_app` mounts the layer on `api_routes` (NOT on the
    /// merged webhook transport) so every state-changing API endpoint is
    /// replay-protected. When `None`, the layer is not mounted and POST
    /// endpoints have no replay protection — acceptable for tests that build
    /// minimal routers but a misconfiguration in production.
    ///
    /// See ADR-0048 for the backend selection contract; the composition root
    /// chooses between [`crate::middleware::InMemoryIdempotencyStore`] and a
    /// PG-backed bridge (`StorageBackedIdempotencyStore<PgIdempotencyStore>`)
    /// based on `ApiConfig.idempotency.backend`.
    pub idempotency_store: Option<Arc<dyn IdempotencyStore>>,

    /// Optional webhook-activation repository (M3.3 / ADR-0049).
    ///
    /// When `Some`, the composition root invokes
    /// [`crate::services::webhook::bootstrap_webhook_activations`] before
    /// `build_app` to populate the transport's slug map. The same repo
    /// is consulted by the admin reload endpoint
    /// (`POST /internal/v1/webhooks/reload`).
    pub webhook_activation_repo: Option<Arc<dyn WebhookActivationRepo>>,

    /// Optional lifecycle event bus (M3.3 / ADR-0049 — E2).
    ///
    /// Producers (storage CRUD callsites) emit
    /// [`crate::services::webhook::TriggerLifecycleEvent`] on this
    /// bus; the transport-side subscriber reapplies the change
    /// without a full reload. M3.3 ships the consumer; producer
    /// wiring is deferred to a follow-up.
    pub trigger_lifecycle_bus: Option<crate::services::webhook::TriggerLifecycleBus>,

    /// Webhook credential resolver (M3.3 / ADR-0049 — E1+E3).
    ///
    /// Required for storage-driven slug bootstrap and admin reload.
    pub webhook_secret_resolver: Option<Arc<dyn crate::services::webhook::WebhookSecretResolver>>,

    /// Webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    pub webhook_ctx_factory: Option<Arc<dyn crate::services::webhook::WebhookContextFactory>>,

    /// Internal-routes shared token (M3.3 / ADR-0049 — E3).
    ///
    /// Required for `POST /internal/v1/webhooks/reload`. When `None`,
    /// every request to `/internal/v1/...` returns 503.
    pub internal_shared_token: Option<Arc<str>>,

    /// Optional spec-16 scoped execution-store port handle.
    ///
    /// When set (via [`Self::with_execution_store`]), handlers read /
    /// transition execution state through this already-scoped port
    /// instead of [`Self::execution_repo`]. The composition root wraps
    /// the raw adapter in the `nebula-tenancy` decorator so the handle
    /// is tenant-bound before it reaches `AppState`.
    pub execution_store: Option<Arc<dyn ExecutionStore>>,

    /// Optional spec-16 scoped workflow-version port handle (resume /
    /// definition lookup). Wired alongside [`Self::execution_store`].
    pub workflow_version_store: Option<Arc<dyn WorkflowVersionStore>>,

    /// Optional spec-16 scoped control-queue port handle.
    ///
    /// When set (via [`Self::with_control_queue`]), the cancel / start
    /// enqueue path uses this scoped port instead of
    /// [`Self::control_queue_repo`].
    pub control_queue: Option<Arc<dyn ControlQueue>>,
}

/// Fixed placeholder scope passed to scoped port handles.
///
/// `AppState`'s port handles are always wrapped in the
/// `nebula-tenancy` decorator, which **substitutes** its bound
/// (request-derived) tenant scope on every call and ignores the
/// argument. The concrete value here is therefore immaterial to
/// isolation — it only needs to be a valid [`Scope`].
fn placeholder_scope() -> nebula_storage_port::Scope {
    nebula_storage_port::Scope::new("nebula", "nebula")
}

impl AppState {
    /// Create new AppState with provided dependencies.
    ///
    /// `jwt_secret` is a validated [`JwtSecret`]. Obtain one from
    /// [`crate::config::ApiConfig::from_env`] (production) or
    /// `ApiConfig::for_test` (tests with the `test-util` feature).
    pub fn new(
        workflow_repo: Arc<dyn WorkflowRepo>,
        execution_repo: Arc<dyn ExecutionRepo>,
        control_queue_repo: Arc<dyn ControlQueueRepo>,
        jwt_secret: JwtSecret,
    ) -> Self {
        Self {
            jwt_secret,
            api_keys: Arc::new(Vec::new()),
            workflow_repo,
            execution_repo,
            control_queue_repo,
            metrics_registry: None,
            action_registry: None,
            plugin_registry: None,
            webhook_transport: None,
            oauth_pending_store: Arc::new(InMemoryPendingStore::new()),
            oauth_state_tokens: Arc::new(RwLock::new(HashMap::new())),
            oauth_credential_store: Arc::new(InMemoryStore::new()),
            org_resolver: None,
            workspace_resolver: None,
            auth_backend: None,
            membership_store: None,
            idempotency_store: None,
            webhook_activation_repo: None,
            trigger_lifecycle_bus: None,
            webhook_secret_resolver: None,
            webhook_ctx_factory: None,
            internal_shared_token: None,
            execution_store: None,
            workflow_version_store: None,
            control_queue: None,
        }
    }

    /// Attach the spec-16 scoped execution-store + workflow-version
    /// port handles.
    ///
    /// The composition root MUST pass handles already wrapped in the
    /// `nebula-tenancy` decorator (tenant-bound); `AppState` never sees
    /// the raw adapter. When set, handlers prefer these over the legacy
    /// [`Self::execution_repo`] / [`Self::workflow_repo`].
    #[must_use = "builder methods must be chained or built"]
    pub fn with_execution_store(
        mut self,
        execution: Arc<dyn ExecutionStore>,
        workflow_version: Arc<dyn WorkflowVersionStore>,
    ) -> Self {
        self.execution_store = Some(execution);
        self.workflow_version_store = Some(workflow_version);
        self
    }

    /// Attach the spec-16 scoped control-queue port handle (tenant-
    /// bound via the `nebula-tenancy` decorator).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_control_queue(mut self, control_queue: Arc<dyn ControlQueue>) -> Self {
        self.control_queue = Some(control_queue);
        self
    }

    /// List running execution ids. Dual-dispatch: the scoped
    /// [`ExecutionStore`] port when wired, else the legacy
    /// `ExecutionRepo`. The port path passes a fixed placeholder
    /// scope — the `nebula-tenancy` decorator substitutes the bound
    /// tenant scope, so the value here is immaterial to isolation.
    pub(crate) async fn list_running_executions(&self) -> Result<Vec<ExecutionId>, ApiError> {
        if let Some(store) = &self.execution_store {
            let ids = store
                .list_running(&placeholder_scope())
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))?;
            return ids
                .iter()
                .map(|s| {
                    ExecutionId::parse(s).map_err(|e| {
                        ApiError::Internal(format!("stored execution id {s:?} invalid: {e}"))
                    })
                })
                .collect();
        }
        self.execution_repo
            .list_running()
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))
    }

    /// List running execution ids for one workflow. Same dual-dispatch
    /// as [`Self::list_running_executions`].
    pub(crate) async fn list_running_executions_for_workflow(
        &self,
        workflow_id: nebula_core::id::WorkflowId,
    ) -> Result<Vec<ExecutionId>, ApiError> {
        if let Some(store) = &self.execution_store {
            let ids = store
                .list_running_for_workflow(&placeholder_scope(), &workflow_id.to_string())
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))?;
            return ids
                .iter()
                .map(|s| {
                    ExecutionId::parse(s).map_err(|e| {
                        ApiError::Internal(format!("stored execution id {s:?} invalid: {e}"))
                    })
                })
                .collect();
        }
        self.execution_repo
            .list_running_for_workflow(workflow_id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))
    }

    /// Set the static API keys accepted via `X-API-Key` header.
    ///
    /// Each key should use the `nbl_sk_` prefix. Keys are compared in constant
    /// time inside the auth middleware.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_api_keys(mut self, keys: Vec<String>) -> Self {
        self.api_keys = Arc::new(keys);
        self
    }

    /// Attach a metrics registry for Prometheus export via `GET /metrics`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metrics_registry(mut self, registry: Arc<MetricsRegistry>) -> Self {
        self.metrics_registry = Some(registry);
        self
    }

    /// Attach an action registry for the action catalog endpoints.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_action_registry(mut self, registry: Arc<ActionRegistry>) -> Self {
        self.action_registry = Some(registry);
        self
    }

    /// Attach a plugin registry for the plugin catalog endpoints.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_plugin_registry(mut self, registry: Arc<RwLock<PluginRegistry>>) -> Self {
        self.plugin_registry = Some(registry);
        self
    }

    /// Attach a webhook HTTP transport. The router the transport
    /// exposes gets merged into the main app router in `build_app`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_transport(mut self, transport: WebhookTransport) -> Self {
        self.webhook_transport = Some(transport);
        self
    }

    /// Attach an org resolver for slug-to-ID lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_org_resolver(mut self, resolver: Arc<dyn OrgResolver>) -> Self {
        self.org_resolver = Some(resolver);
        self
    }

    /// Attach a workspace resolver for slug-to-ID lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_workspace_resolver(mut self, resolver: Arc<dyn WorkspaceResolver>) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    /// Attach a Plane-A authentication backend.
    ///
    /// Replaces the older `with_session_store` builder; the same slot now
    /// drives session resolution, password login, MFA, PATs, and Plane-A
    /// OAuth via [`crate::auth::AuthBackend`].
    #[must_use = "builder methods must be chained or built"]
    pub fn with_auth_backend(mut self, backend: Arc<dyn AuthBackend>) -> Self {
        self.auth_backend = Some(backend);
        self
    }

    /// Attach a membership store for RBAC role lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_membership_store(mut self, store: Arc<dyn MembershipStore>) -> Self {
        self.membership_store = Some(store);
        self
    }

    /// Attach an idempotency store; `build_app` mounts
    /// [`crate::middleware::IdempotencyLayer`] on the API router when this is
    /// `Some`.
    ///
    /// See ADR-0048 for the backend selection contract.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency_store = Some(store);
        self
    }

    /// Attach a webhook-activation repository (M3.3 / ADR-0049).
    ///
    /// Required for storage-driven slug bootstrap and for the admin
    /// reload endpoint. Composition roots that do not enable
    /// `WebhookApiConfig::bootstrap_from_storage` may leave this
    /// `None`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_activation_repo(mut self, repo: Arc<dyn WebhookActivationRepo>) -> Self {
        self.webhook_activation_repo = Some(repo);
        self
    }

    /// Attach a [`crate::services::webhook::TriggerLifecycleBus`]
    /// for slug-routed activation lifecycle events (M3.3 / ADR-0049).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_lifecycle_bus(
        mut self,
        bus: crate::services::webhook::TriggerLifecycleBus,
    ) -> Self {
        self.trigger_lifecycle_bus = Some(bus);
        self
    }

    /// Attach a webhook secret resolver (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_secret_resolver(
        mut self,
        resolver: Arc<dyn crate::services::webhook::WebhookSecretResolver>,
    ) -> Self {
        self.webhook_secret_resolver = Some(resolver);
        self
    }

    /// Attach a webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_ctx_factory(
        mut self,
        factory: Arc<dyn crate::services::webhook::WebhookContextFactory>,
    ) -> Self {
        self.webhook_ctx_factory = Some(factory);
        self
    }

    /// Attach the internal-routes shared token. Required for
    /// `POST /internal/v1/webhooks/reload`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_internal_shared_token(mut self, token: impl Into<Arc<str>>) -> Self {
        self.internal_shared_token = Some(token.into());
        self
    }
}
