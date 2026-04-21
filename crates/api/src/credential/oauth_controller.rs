//! OAuth controller endpoints (ADR-0031 rollout slice).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use nebula_credential::{generate_code_challenge, generate_pkce_verifier, generate_random_state};
use serde::{Deserialize, Serialize};

use crate::{
    credential::{
        flow::{
            AuthorizationUriRequest, TokenExchangeRequest, build_authorization_uri, exchange_code,
        },
        state::{OAuthStateSigner, build_signed_state},
    },
    errors::{ApiError, ApiResult},
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
    /// PKCE verifier generated server-side.
    pub code_verifier: String,
}

/// GET `/credentials/{id}/oauth2/auth`
pub async fn get_oauth2_authorize_url(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Query(query): Query<AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    let csrf = generate_random_state();
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let (signed_state, _payload) = build_signed_state(&signer, &credential_id, csrf)
        .map_err(|e| ApiError::Internal(format!("state generation failed: {e}")))?;

    let code_verifier = generate_pkce_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
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
        code_verifier,
    }))
}

/// GET callback params.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
    /// OAuth token endpoint URL.
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client secret.
    pub client_secret: String,
    /// Redirect URI.
    pub redirect_uri: String,
    /// PKCE verifier.
    pub code_verifier: String,
}

/// POST callback body (for form_post response mode).
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackBody {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
    /// OAuth token endpoint URL.
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client secret.
    pub client_secret: String,
    /// Redirect URI.
    pub redirect_uri: String,
    /// PKCE verifier.
    pub code_verifier: String,
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
    Query(query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(
        &credential_id,
        &state,
        TokenExchangeRequest {
            token_url: query.token_url,
            client_id: query.client_id,
            client_secret: query.client_secret,
            code: query.code,
            redirect_uri: query.redirect_uri,
            code_verifier: query.code_verifier,
            auth_style: nebula_credential::credentials::oauth2::AuthStyle::Header,
        },
        query.state,
    )
    .await
}

/// POST `/credentials/{id}/oauth2/callback`
pub async fn post_oauth2_callback(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(
        &credential_id,
        &state,
        TokenExchangeRequest {
            token_url: body.token_url,
            client_id: body.client_id,
            client_secret: body.client_secret,
            code: body.code,
            redirect_uri: body.redirect_uri,
            code_verifier: body.code_verifier,
            auth_style: nebula_credential::credentials::oauth2::AuthStyle::Header,
        },
        body.state,
    )
    .await
}

async fn handle_callback(
    credential_id: &str,
    state: &AppState,
    token_exchange: TokenExchangeRequest,
    signed_state: String,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    signer
        .verify_for_credential(&signed_state, credential_id)
        .map_err(|e| ApiError::Unauthorized(format!("oauth state validation failed: {e}")))?;

    exchange_code(&token_exchange)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(OAuthCallbackResponse {
        credential_id: credential_id.to_owned(),
        exchanged: true,
    }))
}
