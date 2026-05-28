//! Authentication endpoint handlers — Plane A.
//!
//! Each handler is a thin shim over [`crate::domain::auth::backend::AuthBackend`].
//! Validation lives in the backend; the HTTP layer extracts the request body,
//! dispatches, attaches `Set-Cookie` headers, and translates
//! [`crate::domain::auth::backend::AuthError`] into [`crate::error::ApiError`].
//!
//! Per auth plane separation these endpoints belong to **Plane A** (host login). They
//! never touch the credential / Plane B OAuth state.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE},
    response::IntoResponse,
};
use nebula_core::Principal;
use serde::Deserialize;

use crate::{
    domain::{
        auth::backend::{
            AuthBackend, AuthError, CSRF_COOKIE, ForgotPasswordRequest, LoginRequest,
            LoginResponse, MfaChallengeResponse, MfaConfirmEnrollRequest, MfaEnrollResponse,
            MfaLoginCompleteRequest, OAuthProvider, OAuthStartResponse, PasswordOutcome,
            ResetPasswordRequest, SESSION_COOKIE, SignupRequest, SignupResponse, UserProfile,
            VerifyEmailRequest, cleared_cookie, csrf_cookie, session_cookie,
        },
        shared::AckResponse,
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
};

fn backend(state: &AppState) -> Result<&Arc<dyn AuthBackend>, ApiError> {
    state
        .auth_backend
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("auth backend not configured".to_owned()))
}

fn cookie_headers(set_cookies: &[String]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for c in set_cookies {
        if let Ok(value) = HeaderValue::from_str(c) {
            headers.append(SET_COOKIE, value);
        }
    }
    headers
}

fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for pair in cookie.split(';') {
        let pair = pair.trim();
        if let Some(rest) = pair.strip_prefix(SESSION_COOKIE)
            && let Some(value) = rest.strip_prefix('=')
        {
            return Some(value.to_owned());
        }
    }
    None
}

fn user_id_from_principal(principal: &Principal) -> Result<String, ApiError> {
    match principal {
        Principal::User(id) => Ok(id.to_string()),
        _ => Err(ApiError::Forbidden("user principal required".to_owned())),
    }
}

/// `POST /api/v1/auth/signup` — register a new user.
#[utoipa::path(
    post,
    path = "/auth/signup",
    tag = "auth",
    security(()),
    request_body = SignupRequest,
    responses(
        (status = 200, description = "User registered; verification email queued.", body = SignupResponse),
        (status = 400, description = "Validation error (e.g. weak password, malformed email).", body = ProblemDetails),
        (status = 409, description = "Email is already registered.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body), fields(email = %body.email))]
pub async fn signup(
    State(state): State<AppState>,
    Json(body): Json<SignupRequest>,
) -> ApiResult<Json<SignupResponse>> {
    let backend = backend(&state)?;
    let user = backend.register_user(body).await.map_err(ApiError::from)?;
    Ok(Json(SignupResponse {
        user,
        verification_email_sent: true,
    }))
}

/// `POST /api/v1/auth/login` — verify password and (optionally) TOTP.
///
/// Returns either a `LoginResponse` (200) when password (and TOTP, when
/// enrolled) succeed, or an `MfaChallengeResponse` (202) when MFA is
/// required for the second step.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    security(()),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated; session and CSRF cookies issued.", body = LoginResponse),
        (status = 202, description = "Password OK but MFA verification is required; submit the challenge token to `/auth/mfa/verify`.", body = MfaChallengeResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Invalid credentials, locked account, or expired session.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body), fields(email = %body.email))]
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<axum::response::Response> {
    let backend = backend(&state)?;
    let outcome = backend
        .authenticate_password(&body.email, body.password.expose(), body.totp.as_deref())
        .await
        .map_err(ApiError::from)?;

    match outcome {
        PasswordOutcome::Authenticated(user) => {
            let response = mint_session_response(backend, user).await?;
            Ok(response)
        },
        PasswordOutcome::MfaRequired { challenge_token } => {
            let resp = MfaChallengeResponse {
                mfa_required: true,
                challenge_token,
            };
            Ok((StatusCode::ACCEPTED, Json(resp)).into_response())
        },
    }
}

async fn mint_session_response(
    backend: &Arc<dyn AuthBackend>,
    user: UserProfile,
) -> ApiResult<axum::response::Response> {
    let session = backend
        .create_session(&user.user_id)
        .await
        .map_err(ApiError::from)?;
    let resp = LoginResponse {
        user,
        session_id: session.id.clone(),
        csrf_token: session.csrf_token.clone(),
    };
    let headers = cookie_headers(&[
        session_cookie(&session.id),
        csrf_cookie(&session.csrf_token),
    ]);
    Ok((StatusCode::OK, headers, Json(resp)).into_response())
}

