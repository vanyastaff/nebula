//! OAuth endpoints and profile retrieval logic.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use url::Url;
use uuid::Uuid;

use crate::state::{ApiState, IssuedAccessToken, OAuthUserProfile, PendingOAuthState};

use super::Authenticated;

pub(crate) async fn auth_me(Authenticated(principal): Authenticated) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "provider": principal.provider,
            "accessToken": principal.access_token,
            "user": principal.user
        })),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthStartRequest {
    provider: String,
    redirect_uri: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthStartResponse {
    auth_url: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthCallbackRequest {
    provider: String,
    code: String,
    redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCallbackResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<OAuthUserProfile>,
}

fn is_mock_oauth_enabled() -> bool {
    std::env::var("NEBULA_OAUTH_MOCK")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true)
}

fn is_supported_provider(provider: &str) -> bool {
    matches!(provider, "google" | "github")
}

pub(crate) async fn oauth_start(
    State(state): State<ApiState>,
    Json(req): Json<OAuthStartRequest>,
) -> impl IntoResponse {
    let provider = req.provider.to_lowercase();
    if !is_supported_provider(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_provider",
                "message": "provider must be one of: google, github"
            })),
        )
            .into_response();
    }

    if provider == "github" {
        let Some(github) = state.github_oauth.clone() else {
            if is_mock_oauth_enabled() {
                let state_token = Uuid::new_v4().to_string();
                let auth_url = format!(
                    "{}?code=mock_{}&provider={}&state={}",
                    req.redirect_uri, state_token, provider, state_token
                );
                return (
                    StatusCode::OK,
                    Json(serde_json::json!(OAuthStartResponse {
                        auth_url,
                        state: state_token
                    })),
                )
                    .into_response();
            }

            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "github_oauth_not_configured",
                    "message": "set GITHUB_OAUTH_CLIENT_ID and GITHUB_OAUTH_CLIENT_SECRET"
                })),
            )
                .into_response();
        };

        let state_token = Uuid::new_v4().to_string();
        {
            let mut pending = state.oauth_pending.write().await;
            pending.insert(
                state_token.clone(),
                PendingOAuthState {
                    provider: "github".to_string(),
                    desktop_redirect_uri: req.redirect_uri.clone(),
                    created_at: Instant::now(),
                },
            );
            pending.retain(|_, v| v.created_at.elapsed().as_secs() <= 600);
        }

        let mut auth_url = Url::parse("https://github.com/login/oauth/authorize")
            .expect("valid github authorize url");
        auth_url
            .query_pairs_mut()
            .append_pair("client_id", &github.client_id)
            .append_pair("redirect_uri", &github.redirect_uri)
            .append_pair("scope", &github.scope)
            .append_pair("state", &state_token);

        return (
            StatusCode::OK,
            Json(serde_json::json!(OAuthStartResponse {
                auth_url: auth_url.to_string(),
                state: state_token
            })),
        )
            .into_response();
    }

    if is_mock_oauth_enabled() {
        let state_token = Uuid::new_v4().to_string();
        let auth_url = format!(
            "{}?code=mock_{}&provider={}&state={}",
            req.redirect_uri, state_token, provider, state_token
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!(OAuthStartResponse {
                auth_url,
                state: state_token
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "provider_not_implemented",
            "provider": provider
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct GithubCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn build_deep_link(base: &str, params: &[(&str, &str)]) -> String {
    if let Ok(mut url) = Url::parse(base) {
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in params {
                pairs.append_pair(key, value);
            }
        }
        return url.to_string();
    }

    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    if base.contains('?') {
        format!("{base}&{query}")
    } else {
        format!("{base}?{query}")
    }
}

pub(crate) async fn github_callback(
    State(state): State<ApiState>,
    Query(query): Query<GithubCallbackQuery>,
) -> impl IntoResponse {
    let Some(state_token) = query.state.clone() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_state",
                "message": "state is required"
            })),
        )
            .into_response();
    };

    let pending = {
        let mut map = state.oauth_pending.write().await;
        map.remove(&state_token)
    };

    let Some(pending) = pending else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_state",
                "message": "oauth state is invalid or expired"
            })),
        )
            .into_response();
    };

    if pending.provider != "github" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_provider_state",
                "message": "oauth state/provider mismatch"
            })),
        )
            .into_response();
    }

    if let Some(error) = query.error.as_deref() {
        let redirect = build_deep_link(
            &pending.desktop_redirect_uri,
            &[
                ("error", error),
                (
                    "error_description",
                    query.error_description.as_deref().unwrap_or("oauth_failed"),
                ),
                ("provider", "github"),
            ],
        );
        return Redirect::to(&redirect).into_response();
    }

    let Some(code) = query.code.as_deref() else {
        let redirect = build_deep_link(
            &pending.desktop_redirect_uri,
            &[("error", "missing_code"), ("provider", "github")],
        );
        return Redirect::to(&redirect).into_response();
    };

    let redirect = build_deep_link(
        &pending.desktop_redirect_uri,
        &[
            ("code", code),
            ("provider", "github"),
            ("state", &state_token),
        ],
    );
    Redirect::to(&redirect).into_response()
}

