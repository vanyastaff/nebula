//! Shared API state and related auth/rate-limit records.

use nebula_ports::{ExecutionRepo, WorkflowRepo};
use nebula_webhook::WebhookServer;
use reqwest::Client;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};
use tokio::sync::RwLock;

use crate::status::WorkerStatus;

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Embedded webhook server (same process).
    pub(crate) webhook: Arc<WebhookServer>,
    /// Snapshot of node workers (e.g. 4 workers).
    pub(crate) workers: Vec<WorkerStatus>,
    /// Pending OAuth state values (state -> callback metadata).
    pub(crate) oauth_pending: Arc<RwLock<HashMap<String, PendingOAuthState>>>,
    /// Shared HTTP client used for OAuth token exchange.
    pub(crate) http_client: Client,
    /// GitHub OAuth configuration (enabled when env vars are present).
    pub(crate) github_oauth: Option<GithubOAuthConfig>,
    /// Access tokens issued by `/auth/oauth/callback`.
    pub(crate) access_tokens: Arc<RwLock<HashMap<String, IssuedAccessToken>>>,
    /// API keys for machine-to-machine auth (`X-API-Key`).
    pub(crate) api_keys: Arc<HashSet<String>>,
    /// Sliding window counters for protected route rate limiting.
    pub(crate) rate_limits: Arc<RwLock<HashMap<String, RateLimitEntry>>>,
    /// Rate limit config for protected routes.
    pub(crate) rate_limit_config: RateLimitConfig,
    /// Optional workflow persistence port (Phase 1).
    pub(crate) workflow_repo: Option<Arc<dyn WorkflowRepo>>,
    /// Optional execution persistence/coordination port (Phase 1).
    pub(crate) execution_repo: Option<Arc<dyn ExecutionRepo>>,
}

impl ApiState {
    /// Build API state from environment.
    pub fn new(webhook: Arc<WebhookServer>, workers: Vec<WorkerStatus>) -> Self {
        Self {
            webhook,
            workers,
            oauth_pending: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
            github_oauth: GithubOAuthConfig::from_env(),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(load_api_keys()),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            rate_limit_config: RateLimitConfig::from_env(),
            workflow_repo: None,
            execution_repo: None,
        }
    }

    /// Attach workflow repository dependency.
    pub fn with_workflow_repo(mut self, workflow_repo: Arc<dyn WorkflowRepo>) -> Self {
        self.workflow_repo = Some(workflow_repo);
        self
    }

    /// Attach execution repository dependency.
    pub fn with_execution_repo(mut self, execution_repo: Arc<dyn ExecutionRepo>) -> Self {
        self.execution_repo = Some(execution_repo);
        self
    }
}

fn load_api_keys() -> HashSet<String> {
    std::env::var("NEBULA_API_KEYS")
        .ok()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct GithubOAuthConfig {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) redirect_uri: String,
    pub(crate) scope: String,
}

impl GithubOAuthConfig {
    pub(crate) fn from_env() -> Option<Self> {
        let client_id = std::env::var("GITHUB_OAUTH_CLIENT_ID").ok()?;
        let client_secret = std::env::var("GITHUB_OAUTH_CLIENT_SECRET").ok()?;
        let redirect_uri = std::env::var("GITHUB_OAUTH_REDIRECT_URI")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:5678/auth/github/callback".to_string());
        let scope = std::env::var("GITHUB_OAUTH_SCOPE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "read:user user:email".to_string());
        Some(Self {
            client_id,
            client_secret,
            redirect_uri,
            scope,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingOAuthState {
    pub(crate) provider: String,
    pub(crate) desktop_redirect_uri: String,
    pub(crate) created_at: Instant,
}

#[derive(Debug, Clone)]
pub(crate) struct IssuedAccessToken {
    pub(crate) provider: String,
    pub(crate) issued_at: Instant,
    pub(crate) expires_in: u64,
    pub(crate) user: Option<OAuthUserProfile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthPrincipal {
    pub(crate) provider: String,
    pub(crate) access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<OAuthUserProfile>,
}

#[derive(Debug, Clone)]
pub(crate) struct RateLimitConfig {
    pub(crate) window_seconds: u64,
    pub(crate) max_requests: u32,
}

impl RateLimitConfig {
    pub(crate) fn from_env() -> Self {
        let window_seconds = std::env::var("NEBULA_RATE_LIMIT_WINDOW_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let max_requests = std::env::var("NEBULA_RATE_LIMIT_MAX_REQUESTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        Self {
            window_seconds: window_seconds.max(1),
            max_requests: max_requests.max(1),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RateLimitEntry {
    pub(crate) window_started_at: Instant,
    pub(crate) request_count: u32,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthUserProfile {
    pub(crate) id: String,
    pub(crate) login: String,
    pub(crate) name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) avatar_url: Option<String>,
}
