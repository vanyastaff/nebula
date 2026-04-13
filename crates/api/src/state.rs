//! Application State
//!
//! Shared state for all handlers via Arc.
//! Contains only ports (traits) — independent of concrete implementations.

use std::sync::Arc;

use nebula_config::Config;
use nebula_plugin::PluginRegistry;
use nebula_runtime::ActionRegistry;
use nebula_storage::{ExecutionRepo, WorkflowRepo};
use nebula_telemetry::metrics::MetricsRegistry;
use tokio::sync::RwLock;

use crate::webhook::WebhookTransport;

/// Application state passed through `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    /// Configuration
    pub config: Arc<Config>,

    /// JWT secret used to validate Bearer tokens.
    /// Must be at least 32 bytes of random entropy in production.
    pub jwt_secret: Arc<str>,

    /// Static API keys accepted via `X-API-Key` header.
    ///
    /// Each key must use the `nbl_sk_` prefix. Compared in constant time.
    /// An empty `Vec` means API key auth is disabled for this route group.
    pub api_keys: Arc<Vec<String>>,

    /// Workflow Repository (port/trait)
    pub workflow_repo: Arc<dyn WorkflowRepo>,

    /// Execution Repository (port/trait)
    pub execution_repo: Arc<dyn ExecutionRepo>,

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
}

impl AppState {
    /// Create new AppState with provided dependencies.
    ///
    /// `jwt_secret` must be at least 32 bytes long in production.
    pub fn new(
        config: Config,
        workflow_repo: Arc<dyn WorkflowRepo>,
        execution_repo: Arc<dyn ExecutionRepo>,
        jwt_secret: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            jwt_secret: jwt_secret.into(),
            api_keys: Arc::new(Vec::new()),
            workflow_repo,
            execution_repo,
            metrics_registry: None,
            action_registry: None,
            plugin_registry: None,
            webhook_transport: None,
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
}
