//! Shared API state and related auth/rate-limit records.

use nebula_ports::{ExecutionRepo, WorkflowRepo};
use reqwest::Client;
use serde::Serialize;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::RwLock;

use crate::{config, models::WorkerStatus};

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
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
    pub(crate) api_keys: Arc<std::collections::HashSet<String>>,
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
    pub fn new(workers: Vec<WorkerStatus>) -> Self {
        Self {
            workers,
            oauth_pending: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
            github_oauth: config::load_github_oauth_config(),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(config::load_api_keys()),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            rate_limit_config: config::load_rate_limit_config(),
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

#[derive(Debug, Clone)]
pub(crate) struct GithubOAuthConfig {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) redirect_uri: String,
    pub(crate) scope: String,
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
