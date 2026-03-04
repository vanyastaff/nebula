//! Authentication extractor and per-principal rate limiting.

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts},
};
use std::time::Instant;
use tokio::sync::RwLockWriteGuard;

use crate::{
    error::ApiHttpError,
    state::{ApiState, AuthPrincipal, RateLimitEntry},
};

pub(crate) struct Authenticated(pub(crate) AuthPrincipal);

impl<S> FromRequestParts<S> for Authenticated
where
    ApiState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiHttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = ApiState::from_ref(state);
        let principal = if let Some(header_value) = parts.headers.get(header::AUTHORIZATION) {
            let Ok(raw) = header_value.to_str() else {
                return Err(ApiHttpError::unauthorized(
                    "invalid_authorization_header",
                    "authorization header is invalid",
                ));
            };

            let Some(token) = raw.strip_prefix("Bearer ").map(str::trim) else {
                return Err(ApiHttpError::unauthorized(
                    "invalid_authorization_scheme",
                    "authorization scheme must be Bearer",
                ));
            };

            if token.is_empty() {
                return Err(ApiHttpError::unauthorized(
                    "invalid_bearer_token",
                    "bearer token must not be empty",
                ));
            }

            let mut tokens = state.access_tokens.write().await;
            tokens.retain(|_, record| record.issued_at.elapsed().as_secs() <= record.expires_in);
            let Some(record) = tokens.get(token).cloned() else {
                return Err(ApiHttpError::unauthorized(
                    "invalid_token",
                    "token is unknown, expired, or revoked",
                ));
            };
            AuthPrincipal {
                provider: record.provider,
                access_token: token.to_string(),
                user: record.user,
            }
        } else if let Some(api_key_header) = parts.headers.get("x-api-key") {
            let Ok(api_key) = api_key_header.to_str() else {
                return Err(ApiHttpError::unauthorized(
                    "invalid_api_key",
                    "x-api-key is invalid",
                ));
            };

            if api_key.trim().is_empty() {
                return Err(ApiHttpError::unauthorized(
                    "invalid_api_key",
                    "x-api-key must not be empty",
                ));
            }

            if !state.api_keys.contains(api_key) {
                return Err(ApiHttpError::unauthorized(
                    "invalid_api_key",
                    "x-api-key is invalid",
                ));
            }

            AuthPrincipal {
                provider: "api_key".to_string(),
                access_token: "[api_key]".to_string(),
                user: None,
            }
        } else {
            return Err(ApiHttpError::unauthorized(
                "missing_authentication",
                "provide Authorization: Bearer <token> or X-API-Key",
            ));
        };

        let rate_key = format!("{}:{}", principal.provider, parts.uri.path());
        if let Some(retry_after) = check_rate_limit(&state, &rate_key).await {
            return Err(ApiHttpError::too_many_requests(
                "rate_limited",
                "too many requests",
                retry_after,
            ));
        }

        Ok(Self(principal))
    }
}

async fn check_rate_limit(state: &ApiState, key: &str) -> Option<u64> {
    let window_seconds = state.rate_limit_config.window_seconds;
    let max_requests = state.rate_limit_config.max_requests;
    let now = Instant::now();

    let mut limits: RwLockWriteGuard<'_, std::collections::HashMap<String, RateLimitEntry>> =
        state.rate_limits.write().await;
    limits.retain(|_, entry| entry.window_started_at.elapsed().as_secs() <= window_seconds);

    let entry = limits.entry(key.to_string()).or_insert(RateLimitEntry {
        window_started_at: now,
        request_count: 0,
    });

    let elapsed = entry.window_started_at.elapsed().as_secs();
    if elapsed >= window_seconds {
        entry.window_started_at = now;
        entry.request_count = 0;
    }

    if entry.request_count >= max_requests {
        let remaining = window_seconds.saturating_sub(entry.window_started_at.elapsed().as_secs());
        return Some(remaining.max(1));
    }

    entry.request_count += 1;
    None
}
