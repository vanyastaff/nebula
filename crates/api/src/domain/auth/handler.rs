//! Authentication endpoint handlers — Plane A.
//!
//! Each handler is a thin shim over [`crate::domain::auth::backend::AuthBackend`].
//! Validation lives in the backend; the HTTP layer extracts the request body,
//! dispatches, attaches `Set-Cookie` headers, and translates
//! [`crate::domain::auth::backend::AuthError`] into [`crate::error::ApiError`].
//!
//! Per auth plane separation these endpoints belong to **Plane A** (host login). They
//! never touch the credential / Plane B OAuth state.

use std::{net::IpAddr, sync::Arc};

use axum::{
    Extension, Json,
    extract::{OriginalUri, Path, Query, State, rejection::QueryRejection},
    http::{
        HeaderMap, HeaderValue, StatusCode, Uri,
        header::{COOKIE, HOST, SET_COOKIE},
        uri::Authority,
    },
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use nebula_core::Principal;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    domain::{
        auth::backend::{
            AuthBackend, AuthError, ForgotPasswordRequest, LoginRequest, LoginResponse,
            MfaChallengeResponse, MfaConfirmEnrollRequest, MfaEnrollResponse,
            MfaLoginCompleteRequest, OAuthCompletion, OAuthProvider, OAuthStartResponse,
            PasswordOutcome, ResetPasswordRequest, SESSION_COOKIE, SignupRequest, SignupResponse,
            UserProfile, VerifyEmailRequest, cleared_csrf_cookie, cleared_session_cookie,
            csrf_cookie, session_cookie,
        },
        shared::AckResponse,
    },
    error::{ApiError, ApiResult, ProblemDetails},
    middleware::auth::AuthMethod,
    state::AppState,
};

use crate::domain::auth::backend::provider::MFA_ENROLLMENT_REAUTH_TTL;

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

const OAUTH_TRANSACTION_COOKIE_PREFIX: &str = "__Host-nebula-oauth-";
const OAUTH_TRANSACTION_COOKIE_VERSION: &str = "v1";
const OAUTH_TRANSACTION_COOKIE_TTL_SECONDS: u64 = 600;
const OAUTH_TRANSACTION_COOKIE_LIMIT: usize = 8;
const OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN: &[u8] = b"nebula.oauth.transaction.name.v1\0";

/// Stateless browser binding for one OAuth authorization transaction.
///
/// The dynamic `__Host-` name lets independent flows coexist in one cookie
/// jar. The name contains a domain-separated hash of the provider/state tuple;
/// the versioned value binds that same tuple and is compared in constant time.
/// Origin scoping is provided by the `__Host-` cookie rules: `Secure`,
/// `Path=/`, and no `Domain` attribute.
struct OAuthTransactionBinding {
    name: String,
    value: String,
}

impl OAuthTransactionBinding {
    fn new(provider: OAuthProvider, state: &str) -> Self {
        let provider_name = provider.as_str();
        let mut name_hasher = Sha256::new();
        name_hasher.update(OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN);
        name_hasher.update((provider_name.len() as u64).to_be_bytes());
        name_hasher.update(provider_name.as_bytes());
        name_hasher.update((state.len() as u64).to_be_bytes());
        name_hasher.update(state.as_bytes());
        let name_hash = URL_SAFE_NO_PAD.encode(name_hasher.finalize());

        Self {
            name: format!("{OAUTH_TRANSACTION_COOKIE_PREFIX}{name_hash}"),
            value: format!("{OAUTH_TRANSACTION_COOKIE_VERSION}.{provider_name}.{state}"),
        }
    }

    fn set_cookie(&self) -> String {
        let expires = (Utc::now()
            + TimeDelta::seconds(OAUTH_TRANSACTION_COOKIE_TTL_SECONDS as i64))
        .format("%a, %d %b %Y %H:%M:%S GMT");
        format!(
            "{}={}; Path=/; Max-Age={OAUTH_TRANSACTION_COOKIE_TTL_SECONDS}; Expires={expires}; Secure; HttpOnly; SameSite=Lax",
            self.name, self.value,
        )
    }

    fn cleared_cookie(&self) -> String {
        format!(
            "{}=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Secure; HttpOnly; SameSite=Lax",
            self.name
        )
    }

    fn validate_request(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        let mut presented = None;
        for header in headers.get_all(COOKIE) {
            let raw = header.to_str().map_err(|_| AuthError::InvalidToken)?;
            for pair in raw.split(';') {
                let pair = pair.trim_start_matches([' ', '\t']);
                let Some((name, value)) = pair.split_once('=') else {
                    continue;
                };
                let name = name.trim_end_matches([' ', '\t']);
                if name != self.name {
                    continue;
                }
                if presented.replace(value).is_some() {
                    return Err(AuthError::InvalidToken);
                }
            }
        }

        let presented = presented.ok_or(AuthError::InvalidToken)?;
        if bool::from(self.value.as_bytes().ct_eq(presented.as_bytes())) {
            Ok(())
        } else {
            Err(AuthError::InvalidToken)
        }
    }
}

