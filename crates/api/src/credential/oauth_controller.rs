//! OAuth controller endpoints (ADR-0031 rollout slice).

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    routing::get,
};
use nebula_credential::{
    PendingState, PendingStateStore, SecretString, generate_code_challenge, generate_pkce_verifier,
    generate_random_state,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::{
    credential::{
        flow::{
            AuthorizationUriRequest, TokenExchangeRequest, build_authorization_uri, exchange_code,
        },
        state::{OAuthStateSigner, build_signed_state},
    },
    errors::{ApiError, ApiResult},
    middleware::auth::AuthenticatedUser,
    state::AppState,
};

/// OAuth controller routes mounted under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/credentials/{id}/oauth2/auth",
            get(get_oauth2_authorize_url),
        )
        .route(
            "/credentials/{id}/oauth2/callback",
            get(get_oauth2_callback).post(post_oauth2_callback),
        )
}

/// Response with provider authorization URL and generated state.
#[derive(Debug, Serialize)]
pub struct AuthorizationUriResponse {
    /// URL where the browser should be redirected.
    pub authorize_url: String,
    /// Signed opaque OAuth state.
    pub state: String,
}

/// GET `/credentials/{id}/oauth2/auth`
pub async fn get_oauth2_authorize_url(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    let csrf = generate_random_state();
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let (signed_state, payload) = build_signed_state(&signer, &credential_id, csrf)
        .map_err(|e| ApiError::Internal(format!("state generation failed: {e}")))?;

    let code_verifier = generate_pkce_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let pending = OAuthPendingExchange {
        token_url: query.token_url.clone(),
        client_id: query.client_id.clone(),
        client_secret: SecretString::new(query.client_secret.clone()),
        redirect_uri: query.redirect_uri.clone(),
        code_verifier: SecretString::new(code_verifier),
        auth_style: query
            .auth_style
            .unwrap_or(nebula_credential::credentials::oauth2::AuthStyle::Header),
    };
    let pending_token = state
        .oauth_pending_store
        .put("oauth2", &user.user_id, &payload.csrf_token, pending)
        .await
        .map_err(|e| ApiError::Internal(format!("pending oauth state store failed: {e}")))?;
    state
        .oauth_state_tokens
        .write()
        .await
        .insert(signed_state.clone(), pending_token);

    let authorize_url =
        build_authorization_uri(&query, &signed_state, &code_challenge).map_err(|e| {
            ApiError::Validation {
                detail: format!("invalid authorization url: {e}"),
                errors: Vec::new(),
            }
        })?;

    Ok(Json(AuthorizationUriResponse {
        authorize_url: authorize_url.to_string(),
        state: signed_state,
    }))
}

/// GET callback params.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// POST callback body (for form_post response mode).
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackBody {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// Callback response while persistence wiring is still in rollout.
#[derive(Debug, Serialize)]
pub struct OAuthCallbackResponse {
    /// Credential id from route.
    pub credential_id: String,
    /// True when callback state verified and code exchange succeeded.
    pub exchanged: bool,
}

/// GET `/credentials/{id}/oauth2/callback`
pub async fn get_oauth2_callback(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(&credential_id, &state, &user, query.code, query.state).await
}

/// POST `/credentials/{id}/oauth2/callback`
pub async fn post_oauth2_callback(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(body): Json<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(&credential_id, &state, &user, body.code, body.state).await
}

async fn handle_callback(
    credential_id: &str,
    state: &AppState,
    user: &AuthenticatedUser,
    code: String,
    signed_state: String,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let payload = signer
        .verify_for_credential(&signed_state, credential_id)
        .map_err(|e| ApiError::Unauthorized(format!("oauth state validation failed: {e}")))?;

    let token = state
        .oauth_state_tokens
        .write()
        .await
        .remove(&signed_state)
        .ok_or_else(|| {
            ApiError::Unauthorized("oauth state not found or already consumed".to_owned())
        })?;
    let pending = state
        .oauth_pending_store
        .consume::<OAuthPendingExchange>("oauth2", &token, &user.user_id, &payload.csrf_token)
        .await
        .map_err(|e| ApiError::Unauthorized(format!("oauth pending consume failed: {e}")))?;

    let token_exchange = TokenExchangeRequest {
        token_url: pending.token_url,
        client_id: pending.client_id,
        client_secret: pending.client_secret.expose_secret(ToOwned::to_owned),
        code,
        redirect_uri: pending.redirect_uri,
        code_verifier: pending.code_verifier.expose_secret(ToOwned::to_owned),
        auth_style: pending.auth_style,
    };
    exchange_code(&token_exchange)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(OAuthCallbackResponse {
        credential_id: credential_id.to_owned(),
        exchanged: true,
    }))
}

/// Pending data required to exchange authorization code in callback.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthPendingExchange {
    token_url: String,
    client_id: String,
    #[serde(with = "nebula_credential::serde_secret")]
    client_secret: SecretString,
    redirect_uri: String,
    #[serde(with = "nebula_credential::serde_secret")]
    code_verifier: SecretString,
    auth_style: nebula_credential::credentials::oauth2::AuthStyle,
}

impl Zeroize for OAuthPendingExchange {
    fn zeroize(&mut self) {
        self.token_url.zeroize();
        self.client_id.zeroize();
        self.client_secret.zeroize();
        self.redirect_uri.zeroize();
        self.code_verifier.zeroize();
    }
}

impl PendingState for OAuthPendingExchange {
    const KIND: &'static str = "api_oauth_pending";

    fn expires_in(&self) -> std::time::Duration {
        std::time::Duration::from_secs(600)
    }
}