#[derive(Debug, Deserialize)]
struct GithubAccessTokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubUserResponse {
    id: u64,
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubEmailResponse {
    email: String,
    primary: bool,
    verified: bool,
}

fn primary_or_verified_email(emails: &[GithubEmailResponse]) -> Option<String> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email.clone())
        .or_else(|| emails.iter().find(|e| e.verified).map(|e| e.email.clone()))
}

async fn fetch_github_user_profile(
    client: &Client,
    access_token: &str,
) -> Result<OAuthUserProfile, String> {
    let user_resp = client
        .get("https://api.github.com/user")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "nebula-desktop")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("github user request failed: {e}"))?;

    let user_status = user_resp.status();
    if !user_status.is_success() {
        return Err(format!(
            "github user request status {}",
            user_status.as_u16()
        ));
    }

    let user = user_resp
        .json::<GithubUserResponse>()
        .await
        .map_err(|e| format!("github user parse failed: {e}"))?;

    let mut email = user.email.clone();
    if email.is_none() {
        let emails_resp = client
            .get("https://api.github.com/user/emails")
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "nebula-desktop")
            .bearer_auth(access_token)
            .send()
            .await;

        if let Ok(resp) = emails_resp
            && resp.status().is_success()
            && let Ok(emails) = resp.json::<Vec<GithubEmailResponse>>().await
        {
            email = primary_or_verified_email(&emails);
        }
    }

    Ok(OAuthUserProfile {
        id: user.id.to_string(),
        login: user.login,
        name: user.name,
        email,
        avatar_url: user.avatar_url,
    })
}

async fn issue_access_token(
    state: &ApiState,
    provider: &str,
    access_token: String,
    token_type: String,
    expires_in: u64,
    user: Option<OAuthUserProfile>,
) -> Response {
    {
        let mut tokens = state.access_tokens.write().await;
        tokens.insert(
            access_token.clone(),
            IssuedAccessToken {
                provider: provider.to_string(),
                issued_at: Instant::now(),
                expires_in,
                user: user.clone(),
            },
        );
        tokens.retain(|_, record| record.issued_at.elapsed().as_secs() <= record.expires_in);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!(OAuthCallbackResponse {
            access_token,
            token_type,
            expires_in,
            user
        })),
    )
        .into_response()
}

pub(crate) async fn oauth_callback(
    State(state): State<ApiState>,
    Json(req): Json<OAuthCallbackRequest>,
) -> impl IntoResponse {
    let provider = req.provider.to_lowercase();
    if !is_supported_provider(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_provider",
                "message": "provider must be one of: google, github"
            })),
        )
            .into_response();
    }

    if req.code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_code",
                "message": "code is required"
            })),
        )
            .into_response();
    }

    if provider == "github" {
        if is_mock_oauth_enabled() && req.code.starts_with("mock_") {
            let token = format!("mock_token_{}_{}", provider, Uuid::new_v4());
            return issue_access_token(&state, &provider, token, "Bearer".to_string(), 3600, None)
                .await;
        }

        let Some(github) = state.github_oauth.as_ref() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "github_oauth_not_configured",
                    "message": "set GITHUB_OAUTH_CLIENT_ID and GITHUB_OAUTH_CLIENT_SECRET"
                })),
            )
                .into_response();
        };

        let form = [
            ("client_id", github.client_id.as_str()),
            ("client_secret", github.client_secret.as_str()),
            ("code", req.code.as_str()),
            ("redirect_uri", github.redirect_uri.as_str()),
        ];

        let token_response = match state
            .http_client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&form)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": "oauth_provider_unreachable",
                        "message": err.to_string()
                    })),
                )
                    .into_response();
            }
        };

        let status = token_response.status();
        let payload = match token_response.json::<GithubAccessTokenResponse>().await {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": "oauth_provider_invalid_response",
                        "message": err.to_string()
                    })),
                )
                    .into_response();
            }
        };

        if !status.is_success() {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "oauth_provider_failed",
                    "status": status.as_u16(),
                    "providerError": payload.error,
                    "providerErrorDescription": payload.error_description
                })),
            )
                .into_response();
        }

        if let Some(error) = payload.error {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": error,
                    "message": payload.error_description.unwrap_or_else(|| "github oauth failed".to_string())
                })),
            )
                .into_response();
        }

        let Some(access_token) = payload.access_token else {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "oauth_token_missing",
                    "message": "provider did not return access token"
                })),
            )
                .into_response();
        };

        let user_profile = fetch_github_user_profile(&state.http_client, &access_token)
            .await
            .ok();

        return issue_access_token(
            &state,
            &provider,
            access_token,
            payload.token_type.unwrap_or_else(|| "Bearer".to_string()),
            3600,
            user_profile,
        )
        .await;
    }

    if is_mock_oauth_enabled() && req.code.starts_with("mock_") {
        let token = format!("mock_token_{}_{}", provider, Uuid::new_v4());
        return issue_access_token(&state, &provider, token, "Bearer".to_string(), 3600, None)
            .await;
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "oauth_callback_not_configured",
            "message": "real OAuth code exchange is not implemented yet",
            "provider": provider,
            "redirectUri": req.redirect_uri
        })),
    )
        .into_response()
}
