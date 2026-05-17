//! Authentication endpoint handlers — Plane A.
//!
//! Each handler is a thin shim over [`crate::domain::auth::backend::AuthBackend`].
//! Validation lives in the backend; the HTTP layer extracts the request body,
//! dispatches, attaches `Set-Cookie` headers, and translates
//! [`crate::domain::auth::backend::AuthError`] into [`crate::error::ApiError`].
//!
//! Per ADR-0033 these endpoints belong to **Plane A** (host login). They
//! never touch the credential / Plane B OAuth state.

use std::sync::Arc;

use axum::{
    Json,
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
            LoginResponse, MfaChallengeResponse, MfaEnrollResponse, MfaVerifyRequest,
            MfaVerifyResponse, OAuthProvider, OAuthStartResponse, PasswordOutcome,
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

async fn principal_from_cookie(
    headers: &HeaderMap,
    backend: &Arc<dyn AuthBackend>,
) -> Result<Principal, ApiError> {
    let session_id = extract_session_id(headers)
        .ok_or_else(|| ApiError::Unauthorized("session cookie required".to_owned()))?;
    backend
        .get_principal_by_session(&session_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("session expired".to_owned()))
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
/// Requires a valid `nebula_session` cookie — extracted inline to avoid
/// applying the full auth middleware to the unauthenticated `/auth/*`
/// route group.
#[utoipa::path(
    post,
    path = "/auth/mfa/enroll",
    tag = "auth",
    security(()),
    responses(
        (status = 200, description = "Enrollment payload — display the otpauth URI as a QR code; the user must confirm via `/auth/mfa/verify`.", body = MfaEnrollResponse),
        (status = 401, description = "Session cookie is missing or expired.", body = ProblemDetails),
        (status = 403, description = "Authenticated principal is not a user (e.g. service account).", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, headers))]
pub async fn mfa_enroll(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<MfaEnrollResponse>> {
    let backend = backend(&state)?;
    let principal = principal_from_cookie(&headers, backend).await?;
    let user_id = user_id_from_principal(&principal)?;
    let enroll = backend
        .start_mfa_enrollment(&user_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(MfaEnrollResponse {
        otpauth_uri: enroll.otpauth_uri,
        secret_base32: enroll.secret_base32,
    }))
}

/// `POST /api/v1/auth/mfa/verify` — confirm enrollment OR complete a login.
///
/// If the request includes a `challenge_token`, the handler treats it as
/// the second-factor step of an in-flight login and mints a session on
/// success. Without the token, it confirms enrollment for the user
/// resolved via the session cookie.
#[utoipa::path(
    post,
    path = "/auth/mfa/verify",
    tag = "auth",
    security(()),
    request_body = MfaVerifyRequest,
    responses(
        (status = 200, description = "Either the second factor for an in-flight login (with `challenge_token`) succeeded — body is `LoginResponse` with session/CSRF cookies — OR the enrolled user confirmed their authenticator (without `challenge_token`) — body is `AckResponse`. The OpenAPI body advertises `oneOf` via the `MfaVerifyResponse` untagged enum so client generators receive both shapes.", body = MfaVerifyResponse),
        (status = 401, description = "Invalid TOTP code, missing session, or expired challenge token.", body = ProblemDetails),
        (status = 403, description = "Authenticated principal is not a user.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, headers, body))]
pub async fn mfa_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MfaVerifyRequest>,
) -> ApiResult<axum::response::Response> {
    let backend = backend(&state)?;
    if let Some(challenge_token) = body.challenge_token {
        let user = backend
            .verify_mfa(&challenge_token, &body.code)
            .await
            .map_err(ApiError::from)?;
        let response = mint_session_response(backend, user).await?;
        Ok(response)
    } else {
        let principal = principal_from_cookie(&headers, backend).await?;
        let user_id = user_id_from_principal(&principal)?;
        backend
            .confirm_mfa_enrollment(&user_id, &body.code)
            .await
            .map_err(ApiError::from)?;
        Ok((StatusCode::OK, Json(AckResponse::ok())).into_response())
    }
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
    let start = backend
        .start_oauth(provider)
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
    let result = backend
        .complete_oauth(provider, &params.state, &params.code)
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
