//! Application State
//!
//! Shared state for all handlers via Arc.
//! Contains only ports (traits) — independent of concrete implementations.

#[cfg(feature = "credential-oauth")]
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, WorkspaceId, WorkspaceRole, scope::Principal};
#[cfg(feature = "credential-oauth")]
use nebula_credential::PendingToken;
use nebula_plugin::PluginRegistry;
use nebula_runtime::ActionRegistry;
#[cfg(feature = "credential-oauth")]
use nebula_storage::credential::{InMemoryPendingStore, InMemoryStore};
use nebula_storage::{ExecutionRepo, WorkflowRepo, repos::ControlQueueRepo};
use nebula_telemetry::metrics::MetricsRegistry;
use tokio::sync::RwLock;

use crate::{config::JwtSecret, errors::ApiError, services::webhook::WebhookTransport};

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

/// Session storage for cookie-based authentication.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Retrieve the [`Principal`] associated with a session ID, if any.
    async fn get_principal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<Principal>, ApiError>;
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

    /// Feature-gated OAuth pending store (ADR-0031 rollout slice).
    #[cfg(feature = "credential-oauth")]
    pub oauth_pending_store: Arc<InMemoryPendingStore>,

    /// Maps signed state -> pending token so callback can consume pending data.
    #[cfg(feature = "credential-oauth")]
    pub oauth_state_tokens: Arc<RwLock<HashMap<String, PendingToken>>>,

    /// Credential state store used by OAuth callback completion in rollout mode.
    #[cfg(feature = "credential-oauth")]
    pub oauth_credential_store: Arc<InMemoryStore>,

    /// Optional org-slug → [`OrgId`] resolver.
    pub org_resolver: Option<Arc<dyn OrgResolver>>,

    /// Optional workspace-slug → [`WorkspaceId`] resolver.
    pub workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,

    /// Optional session store for cookie-based auth.
    pub session_store: Option<Arc<dyn SessionStore>>,

    /// Optional membership store for RBAC role lookups.
    pub membership_store: Option<Arc<dyn MembershipStore>>,
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
            #[cfg(feature = "credential-oauth")]
            oauth_pending_store: Arc::new(InMemoryPendingStore::new()),
            #[cfg(feature = "credential-oauth")]
            oauth_state_tokens: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "credential-oauth")]
            oauth_credential_store: Arc::new(InMemoryStore::new()),
            org_resolver: None,
            workspace_resolver: None,
            session_store: None,
            membership_store: None,
        }
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

    /// Attach a session store for cookie-based authentication.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Attach a membership store for RBAC role lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_membership_store(mut self, store: Arc<dyn MembershipStore>) -> Self {
        self.membership_store = Some(store);
        self
    }
}
