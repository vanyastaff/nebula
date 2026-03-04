//! Runtime configuration loaders (env-based).

use std::collections::HashSet;

use crate::state::{GithubOAuthConfig, RateLimitConfig};

pub(crate) fn load_api_keys() -> HashSet<String> {
    std::env::var("NEBULA_API_KEYS")
        .ok()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn load_github_oauth_config() -> Option<GithubOAuthConfig> {
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
    Some(GithubOAuthConfig {
        client_id,
        client_secret,
        redirect_uri,
        scope,
    })
}

pub(crate) fn load_rate_limit_config() -> RateLimitConfig {
    let window_seconds = std::env::var("NEBULA_RATE_LIMIT_WINDOW_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let max_requests = std::env::var("NEBULA_RATE_LIMIT_MAX_REQUESTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    RateLimitConfig {
        window_seconds: window_seconds.max(1),
        max_requests: max_requests.max(1),
    }
}

pub(crate) fn is_mock_oauth_enabled() -> bool {
    std::env::var("NEBULA_OAUTH_MOCK")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true)
}

pub(crate) fn load_cors_allowed_origins() -> Vec<axum::http::HeaderValue> {
    let configured = std::env::var("NEBULA_CORS_ALLOW_ORIGINS")
        .ok()
        .unwrap_or_default();

    let mut origins: Vec<axum::http::HeaderValue> = configured
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| axum::http::HeaderValue::from_str(value).ok())
        .collect();

    if origins.is_empty() {
        origins = vec![
            axum::http::HeaderValue::from_static("http://localhost:5173"),
            axum::http::HeaderValue::from_static("http://127.0.0.1:5173"),
            axum::http::HeaderValue::from_static("http://tauri.localhost"),
            axum::http::HeaderValue::from_static("tauri://localhost"),
        ];
    }

    origins
}