/// `POST /api/v1/auth/logout` — revoke the active session and clear cookies.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    security(()),
    responses(
        (status = 200, description = "Session revoked (or absent); session and CSRF cookies cleared.", body = AckResponse),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, headers))]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<axum::response::Response> {
    let backend = backend(&state)?;
    if let Some(session_id) = extract_session_id(&headers) {
        backend
            .revoke_session(&session_id)
            .await
            .map_err(ApiError::from)?;
    }
    let cleared = cookie_headers(&[cleared_cookie(SESSION_COOKIE), cleared_cookie(CSRF_COOKIE)]);
    Ok((StatusCode::OK, cleared, Json(AckResponse::ok())).into_response())
}

/// `POST /api/v1/auth/forgot-password` — always 202 to avoid enumeration.
#[utoipa::path(
    post,
    path = "/auth/forgot-password",
    tag = "auth",
    security(()),
    request_body = ForgotPasswordRequest,
    responses(
        (status = 202, description = "Reset email queued (always returned, regardless of whether the email is registered, to avoid account enumeration).", body = AckResponse),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body))]
pub async fn forgot_password(
    State(state): State<AppState>,
    Json(body): Json<ForgotPasswordRequest>,
) -> ApiResult<(StatusCode, Json<AckResponse>)> {
    let backend = backend(&state)?;
    backend
        .request_password_reset(&body.email)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::ACCEPTED, Json(AckResponse::ok())))
}

/// `POST /api/v1/auth/reset-password` — consume reset token, set new pass.
#[utoipa::path(
    post,
    path = "/auth/reset-password",
    tag = "auth",
    security(()),
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password reset.", body = AckResponse),
        (status = 400, description = "Validation error (e.g. weak new password).", body = ProblemDetails),
        (status = 401, description = "Reset token is invalid, expired, or already consumed.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body))]
pub async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordRequest>,
) -> ApiResult<Json<AckResponse>> {
    let backend = backend(&state)?;
    backend
        .complete_password_reset(&body.token, body.new_password.expose())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(AckResponse::ok()))
}

/// `POST /api/v1/auth/verify-email` — consume one-time verification token.
#[utoipa::path(
    post,
    path = "/auth/verify-email",
    tag = "auth",
    security(()),
    request_body = VerifyEmailRequest,
    responses(
        (status = 200, description = "Email address verified.", body = AckResponse),
        (status = 401, description = "Verification token is invalid, expired, or already consumed.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body))]
pub async fn verify_email(
    State(state): State<AppState>,
    Json(body): Json<VerifyEmailRequest>,
) -> ApiResult<Json<AckResponse>> {
    let backend = backend(&state)?;
    backend
        .verify_email(&body.token)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(AckResponse::ok()))
}

/// `POST /api/v1/auth/mfa/enroll` — return otpauth URI + base32 secret.
///
/// Session-bearing; mounted on the CSRF-gated `auth_mfa_session_router`
/// (see `crate::domain::auth::routes::mfa_session_router`). The principal
/// is read from the `AuthContext` populated by `auth_middleware`, so any
/// auth method that produces a `Principal::User` is accepted (session
/// cookie, JWT, PAT, API key). PAT and API-key callers stay CSRF-exempt
/// inside `csrf_middleware` as usual.
#[utoipa::path(
    post,
    path = "/auth/mfa/enroll",
    tag = "auth",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 200, description = "Enrollment payload — display the otpauth URI as a QR code; the user must confirm via `/auth/mfa/verify`.", body = MfaEnrollResponse),
        (status = 401, description = "Authentication required (no valid session/bearer/api-key).", body = ProblemDetails),
        (status = 403, description = "Authenticated principal is not a user (e.g. service account).", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, auth))]
