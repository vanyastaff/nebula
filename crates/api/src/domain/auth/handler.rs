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
#[path = "handler_mfa_authority_tests.rs"]
mod mfa_authority_tests;

#[cfg(test)]
#[path = "handler_oauth_debug_tests.rs"]
mod oauth_debug_tests;

#[cfg(test)]
#[path = "handler_oauth_transaction_tests.rs"]
mod oauth_transaction_tests;