fn oauth_transaction_cookie_count(headers: &HeaderMap) -> Result<usize, AuthError> {
    let mut count = 0_usize;
    for header in headers.get_all(COOKIE) {
        let raw = header
            .to_str()
            .map_err(|_| AuthError::InvalidInput("OAuth request cookie header is invalid"))?;
        for pair in raw.split(';') {
            let pair = pair.trim_start_matches([' ', '\t']);
            let name = pair
                .split_once('=')
                .map_or(pair, |(name, _)| name)
                .trim_end_matches([' ', '\t']);
            if name.starts_with(OAUTH_TRANSACTION_COOKIE_PREFIX) {
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

fn validate_oauth_request_authority(
    public_url: &str,
    headers: &HeaderMap,
    request_uri: &Uri,
) -> Result<(), AuthError> {
    let public_url =
        crate::config::oauth::parse_public_oauth_base_url(public_url, !cfg!(debug_assertions))
            .map_err(|()| AuthError::Internal("OAuth callback base URL is invalid".to_owned()))?;
    let expected_host = public_url
        .host_str()
        .ok_or_else(|| AuthError::Internal("OAuth callback base URL is invalid".to_owned()))?;
    let expected_port = public_url
        .port_or_known_default()
        .ok_or_else(|| AuthError::Internal("OAuth callback base URL is invalid".to_owned()))?;
    let request_default_port = match public_url.scheme() {
        "https" => 443,
        "http" => 80,
        _ => {
            return Err(AuthError::Internal(
                "OAuth callback base URL is invalid".to_owned(),
            ));
        },
    };

    let mut authorities = Vec::new();
    let mut host_header_count = 0_usize;
    for header in headers.get_all(HOST) {
        host_header_count = host_header_count.saturating_add(1);
        let raw = header
            .to_str()
            .map_err(|_| AuthError::InvalidInput("OAuth request authority is invalid"))?;
        authorities.push(
            raw.parse::<Authority>()
                .map_err(|_| AuthError::InvalidInput("OAuth request authority is invalid"))?,
        );
    }
    if let Some(authority) = request_uri.authority() {
        authorities.push(authority.clone());
    }
    if host_header_count > 1
        || authorities.is_empty()
        || authorities.iter().any(|authority| {
            !authority_host_matches(authority.host(), expected_host)
                || authority.port_u16().unwrap_or(request_default_port) != expected_port
        })
    {
        return Err(AuthError::InvalidInput(
            "OAuth request authority does not match the public URL",
        ));
    }
    Ok(())
}

fn authority_host_matches(request_host: &str, expected_host: &str) -> bool {
    let request_host = request_host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(request_host);
    let expected_host = expected_host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(expected_host);
    match (
        request_host.parse::<IpAddr>(),
        expected_host.parse::<IpAddr>(),
    ) {
        (Ok(request), Ok(expected)) => request == expected,
        (Err(_), Err(_)) => request_host.eq_ignore_ascii_case(expected_host),
        _ => false,
    }
}

fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(COOKIE)?.to_str().ok()?;
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

fn user_id_from_fresh_mfa_session(auth: &AuthContext) -> Result<String, ApiError> {
    let AuthMethod::Session { authenticated_at } = &auth.auth_method else {
        return Err(ApiError::Forbidden(
            "MFA enrollment requires session authentication".to_owned(),
        ));
    };
    let maximum_age = TimeDelta::from_std(MFA_ENROLLMENT_REAUTH_TTL)
        .map_err(|_| ApiError::Internal("invalid MFA reauthentication window".to_owned()))?;
    let age = Utc::now().signed_duration_since(*authenticated_at);
    if age < TimeDelta::zero() || age > maximum_age {
        return Err(ApiError::Unauthorized(
            "fresh session authentication required".to_owned(),
        ));
    }
    user_id_from_principal(&auth.principal)
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
        (status = 202, description = "Password OK but MFA verification is required; submit the challenge token to `/auth/login/mfa`.", body = MfaChallengeResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Invalid credentials, locked account, or expired session.", body = ProblemDetails),
        (status = 500, description = "Session creation failed after authentication.", body = ProblemDetails),
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
        .map_err(|error| match error {
            // The user existed when the first factor completed. A concurrent
            // deletion must invalidate the login continuation, not expose an
            // internal identity lookup as a resource-shaped 404.
            AuthError::UserNotFound => ApiError::from(AuthError::InvalidToken),
            other => ApiError::from(other),
        })?;
    let resp = LoginResponse {
        user,
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
    let cleared = cookie_headers(&[cleared_session_cookie(), cleared_csrf_cookie()]);
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
/// is read from the `AuthContext` populated by `auth_middleware`. Only a
/// session created by primary authentication within the bounded freshness
/// window is accepted; bearer and API-key authority is deliberately denied.
#[utoipa::path(
    post,
    path = "/auth/mfa/enroll",
    tag = "auth",
    security(("session_cookie" = [], "csrf" = [])),
    responses(
        (status = 200, description = "Enrollment payload — display the otpauth URI as a QR code; the user must confirm via `/auth/mfa/verify`.", body = MfaEnrollResponse),
        (status = 401, description = "A fresh session authentication is required.", body = ProblemDetails),
        (status = 403, description = "Only a user authenticated by the host-bound session cookie may enroll MFA.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, auth))]
pub async fn mfa_enroll(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<MfaEnrollResponse>> {
    let backend = backend(&state)?;
    let user_id = user_id_from_fresh_mfa_session(&auth)?;
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
    security(("session_cookie" = [], "csrf" = [])),
    request_body = MfaConfirmEnrollRequest,
    responses(
        (status = 200, description = "Enrollment confirmed; the account now requires MFA at login.", body = AckResponse),
        (status = 401, description = "Invalid TOTP code, unavailable candidate, or stale session authentication.", body = ProblemDetails),
        (status = 403, description = "Only a user authenticated by the host-bound session cookie may confirm MFA enrollment.", body = ProblemDetails),
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
    let user_id = user_id_from_fresh_mfa_session(&auth)?;
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
/// `/auth/*` sub-router. The `challenge_token` issued by password login or an
/// OAuth callback is the sole authority.
#[utoipa::path(
    post,
    path = "/auth/login/mfa",
    tag = "auth",
    security(()),
    request_body = MfaLoginCompleteRequest,
    responses(
        (status = 200, description = "Second factor accepted; session and CSRF cookies issued.", body = LoginResponse),
        (status = 401, description = "Invalid TOTP code or expired challenge token.", body = ProblemDetails),
        (status = 500, description = "MFA verification or session creation failed internally.", body = ProblemDetails),
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

/// Derive the canonical Plane-A OAuth `redirect_uri` per ADR-0085 D-3.
///
/// The Plane-A auth router is nested under `/api/v1/` in
/// `crates/api/src/domain/mod.rs:170`, so the **full** callback URL
/// the IdP redirects back to is
/// `{public_url}/api/v1/auth/oauth/{provider}/callback`.
///
/// Shared by `oauth_start` and `oauth_callback` so the value persisted
/// in the OAuth state row at start_oauth time matches the value
/// re-derived at callback time (`public_url_changed_mid_flow` defense).
pub(crate) fn derive_oauth_redirect_uri(
    public_url: &str,
    provider: OAuthProvider,
) -> Result<String, AuthError> {
    let mut url =
        crate::config::oauth::parse_public_oauth_base_url(public_url, !cfg!(debug_assertions))
            .map_err(|()| AuthError::Internal("OAuth callback base URL is invalid".to_owned()))?;
    url.path_segments_mut()
        .map_err(|()| AuthError::Internal("OAuth callback base URL is invalid".to_owned()))?
        .pop_if_empty()
        .push("api")
        .push("v1")
        .push("auth")
        .push("oauth")
        .push(provider.as_str())
        .push("callback");
    Ok(url.into())
}

/// `GET /api/v1/auth/oauth/{provider}` — start a Plane-A sign-in flow.
#[utoipa::path(
    get,
    path = "/auth/oauth/{provider}",
    tag = "auth",
    security(()),
    params(
        ("provider" = inline(OAuthProvider), Path, description = "Closed Plane-A OAuth provider key."),
    ),
    responses(
        (status = 200, description = "Authorize URL and opaque one-time state; the client must redirect the user to `authorize_url`.", body = OAuthStartResponse),
        (status = 400, description = "Unknown provider key or request authority does not match the configured public URL.", body = ProblemDetails),
        (status = 429, description = "OAuth state capacity or request rate limit reached.", body = ProblemDetails),
        (status = 500, description = "Server-side OAuth composition or persistence failure.", body = ProblemDetails),
        (status = 502, description = "Provider discovery failed.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured or provider is not enabled.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, provider, request_uri, headers))]
pub async fn oauth_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    OriginalUri(request_uri): OriginalUri,
    headers: HeaderMap,
) -> ApiResult<axum::response::Response> {
    let provider: OAuthProvider = provider.parse().map_err(ApiError::from)?;
    validate_oauth_request_authority(&state.public_url, &headers, &request_uri)
        .map_err(ApiError::from)?;
    if oauth_transaction_cookie_count(&headers).map_err(ApiError::from)?
        >= OAUTH_TRANSACTION_COOKIE_LIMIT
    {
        return Err(ApiError::RateLimitExceeded);
    }
    let redirect_uri = derive_oauth_redirect_uri(&state.public_url, provider)?;
    let backend = backend(&state)?;
    let start = backend
        .start_oauth(provider, &redirect_uri)
        .await
        .map_err(ApiError::from)?;
    let transaction = OAuthTransactionBinding::new(provider, &start.state);
    let transaction_cookie = HeaderValue::from_str(&transaction.set_cookie()).map_err(|_| {
        ApiError::Internal("OAuth transaction cookie construction failed".to_owned())
    })?;
    let mut response = Json(OAuthStartResponse {
        authorize_url: start.authorize_url,
        state: start.state,
    })
    .into_response();
    response
        .headers_mut()
        .append(SET_COOKIE, transaction_cookie);
    Ok(response)
}

/// `GET /api/v1/auth/oauth/{provider}/callback` — complete the provider first factor.
#[utoipa::path(
    get,
    path = "/auth/oauth/{provider}/callback",
    tag = "auth",
    security(()),
    params(
        ("provider" = inline(OAuthProvider), Path, description = "Closed provider key returned by `/auth/oauth/{provider}`."),
        ("state" = String, Query, description = "Opaque one-time state issued by `/auth/oauth/{provider}`."),
        ("code" = Option<String>, Query, description = "Authorization code returned by the provider; exactly one of `code` or `error` is required."),
        ("error" = Option<String>, Query, description = "Provider error identifier; exactly one of `code` or `error` is required. Provider descriptions and URIs are ignored."),
    ),
    responses(
        (status = 200, description = "Code exchanged; session and CSRF cookies issued.", body = LoginResponse),
        (status = 202, description = "Provider first factor accepted, but local Nebula MFA is required; no session or CSRF cookie is issued.", body = MfaChallengeResponse),
        (status = 400, description = "Unknown provider, malformed callback parameters, or request authority mismatch.", body = ProblemDetails),
        (status = 401, description = "Authorization was denied, or the one-time OAuth state/browser binding is invalid, expired, or already consumed.", body = ProblemDetails),
        (status = 403, description = "Provider identity did not supply a verified email.", body = ProblemDetails),
        (status = 409, description = "The verified email belongs to an existing account that requires explicit authenticated linking.", body = ProblemDetails),
        (status = 500, description = "Server-side OAuth composition or persistence failure.", body = ProblemDetails),
        (status = 502, description = "OAuth provider request or response failed.", body = ProblemDetails),
        (status = 503, description = "Auth backend is not configured or provider is not enabled.", body = ProblemDetails),
    ),
)]
#[tracing::instrument(level = "info", skip(state, provider, request_uri, headers, params))]
pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    OriginalUri(request_uri): OriginalUri,
    headers: HeaderMap,
    params: Result<Query<OAuthCallbackParams>, QueryRejection>,
) -> ApiResult<axum::response::Response> {
    let Query(params) = params.map_err(|_| {
        ApiError::from(AuthError::InvalidInput("OAuth callback query is malformed"))
    })?;
    let callback = validate_oauth_callback_params(&params).map_err(ApiError::from)?;
    let provider: OAuthProvider = provider.parse().map_err(ApiError::from)?;
    validate_oauth_request_authority(&state.public_url, &headers, &request_uri)
        .map_err(ApiError::from)?;
    let transaction = OAuthTransactionBinding::new(provider, &params.state);
    transaction
        .validate_request(&headers)
        .map_err(ApiError::from)?;
    let clear_cookie = HeaderValue::from_str(&transaction.cleared_cookie()).map_err(|_| {
        ApiError::Internal("OAuth transaction cookie construction failed".to_owned())
    })?;

    // Once the exact browser binding has been accepted, the transaction is
    // terminal from the browser's perspective. Clear its cookie for every
    // backend outcome, including composition and upstream failures.
    let result: Result<Option<_>, ApiError> = async {
        let redirect_uri = derive_oauth_redirect_uri(&state.public_url, provider)?;
        let backend = backend(&state)?;
        match callback {
            ValidatedOAuthCallback::AuthorizationCode(code) => backend
                .complete_oauth(provider, &params.state, code, &redirect_uri)
                .await
                .map(Some)
                .map_err(ApiError::from),
            ValidatedOAuthCallback::ProviderError => {
                backend
                    .cancel_oauth(provider, &params.state, &redirect_uri)
                    .await
                    .map_err(ApiError::from)?;
                Ok(None)
            },
        }
    }
    .await;

    let mut response = match result {
        Ok(Some(OAuthCompletion::SessionCreated { user, session })) => {
            let resp = LoginResponse {
                user,
                csrf_token: session.csrf_token.clone(),
            };
            let cleared = cookie_headers(&[
                session_cookie(&session.id),
                csrf_cookie(&session.csrf_token),
            ]);
            (StatusCode::OK, cleared, Json(resp)).into_response()
        },
        Ok(Some(OAuthCompletion::MfaRequired { challenge_token })) => (
            StatusCode::ACCEPTED,
            Json(MfaChallengeResponse {
                mfa_required: true,
                challenge_token,
            }),
        )
            .into_response(),
        Ok(None) => ApiError::from(AuthError::OAuthDenied).into_response(),
        Err(error) => error.into_response(),
    };
    response.headers_mut().append(SET_COOKIE, clear_cookie);
    Ok(response)
}

/// Query string for the OAuth callback.
#[derive(Deserialize)]
#[non_exhaustive]
pub struct OAuthCallbackParams {
    /// Opaque state token previously issued by `start_oauth`.
    pub state: String,
    /// Authorization code returned by the provider, mutually exclusive with
    /// `error`.
    pub code: Option<String>,
    /// Provider error identifier, mutually exclusive with `code`. Its value
    /// is validated for shape but never surfaced or logged.
    pub error: Option<String>,
}

impl std::fmt::Debug for OAuthCallbackParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthCallbackParams")
            .field("state", &"[redacted]")
            .field("code", &"[redacted]")
            .field("error", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy)]
enum ValidatedOAuthCallback<'a> {
    AuthorizationCode(&'a str),
    ProviderError,
}

fn validate_oauth_callback_params(
    params: &OAuthCallbackParams,
) -> Result<ValidatedOAuthCallback<'_>, AuthError> {
    let state_valid = params.state.len() == 43
        && params
            .state
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !state_valid {
        return Err(AuthError::InvalidInput("OAuth callback parameters invalid"));
    }
    match (params.code.as_deref(), params.error.as_deref()) {
        (Some(code), None)
            if !code.is_empty()
                && code.len() <= 4096
                && code.bytes().all(|byte| byte.is_ascii_graphic()) =>
        {
            Ok(ValidatedOAuthCallback::AuthorizationCode(code))
        },
        (None, Some(error))
            if !error.is_empty()
                && error.len() <= 256
                && error.bytes().all(|byte| byte.is_ascii_graphic()) =>
        {
            Ok(ValidatedOAuthCallback::ProviderError)
        },
        _ => Err(AuthError::InvalidInput("OAuth callback parameters invalid")),
    }
}

// ── Re-exports kept for the legacy AuthContext consumers ────────────────────

/// Extension type carried by the auth middleware (re-exported for handlers).
pub use crate::middleware::auth::AuthContext;

#[cfg(test)]
mod mfa_authority_tests {
    use chrono::{TimeDelta, Utc};
    use nebula_core::{Principal, UserId};

    use super::user_id_from_fresh_mfa_session;
    use crate::{
        access::Grant,
        error::ApiError,
        middleware::auth::{AuthContext, AuthMethod},
    };

    fn auth(auth_method: AuthMethod) -> AuthContext {
        AuthContext {
            principal: Principal::User(UserId::new()),
            auth_method,
            grant: Grant::UnrestrictedIdentity,
        }
    }

    #[test]
    fn fresh_session_is_the_only_mfa_enrollment_authority() {
        let fresh = auth(AuthMethod::Session {
            authenticated_at: Utc::now(),
        });
        assert!(user_id_from_fresh_mfa_session(&fresh).is_ok());

        for method in [AuthMethod::Pat, AuthMethod::ApiKey, AuthMethod::Jwt] {
            let error = user_id_from_fresh_mfa_session(&auth(method))
                .expect_err("header authority must not enroll MFA");
            assert!(matches!(error, ApiError::Forbidden(_)));
        }
    }

    #[test]
    fn stale_or_future_session_requires_primary_reauthentication() {
        for authenticated_at in [
            Utc::now() - TimeDelta::minutes(11),
            Utc::now() + TimeDelta::minutes(1),
        ] {
            let error =
                user_id_from_fresh_mfa_session(&auth(AuthMethod::Session { authenticated_at }))
                    .expect_err("out-of-window session must fail closed");
            assert!(matches!(error, ApiError::Unauthorized(_)));
        }
    }
}

#[cfg(test)]
mod oauth_debug_tests {
    use super::{OAuthCallbackParams, derive_oauth_redirect_uri, validate_oauth_callback_params};
    use crate::domain::auth::backend::{AuthError, OAuthProvider};

    #[test]
    fn oauth_callback_params_debug_redacts_code_and_state() {
        let params = OAuthCallbackParams {
            state: "STATE_CANARY-438d".to_owned(),
            code: Some("CODE_CANARY-e9d3".to_owned()),
            error: Some("ERROR_CANARY-b150".to_owned()),
        };

        let debug = format!("{params:?}");
        assert!(!debug.contains("STATE_CANARY-438d"));
        assert!(!debug.contains("CODE_CANARY-e9d3"));
        assert!(!debug.contains("ERROR_CANARY-b150"));
    }

    #[test]
    fn redirect_derivation_revalidates_custom_app_state_origin() {
        let redirect = derive_oauth_redirect_uri("https://nebula.example/", OAuthProvider::GitHub)
            .expect("canonical public origin must derive a callback");
        assert_eq!(
            redirect,
            "https://nebula.example/api/v1/auth/oauth/github/callback"
        );
        for base in [
            "https://nebula.example/nebula",
            "https://nebula.example/nebula/",
        ] {
            assert_eq!(
                derive_oauth_redirect_uri(base, OAuthProvider::GitHub)
                    .expect("canonical mount prefix must derive a callback"),
                "https://nebula.example/nebula/api/v1/auth/oauth/github/callback"
            );
        }

        const CANARY: &str = "PUBLIC_ORIGIN_CANARY_DO_NOT_ECHO";
        for invalid in [
            format!("https://user:{CANARY}@nebula.example/"),
            format!("https://nebula.example/?secret={CANARY}"),
            format!("https://nebula.example/base//{CANARY}"),
            format!("https://nebula.example/base/%2F{CANARY}"),
            "http://nebula.example/".to_owned(),
        ] {
            let error = derive_oauth_redirect_uri(&invalid, OAuthProvider::GitHub)
                .expect_err("non-canonical AppState origin must fail closed");
            assert!(matches!(&error, AuthError::Internal(_)));
            assert!(!error.to_string().contains(CANARY));
            assert!(!format!("{error:?}").contains(CANARY));
        }
    }

    #[test]
    fn callback_parameter_validation_is_bounded_and_secret_free() {
        let valid = OAuthCallbackParams {
            state: "A".repeat(43),
            code: Some("visible-code_~.-".to_owned()),
            error: None,
        };
        validate_oauth_callback_params(&valid).expect("bounded visible callback is valid");
        let provider_error = OAuthCallbackParams {
            state: "A".repeat(43),
            code: None,
            error: Some("access_denied".to_owned()),
        };
        validate_oauth_callback_params(&provider_error)
            .expect("bounded provider error callback is valid");

        const CANARY: &str = "CALLBACK_INPUT_CANARY_DO_NOT_ECHO";
        for invalid in [
            OAuthCallbackParams {
                state: CANARY.to_owned(),
                code: Some("code".to_owned()),
                error: None,
            },
            OAuthCallbackParams {
                state: "A".repeat(43),
                code: Some(String::new()),
                error: None,
            },
            OAuthCallbackParams {
                state: "A".repeat(43),
                code: Some(format!("{CANARY}\n")),
                error: None,
            },
            OAuthCallbackParams {
                state: "A".repeat(43),
                code: Some("x".repeat(4097)),
                error: None,
            },
            OAuthCallbackParams {
                state: "A".repeat(43),
                code: Some("code".to_owned()),
                error: Some(CANARY.to_owned()),
            },
            OAuthCallbackParams {
                state: "A".repeat(43),
                code: None,
                error: Some("x".repeat(257)),
            },
        ] {
            let error = match validate_oauth_callback_params(&invalid) {
                Ok(_) => panic!("invalid callback parameters must fail closed"),
                Err(error) => error,
            };
            assert!(!error.to_string().contains(CANARY));
            assert!(!format!("{error:?}").contains(CANARY));
        }
    }
}

#[cfg(test)]
mod oauth_transaction_tests {
    use std::{
        collections::{HashMap, HashSet},
        net::{IpAddr, Ipv4Addr},
        sync::Arc,
    };

    use axum::{
        Json,
        body::to_bytes,
        extract::{OriginalUri, Path, Query, State},
        http::{
            HeaderMap, HeaderValue, StatusCode, Uri,
            header::{COOKIE, HOST, SET_COOKIE},
        },
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use secrecy::SecretString;
    use sha2::{Digest as _, Sha256};

    use super::{
        OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN, OAUTH_TRANSACTION_COOKIE_LIMIT,
        OAUTH_TRANSACTION_COOKIE_PREFIX, OAuthCallbackParams, OAuthTransactionBinding,
        mfa_complete_login, oauth_callback, oauth_start, oauth_transaction_cookie_count,
        validate_oauth_request_authority,
    };
    use crate::{
        ApiConfig, AppState, OAuthIdentityRuntime,
        config::{OAuthProviderConfig, OAuthProvidersConfig},
        domain::auth::backend::{
            AuthBackend, CSRF_COOKIE, InMemoryAuthBackend, MfaLoginCompleteRequest, OAuthProvider,
            SESSION_COOKIE, mfa,
        },
        error::ApiError,
        transport::oauth::{
            OAuthTestProviderProfile,
            test_support::{TestResponse, TlsFixture},
        },
    };

    const PUBLIC_URL: &str = "https://nebula.example/nebula";
    const CALLBACK_PATH: &str = "/api/v1/auth/oauth/github/callback";
    const TEST_DNS_ANSWER: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

    struct StartedFlow {
        state: String,
        cookie_pair: String,
        set_cookie: String,
    }

    fn request_headers(authority: &str, cookie_pairs: &[String]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_str(authority).expect("test authority is a valid header value"),
        );
        for pair in cookie_pairs {
            headers.append(
                COOKIE,
                HeaderValue::from_str(pair).expect("test cookie pair is a valid header value"),
            );
        }
        headers
    }

    fn manual_config(
        fixture: &TlsFixture,
    ) -> (
        OAuthProvidersConfig,
        HashMap<OAuthProvider, OAuthTestProviderProfile>,
    ) {
        (
            OAuthProvidersConfig {
                providers: HashMap::from([(
                    OAuthProvider::GitHub,
                    OAuthProviderConfig {
                        client_id: SecretString::new("test-client".to_owned().into_boxed_str()),
                        client_secret: SecretString::new("test-secret".to_owned().into_boxed_str()),
                    },
                )]),
            },
            HashMap::from([(
                OAuthProvider::GitHub,
                OAuthTestProviderProfile::manual(
                    "https://accounts.example.com/authorize".to_owned(),
                    fixture.endpoint("/token"),
                    fixture.endpoint("/userinfo"),
                    Some(fixture.endpoint("/emails")),
                    vec!["user:email".to_owned()],
                ),
            )]),
        )
    }

    fn state_and_backend_with_oauth(fixture: &TlsFixture) -> (AppState, Arc<InMemoryAuthBackend>) {
        let (config, profiles) = manual_config(fixture);
        let runtime = OAuthIdentityRuntime::from_config_for_test(
            config,
            profiles,
            fixture.trust_anchor(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            vec![TEST_DNS_ANSWER],
        )
        .expect("test OAuth runtime must build")
        .expect("configured test provider must enable OAuth");
        let backend = Arc::new(InMemoryAuthBackend::new().with_oauth_runtime(Arc::new(runtime)));
        let auth_backend: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
        let state = AppState::in_memory(ApiConfig::for_test().jwt_secret)
            .with_auth_backend(auth_backend)
            .with_public_url(PUBLIC_URL);
        (state, backend)
    }

    fn state_with_oauth(fixture: &TlsFixture) -> AppState {
        state_and_backend_with_oauth(fixture).0
    }

    async fn start_flow(
        state: AppState,
        authority: &str,
        cookies: &[String],
    ) -> Result<StartedFlow, ApiError> {
        let response = oauth_start(
            State(state),
            Path("github".to_owned()),
            OriginalUri(
                "/api/v1/auth/oauth/github"
                    .parse()
                    .expect("test start URI is valid"),
            ),
            request_headers(authority, cookies),
        )
        .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let set_cookie = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .expect("transaction Set-Cookie is visible ASCII")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(set_cookie.len(), 1, "start sets exactly one cookie");
        let set_cookie = set_cookie
            .into_iter()
            .next()
            .expect("one transaction cookie was asserted");
        let cookie_pair = set_cookie
            .split(';')
            .next()
            .expect("Set-Cookie begins with a cookie pair")
            .to_owned();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read OAuth start response body");
        let body: serde_json::Value =
            serde_json::from_slice(&body).expect("OAuth start response is JSON");
        let state = body["state"]
            .as_str()
            .expect("OAuth start response carries state")
            .to_owned();
        Ok(StartedFlow {
            state,
            cookie_pair,
            set_cookie,
        })
    }

    fn callback_params(state: &str) -> OAuthCallbackParams {
        OAuthCallbackParams {
            state: state.to_owned(),
            code: Some("visible-code".to_owned()),
            error: None,
        }
    }

    async fn callback(
        state: AppState,
        provider: &str,
        flow_state: &str,
        authority: &str,
        cookies: &[String],
    ) -> Result<axum::response::Response, ApiError> {
        let uri: Uri =
            format!("/api/v1/auth/oauth/{provider}/callback?state={flow_state}&code=visible-code")
                .parse()
                .expect("test callback URI is valid");
        oauth_callback(
            State(state),
            Path(provider.to_owned()),
            OriginalUri(uri),
            request_headers(authority, cookies),
            Ok(Query(callback_params(flow_state))),
        )
        .await
    }

    async fn provider_error_callback(
        state: AppState,
        flow_state: &str,
        authority: &str,
        cookies: &[String],
        include_code: bool,
        provider_error: &str,
    ) -> Result<axum::response::Response, ApiError> {
        let uri: Uri = format!(
            "{CALLBACK_PATH}?state={flow_state}&error={provider_error}&error_description=IGNORED_PROVIDER_TEXT"
        )
        .parse()
        .expect("test provider-error callback URI is valid");
        oauth_callback(
            State(state),
            Path("github".to_owned()),
            OriginalUri(uri),
            request_headers(authority, cookies),
            Ok(Query(OAuthCallbackParams {
                state: flow_state.to_owned(),
                code: include_code.then(|| "unexpected-code".to_owned()),
                error: Some(provider_error.to_owned()),
            })),
        )
        .await
    }

    fn assert_status(error: &ApiError, expected: StatusCode) {
        assert_eq!(error.to_problem_details().0, expected);
    }

    fn assert_transaction_cookie_security(flow: &StartedFlow) {
        let (name, value) = flow
            .cookie_pair
            .split_once('=')
            .expect("transaction cookie has a name and value");
        let encoded_hash = name
            .strip_prefix(OAUTH_TRANSACTION_COOKIE_PREFIX)
            .expect("transaction cookie uses the canonical __Host prefix");
        let decoded_hash = URL_SAFE_NO_PAD
            .decode(encoded_hash)
            .expect("cookie name suffix is base64url");
        let provider_name = OAuthProvider::GitHub.as_str();
        let mut expected_hash = Sha256::new();
        expected_hash.update(OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN);
        expected_hash.update((provider_name.len() as u64).to_be_bytes());
        expected_hash.update(provider_name.as_bytes());
        expected_hash.update((flow.state.len() as u64).to_be_bytes());
        expected_hash.update(flow.state.as_bytes());
        assert_eq!(
            decoded_hash,
            expected_hash.finalize().as_slice(),
            "cookie name hashes the explicit domain and length-delimited tuple"
        );
        assert_eq!(encoded_hash.len(), 43, "SHA-256 base64url is untruncated");
        assert_eq!(value, format!("v1.github.{}", flow.state));

        let attributes = flow.set_cookie.split("; ").collect::<Vec<_>>();
        assert_eq!(attributes[0], flow.cookie_pair);
        assert_eq!(attributes[1], "Path=/");
        assert_eq!(attributes[2], "Max-Age=600");
        assert!(attributes[3].starts_with("Expires="));
        assert_eq!(attributes[4..], ["Secure", "HttpOnly", "SameSite=Lax"]);
        assert!(!flow.set_cookie.contains("Domain="));
    }

    #[test]
    fn transaction_cookie_parser_is_unique_exact_and_conservatively_bounded() {
        let binding = OAuthTransactionBinding::new(OAuthProvider::GitHub, &"A".repeat(43));
        let exact = format!("{}={}", binding.name, binding.value);

        let correct = request_headers("nebula.example", std::slice::from_ref(&exact));
        binding
            .validate_request(&correct)
            .expect("one exact binding is accepted");

        for invalid in [
            request_headers("nebula.example", &[]),
            request_headers(
                "nebula.example",
                &[format!("{}={}x", binding.name, binding.value)],
            ),
            request_headers(
                "nebula.example",
                &[format!("{}={} ", binding.name, binding.value)],
            ),
            request_headers("nebula.example", &[exact.clone(), exact]),
        ] {
            assert!(
                binding.validate_request(&invalid).is_err(),
                "missing, mismatched, trailing-byte, and duplicate bindings fail closed"
            );
        }

        let mut bounded = HeaderMap::new();
        bounded.append(
            COOKIE,
            HeaderValue::from_static(
                "__Host-nebula-oauth-malformed; unrelated=1; __Host-nebula-oauth-other=x",
            ),
        );
        assert_eq!(
            oauth_transaction_cookie_count(&bounded).expect("ASCII Cookie header is countable"),
            2,
            "canonical-prefix entries count even without '='"
        );
    }

    #[test]
    fn request_authority_matches_canonical_host_and_normalized_default_port() {
        let path: Uri = CALLBACK_PATH.parse().expect("callback path URI is valid");
        for authority in ["nebula.example", "NEBULA.EXAMPLE:443"] {
            validate_oauth_request_authority(PUBLIC_URL, &request_headers(authority, &[]), &path)
                .expect("canonical host with implicit or explicit default port is accepted");
        }
        validate_oauth_request_authority(
            PUBLIC_URL,
            &HeaderMap::new(),
            &"https://nebula.example/api/v1/auth/oauth/github/callback"
                .parse()
                .expect("absolute callback URI is valid"),
        )
        .expect("HTTP/2-style URI authority is accepted without Host");

        for headers in [
            HeaderMap::new(),
            request_headers("alias.example", &[]),
            request_headers("nebula.example:8443", &[]),
        ] {
            assert!(
                validate_oauth_request_authority(PUBLIC_URL, &headers, &path).is_err(),
                "missing, alias, and wrong-port authorities fail closed"
            );
        }

        let mut duplicate = request_headers("nebula.example", &[]);
        duplicate.append(HOST, HeaderValue::from_static("nebula.example"));
        assert!(validate_oauth_request_authority(PUBLIC_URL, &duplicate, &path).is_err());
    }

    #[tokio::test]
    async fn callback_binding_blocks_before_egress_then_correct_flow_succeeds_and_clears() {
        let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
            "/userinfo" => TestResponse::json(r#"{"id":101}"#),
            "/emails" => TestResponse::json(
                r#"[{"email":"bound@example.com","primary":true,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let state = state_with_oauth(&fixture);
        let flow = start_flow(state.clone(), "nebula.example", &[])
            .await
            .expect("OAuth start succeeds");
        assert_transaction_cookie_security(&flow);

        let (name, _) = flow
            .cookie_pair
            .split_once('=')
            .expect("started cookie has a name");
        for attempt in [
            callback(state.clone(), "github", &flow.state, "nebula.example", &[]).await,
            callback(
                state.clone(),
                "github",
                &flow.state,
                "nebula.example",
                &[format!("{name}=wrong")],
            )
            .await,
            callback(
                state.clone(),
                "github",
                &flow.state,
                "nebula.example",
                &[flow.cookie_pair.clone(), flow.cookie_pair.clone()],
            )
            .await,
            callback(
                state.clone(),
                "google",
                &flow.state,
                "nebula.example",
                std::slice::from_ref(&flow.cookie_pair),
            )
            .await,
        ] {
            let error = attempt.expect_err("invalid browser binding must fail");
            assert_status(&error, StatusCode::UNAUTHORIZED);
        }
        let authority_error = callback(
            state.clone(),
            "github",
            &flow.state,
            "nebula.example:8443",
            std::slice::from_ref(&flow.cookie_pair),
        )
        .await
        .expect_err("wrong callback port must fail before binding/backend");
        assert_status(&authority_error, StatusCode::BAD_REQUEST);
        assert!(
            fixture.requests().is_empty(),
            "rejected bindings and authority mismatch cannot reach provider egress"
        );

        let response = callback(
            state,
            "github",
            &flow.state,
            "nebula.example:443",
            std::slice::from_ref(&flow.cookie_pair),
        )
        .await
        .expect("correct browser binding reaches the backend");
        assert_eq!(response.status(), StatusCode::OK);
        let expected_clear =
            OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
        let cleared = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("response cookie is ASCII"))
            .any(|cookie| cookie == expected_clear.as_str());
        assert!(cleared);
        assert!(expected_clear.contains("Max-Age=0"));
        assert!(expected_clear.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert_eq!(
            fixture.requests().len(),
            3,
            "token + userinfo + verified email exactly once"
        );
    }

    #[tokio::test]
    async fn linked_oauth_user_with_local_mfa_gets_only_a_one_time_challenge() {
        let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
            "/userinfo" => {
                TestResponse::json(r#"{"id":303,"amr":["mfa"],"acr":"provider-high-assurance"}"#)
            },
            "/emails" => TestResponse::json(
                r#"[{"email":"oauth-mfa@example.com","primary":true,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let (state, backend) = state_and_backend_with_oauth(&fixture);

        // First login creates the OAuth-only local user and its initial
        // session atomically. Local MFA is enrolled only after that.
        let first = start_flow(state.clone(), "nebula.example", &[])
            .await
            .expect("first OAuth start succeeds");
        let first_response = callback(
            state.clone(),
            "github",
            &first.state,
            "nebula.example",
            std::slice::from_ref(&first.cookie_pair),
        )
        .await
        .expect("first OAuth callback succeeds");
        assert_eq!(first_response.status(), StatusCode::OK);
        let first_body = to_bytes(first_response.into_body(), usize::MAX)
            .await
            .expect("read first OAuth response");
        let first_body: serde_json::Value =
            serde_json::from_slice(&first_body).expect("first OAuth response is JSON");
        let user_id = first_body["user"]["user_id"]
            .as_str()
            .expect("first OAuth response carries user id");
        let enrollment = backend
            .start_mfa_enrollment(user_id)
            .await
            .expect("start local MFA enrollment");
        let enrollment_code =
            mfa::current_code(&enrollment.secret_base32).expect("current enrollment code");
        backend
            .confirm_mfa_enrollment(user_id, &enrollment_code)
            .await
            .expect("confirm local MFA enrollment");

        // A provider claim that it performed MFA never substitutes for the
        // independently enrolled Nebula factor.
        let second = start_flow(state.clone(), "nebula.example", &[])
            .await
            .expect("second OAuth start succeeds");
        let response = callback(
            state.clone(),
            "github",
            &second.state,
            "nebula.example",
            std::slice::from_ref(&second.cookie_pair),
        )
        .await
        .expect("linked OAuth callback reaches local MFA gate");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("response cookie is ASCII").to_owned())
            .collect::<Vec<_>>();
        let expected_clear =
            OAuthTransactionBinding::new(OAuthProvider::GitHub, &second.state).cleared_cookie();
        assert_eq!(cookies, [expected_clear]);
        assert!(!cookies.iter().any(|cookie| {
            cookie.starts_with(&format!("{SESSION_COOKIE}="))
                || cookie.starts_with(&format!("{CSRF_COOKIE}="))
        }));
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read MFA-required OAuth response");
        let body: serde_json::Value =
            serde_json::from_slice(&body).expect("MFA-required response is JSON");
        assert_eq!(body["mfa_required"], true);
        assert!(body.get("session_id").is_none());
        assert!(body.get("csrf_token").is_none());
        let challenge_token = body["challenge_token"]
            .as_str()
            .expect("MFA-required response carries a challenge")
            .to_owned();

        let code = mfa::current_code(&enrollment.secret_base32).expect("current login MFA code");
        let completed = mfa_complete_login(
            State(state.clone()),
            Json(MfaLoginCompleteRequest {
                code: code.clone(),
                challenge_token: challenge_token.clone(),
            }),
        )
        .await
        .expect("local MFA completes OAuth login");
        assert_eq!(completed.status(), StatusCode::OK);
        let completed_cookies = completed
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("session cookie is ASCII"))
            .collect::<Vec<_>>();
        assert!(
            completed_cookies
                .iter()
                .any(|cookie| cookie.starts_with(&format!("{SESSION_COOKIE}=")))
        );
        assert!(
            completed_cookies
                .iter()
                .any(|cookie| cookie.starts_with(&format!("{CSRF_COOKIE}=")))
        );

        let replay = mfa_complete_login(
            State(state),
            Json(MfaLoginCompleteRequest {
                code,
                challenge_token,
            }),
        )
        .await;
        let replay_error = match replay {
            Ok(_) => panic!("MFA challenge replay must not create another session"),
            Err(error) => error,
        };
        assert_status(&replay_error, StatusCode::UNAUTHORIZED);
        assert_eq!(
            fixture.requests().len(),
            5,
            "repeat linked login fetches token + userinfo but not email"
        );
    }

    #[tokio::test]
    async fn parallel_starts_coexist_and_the_ninth_start_is_rejected() {
        let fixture =
            TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(500)).await;
        let state = state_with_oauth(&fixture);
        let mut jar = Vec::new();
        let mut names = HashSet::new();

        for index in 0..OAUTH_TRANSACTION_COOKIE_LIMIT {
            let authority = if index % 2 == 0 {
                "nebula.example"
            } else {
                "NEBULA.EXAMPLE:443"
            };
            let flow = start_flow(state.clone(), authority, &jar)
                .await
                .unwrap_or_else(|error| panic!("start {index} failed: {error:?}"));
            assert_transaction_cookie_security(&flow);
            let name = flow
                .cookie_pair
                .split_once('=')
                .expect("started cookie has a name")
                .0
                .to_owned();
            assert!(names.insert(name), "parallel flows need distinct names");
            jar.push(flow.cookie_pair);
        }

        let error = match start_flow(state, "nebula.example", &jar).await {
            Ok(_) => panic!("the ninth active transaction must be rejected"),
            Err(error) => error,
        };
        assert_status(&error, StatusCode::TOO_MANY_REQUESTS);
        assert!(
            fixture.requests().is_empty(),
            "authorization starts never use provider egress for manual endpoints"
        );
    }

    #[tokio::test]
    async fn provider_error_consumes_bound_state_clears_cookie_and_never_uses_egress() {
        const ERROR_CANARY: &str = "PROVIDER_ERROR_CANARY_DO_NOT_ECHO";
        let fixture =
            TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(500)).await;
        let state = state_with_oauth(&fixture);
        let flow = start_flow(state.clone(), "nebula.example", &[])
            .await
            .expect("OAuth start succeeds");

        let missing_cookie = provider_error_callback(
            state.clone(),
            &flow.state,
            "nebula.example",
            &[],
            false,
            ERROR_CANARY,
        )
        .await
        .expect_err("provider error without browser binding is rejected");
        assert_status(&missing_cookie, StatusCode::UNAUTHORIZED);

        let both = provider_error_callback(
            state.clone(),
            &flow.state,
            "nebula.example",
            std::slice::from_ref(&flow.cookie_pair),
            true,
            ERROR_CANARY,
        )
        .await
        .expect_err("code plus error is malformed");
        assert_status(&both, StatusCode::BAD_REQUEST);
        assert!(fixture.requests().is_empty());

        let denied = provider_error_callback(
            state.clone(),
            &flow.state,
            "nebula.example",
            std::slice::from_ref(&flow.cookie_pair),
            false,
            ERROR_CANARY,
        )
        .await
        .expect("bound provider error reaches cancellation");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let expected_clear =
            OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
        assert!(denied.headers().get_all(SET_COOKIE).iter().any(|value| {
            value
                .to_str()
                .is_ok_and(|cookie| cookie == expected_clear.as_str())
        }));
        let denied_body = to_bytes(denied.into_body(), usize::MAX)
            .await
            .expect("read provider denial response");
        let denied_body =
            String::from_utf8(denied_body.to_vec()).expect("provider denial response is UTF-8");
        assert!(denied_body.contains("OAuth authorization was not granted"));
        assert!(!denied_body.contains(ERROR_CANARY));
        assert!(!denied_body.contains("IGNORED_PROVIDER_TEXT"));

        let replay = callback(
            state,
            "github",
            &flow.state,
            "nebula.example",
            std::slice::from_ref(&flow.cookie_pair),
        )
        .await
        .expect("bound replay returns an HTTP problem response");
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
        assert!(
            fixture.requests().is_empty(),
            "denial and replay consume state without token/userinfo egress"
        );
    }

    #[tokio::test]
    async fn two_started_transactions_can_complete_independently() {
        let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"bearer"}"#),
            "/userinfo" => TestResponse::json(r#"{"id":202}"#),
            "/emails" => TestResponse::json(
                r#"[{"email":"parallel@example.com","primary":true,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let state = state_with_oauth(&fixture);
        let first = start_flow(state.clone(), "nebula.example", &[])
            .await
            .expect("first start succeeds");
        let second = start_flow(
            state.clone(),
            "nebula.example:443",
            std::slice::from_ref(&first.cookie_pair),
        )
        .await
        .expect("second start coexists");
        assert_ne!(first.state, second.state);
        assert_ne!(first.cookie_pair, second.cookie_pair);
        let jar = vec![first.cookie_pair.clone(), second.cookie_pair.clone()];

        for flow in [&first, &second] {
            let response = callback(state.clone(), "github", &flow.state, "nebula.example", &jar)
                .await
                .expect("each parallel transaction completes");
            assert_eq!(response.status(), StatusCode::OK);
            let expected_clear =
                OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
            assert!(
                response
                    .headers()
                    .get_all(SET_COOKIE)
                    .iter()
                    .any(|value| { value.to_str().is_ok_and(|cookie| cookie == expected_clear) })
            );
        }
        assert_eq!(fixture.requests().len(), 5);
    }
}