pub async fn mfa_enroll(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<MfaEnrollResponse>> {
    let backend = backend(&state)?;
    let user_id = user_id_from_principal(&auth.principal)?;
    let enroll = backend
        .start_mfa_enrollment(&user_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(MfaEnrollResponse {
        otpauth_uri: enroll.otpauth_uri,
        secret_base32: enroll.secret_base32,
    }))
}

/// `POST /api/v1/auth/mfa/verify` — confirm enrollment for the current user.
///
/// Session-bearing; CSRF-gated by `csrf_middleware` (the route lives in the
/// session-required sub-group `auth_mfa_session_router`). The second-factor
/// login-completion path now lives at [`mfa_complete_login`]
/// (`POST /auth/login/mfa`) because it is cookie-less and therefore
/// CSRF-exempt by construction.
#[utoipa::path(
    post,
    path = "/auth/mfa/verify",
    tag = "auth",
    security(("bearer" = []), ("api_key" = [])),
    request_body = MfaConfirmEnrollRequest,
    responses(
        (status = 200, description = "Enrollment confirmed; the account now requires MFA at login.", body = AckResponse),
        (status = 401, description = "Invalid TOTP code or missing session.", body = ProblemDetails),
        (status = 403, description = "Authenticated principal is not a user.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, auth, body))]
pub async fn mfa_verify(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<MfaConfirmEnrollRequest>,
) -> ApiResult<Json<AckResponse>> {
    let backend = backend(&state)?;
    let user_id = user_id_from_principal(&auth.principal)?;
    backend
        .confirm_mfa_enrollment(&user_id, &body.code)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(AckResponse::ok()))
}

/// `POST /api/v1/auth/login/mfa` — complete a second-factor login.
///
/// Cookie-less by design: the caller has no session yet, so this route is
/// CSRF-exempt by construction and lives on the flat unauthenticated
/// `/auth/*` sub-router. The `challenge_token` issued by the password step
/// in `/auth/login` is the sole authority.
#[utoipa::path(
    post,
    path = "/auth/login/mfa",
    tag = "auth",
    security(()),
    request_body = MfaLoginCompleteRequest,
    responses(
        (status = 200, description = "Second factor accepted; session and CSRF cookies issued.", body = LoginResponse),
        (status = 401, description = "Invalid TOTP code or expired challenge token.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, body))]
pub async fn mfa_complete_login(
    State(state): State<AppState>,
    Json(body): Json<MfaLoginCompleteRequest>,
) -> ApiResult<axum::response::Response> {
    let backend = backend(&state)?;
    let user = backend
        .verify_mfa(&body.challenge_token, &body.code)
        .await
        .map_err(ApiError::from)?;
    mint_session_response(backend, user).await
}

/// Derive the canonical Plane-A OAuth `redirect_uri` per ADR-0085 D-3
/// (recon-4): `format!("{}/auth/oauth/{}/callback", public_url,
/// provider.as_str())`.
///
/// Shared by `oauth_start` and `oauth_callback` so the value persisted
/// in the OAuth state row at start_oauth time matches the value
/// re-derived at callback time (`public_url_changed_mid_flow` defense
/// per REQ-oauth-003 Scenario 3.10 — PR-4 wires the comparison).
#[must_use]
pub(crate) fn derive_oauth_redirect_uri(public_url: &str, provider: OAuthProvider) -> String {
    // Trim trailing slash so we always emit a single `/` separator. The
    // boot-time validation (T2.8) ensures `public_url` is absolute with
    // a scheme; here we only normalize the slash.
    let base = public_url.trim_end_matches('/');
    format!("{base}/auth/oauth/{}/callback", provider.as_str())
}

/// `GET /api/v1/auth/oauth/{provider}` — start a Plane-A sign-in flow.
#[utoipa::path(
    get,
    path = "/auth/oauth/{provider}",
    tag = "auth",
    security(()),
    params(
        ("provider" = String, Path, description = "OAuth provider key (e.g. `github`, `google`)."),
    ),
    responses(
        (status = 200, description = "Authorize URL and signed state token; the client must redirect the user to `authorize_url`.", body = OAuthStartResponse),
        (status = 400, description = "Unknown or malformed provider key.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured or provider is not enabled.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state))]
pub async fn oauth_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<Json<OAuthStartResponse>> {
    let backend = backend(&state)?;
    let provider: OAuthProvider = provider.parse().map_err(ApiError::from)?;
    let redirect_uri = derive_oauth_redirect_uri(&state.public_url, provider);
    let start = backend
        .start_oauth(provider, &redirect_uri)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(OAuthStartResponse {
        authorize_url: start.authorize_url,
        state: start.state,
    }))
}

/// `GET /api/v1/auth/oauth/{provider}/callback` — exchange code, mint session.
#[utoipa::path(
    get,
    path = "/auth/oauth/{provider}/callback",
    tag = "auth",
    security(()),
    params(
        ("provider" = String, Path, description = "OAuth provider key returned by `/auth/oauth/{provider}`."),
        ("state" = String, Query, description = "Opaque signed state issued by `/auth/oauth/{provider}`."),
        ("code" = String, Query, description = "Authorization code returned by the provider."),
    ),
    responses(
        (status = 200, description = "Code exchanged; session and CSRF cookies issued.", body = LoginResponse),
        (status = 400, description = "Unknown provider or malformed callback parameters.", body = ProblemDetails),
        (status = 401, description = "State validation failed or token exchange rejected.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured or provider is not enabled.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, params))]
pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackParams>,
) -> ApiResult<axum::response::Response> {
    let backend = backend(&state)?;
    let provider: OAuthProvider = provider.parse().map_err(ApiError::from)?;
    let redirect_uri = derive_oauth_redirect_uri(&state.public_url, provider);
    let result = backend
        .complete_oauth(provider, &params.state, &params.code, &redirect_uri)
        .await;

    match result {
        Ok(completion) => {
            let resp = LoginResponse {
                user: completion.user,
                session_id: completion.session.id.clone(),
                csrf_token: completion.session.csrf_token.clone(),
            };
            let cleared = cookie_headers(&[
                session_cookie(&completion.session.id),
                csrf_cookie(&completion.session.csrf_token),
            ]);
            Ok((StatusCode::OK, cleared, Json(resp)).into_response())
        },
        Err(AuthError::NotImplemented(reason)) => Err(ApiError::ServiceUnavailable(format!(
            "oauth provider not configured: {reason}"
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Query string for the OAuth callback.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackParams {
    /// Opaque state token previously issued by `start_oauth`.
    pub state: String,
    /// Authorization code returned by the provider.
    pub code: String,
}

// ── Re-exports kept for the legacy AuthContext consumers ────────────────────

/// Extension type carried by the auth middleware (re-exported for handlers).
pub use crate::middleware::auth::AuthContext;
