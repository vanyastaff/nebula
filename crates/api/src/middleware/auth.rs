//! Authentication middleware — **Plane A** (host / Nebula API).
//!
//! Accepts session cookies, PAT tokens, static API keys, or JWT Bearer tokens.
//! [`AuthenticatedPrincipal`], [`AuthenticatedUser`] (legacy), and
//! [`AuthContext`] are inserted into request extensions after successful
//! authentication. Credential command handlers accept only the first: its
//! fields and constructor are private, so request JSON can never mint trusted
//! identity claims.
//!
//! This is **not** integration credential acquisition (**Plane B**). Plane B
//! enters through the credential facade's universal `resolve` / `continue`
//! protocol; it exposes no raw OAuth authorization/callback routes.

use std::{fmt, str::FromStr};

use axum::{
    extract::{Request, State},
    http::{HeaderName, StatusCode, header},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use nebula_core::UserId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    access::{Grant, parse_pat_grant},
    domain::auth::backend::{PAT_PREFIX as AUTH_PAT_PREFIX, SESSION_COOKIE as AUTH_SESSION_COOKIE},
    state::AppState,
};

/// The canonical prefix for Nebula API keys.
pub const API_KEY_PREFIX: &str = "nbl_sk_";

/// The canonical prefix for personal access tokens.
///
/// Re-exported for the auth middleware; the authoritative definition lives
/// in [`crate::domain::auth::backend::PAT_PREFIX`].
pub const PAT_PREFIX: &str = AUTH_PAT_PREFIX;

/// Cookie name for session-based authentication.
pub const SESSION_COOKIE: &str = AUTH_SESSION_COOKIE;

/// Custom header name for API key authentication.
///
/// Exposed so the CORS layer in `app::build_cors_layer` references
/// the same header constant as the auth middleware — there is
/// exactly one place the `x-api-key` string lives.
pub(crate) static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

/// Standard JWT claims validated on every request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — user ID.
    pub sub: String,
    /// Expiration time (Unix timestamp).
    pub exp: u64,
    /// Issued-at time (Unix timestamp).
    pub iat: u64,
}

/// Typed extension inserted into the request after successful auth.
///
/// Kept for backward compatibility — new code should prefer [`AuthContext`].
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// Authenticated user ID from the JWT `sub` claim, or `"api_key"` when
    /// the request was authenticated via `X-API-Key`.
    pub user_id: String,
}

/// Stable identity classification exposed to API-owned command ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthenticatedPrincipalKind {
    /// Human user.
    User,
    /// Non-human service account.
    ServiceAccount,
    /// Durable workflow identity.
    Workflow,
    /// Internal/system identity. Public credential commands reject this kind
    /// unless a future composition supplies verified durable provenance.
    System,
}

/// Trusted Plane-A principal inserted by authentication middleware.
///
/// The type intentionally has no public constructor and does not implement
/// deserialization. Downstream ports may inspect the normalized kind and
/// subject, but cannot construct an authenticated principal from request data.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    kind: AuthenticatedPrincipalKind,
    subject: String,
    authentication_binding: String,
}

impl AuthenticatedPrincipal {
    fn from_core(principal: &nebula_core::Principal, authentication_binding: String) -> Self {
        let (kind, subject) = match principal {
            nebula_core::Principal::User(id) => (AuthenticatedPrincipalKind::User, id.to_string()),
            nebula_core::Principal::ServiceAccount(id) => {
                (AuthenticatedPrincipalKind::ServiceAccount, id.to_string())
            },
            nebula_core::Principal::Workflow { workflow_id, .. } => (
                AuthenticatedPrincipalKind::Workflow,
                workflow_id.to_string(),
            ),
            nebula_core::Principal::System => {
                (AuthenticatedPrincipalKind::System, "system".to_owned())
            },
            _ => (AuthenticatedPrincipalKind::System, "system".to_owned()),
        };
        Self {
            kind,
            subject,
            authentication_binding,
        }
    }

    /// Normalized authenticated identity kind.
    #[must_use]
    pub const fn kind(&self) -> AuthenticatedPrincipalKind {
        self.kind
    }

    /// Canonical authenticated subject.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Opaque binding for the exact Plane-A credential used on this request.
    ///
    /// Interactive credential acquisition stores this value with pending
    /// state, so another session, PAT, or JWT for the same principal cannot
    /// continue the flow. It is a domain-separated digest; the presented
    /// bearer or cookie value is never exposed through this type.
    #[must_use]
    pub fn authentication_binding(&self) -> &str {
        &self.authentication_binding
    }

    /// Construct a user principal for hermetic API tests.
    ///
    /// This is unavailable in production builds; the `test-util` feature is an
    /// explicitly unsupported harness surface.
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn for_test_user(subject: impl Into<String>) -> Self {
        let subject = subject.into();
        Self {
            kind: AuthenticatedPrincipalKind::User,
            authentication_binding: pending_authentication_binding("test-user", &subject),
            subject,
        }
    }
}

impl fmt::Debug for AuthenticatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedPrincipal")
            .field("kind", &self.kind)
            .field("subject", &"[REDACTED]")
            .field("authentication_binding", &"[REDACTED]")
            .finish()
    }
}

/// Authentication context extracted by auth middleware.
///
/// Inserted into request extensions for downstream middleware and handlers.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The resolved principal identity.
    pub principal: nebula_core::Principal,
    /// Which authentication method was used.
    pub auth_method: AuthMethod,
    /// Effective API access granted to the authenticated caller.
    pub grant: Grant,
}

/// The authentication mechanism that was used for the current request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// Host-bound session cookie (`__Host-nebula-session`).
    Session {
        /// Time at which primary authentication created this session.
        authenticated_at: chrono::DateTime<chrono::Utc>,
    },
    /// Personal access token (`pat_…`).
    Pat,
    /// Static API key (`nbl_sk_…`).
    ApiKey,
    /// JWT Bearer token.
    Jwt,
}

/// Combined authentication middleware supporting four auth methods.
///
/// The middleware tries each path in order:
///
/// 1. **Authorization Bearer** — a `pat_…` PAT resolved via [`AuthBackend`], or an HS256 JWT.
/// 2. **`X-API-Key` header** — compared in constant time against configured keys.
/// 3. **Session cookie** (`__Host-nebula-session`) — resolved via [`AuthBackend`].
///
/// Explicit header credentials deliberately take precedence over ambient
/// cookie authority. If an explicit credential header is present but invalid,
/// authentication fails closed; it never downgrades to a valid session cookie.
/// Supplying both explicit mechanisms is rejected as ambiguous.
///
/// At least one must succeed, otherwise 401 is returned.
///
/// [`AuthenticatedPrincipal`], [`AuthenticatedUser`] (legacy), and
/// [`AuthContext`] are inserted into request extensions on success.
///
/// [`AuthBackend`]: crate::domain::auth::backend::AuthBackend
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let has_authorization = request.headers().contains_key(header::AUTHORIZATION);
    let has_api_key = request.headers().contains_key(&X_API_KEY);
    if has_authorization && has_api_key {
        // Two explicit identities are ambiguous. Reject instead of choosing
        // whichever happened to be checked first.
        return Err(StatusCode::UNAUTHORIZED);
    }

    // ── Path 1: explicit Authorization Bearer (PAT or JWT) ──────────────────
    if has_authorization {
        let bearer_value = extract_single_bearer(&request)?;
        if bearer_value.starts_with(PAT_PREFIX) {
            let backend = state
                .auth_backend
                .as_ref()
                .ok_or(StatusCode::UNAUTHORIZED)?;
            let record = backend
                .lookup_pat(bearer_value)
                .await
                .map_err(|_| StatusCode::UNAUTHORIZED)?
                .ok_or(StatusCode::UNAUTHORIZED)?;

            let grant = parse_pat_grant(&record.scopes).map_err(|_| StatusCode::UNAUTHORIZED)?;
            let principal = nebula_core::Principal::User(record.user_id);
            let authentication_binding = pending_authentication_binding("pat", bearer_value);
            insert_authenticated_extensions(
                &mut request,
                principal,
                AuthMethod::Pat,
                grant,
                record.user_id.to_string(),
                authentication_binding,
            );
            return Ok(next.run(request).await);
        }

        let key = DecodingKey::from_secret(state.jwt_secret.as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        let token_data = decode::<Claims>(bearer_value, &key, &validation)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;

        let user_id_str = token_data.claims.sub.clone();
        let principal = if let Ok(uid) = UserId::from_str(&user_id_str) {
            nebula_core::Principal::User(uid)
        } else {
            // Compatibility lane for technical JWTs whose subject predates
            // typed user IDs. This is independent of precedence hardening.
            nebula_core::Principal::System
        };
        let authentication_binding = pending_authentication_binding("jwt", bearer_value);
        insert_authenticated_extensions(
            &mut request,
            principal,
            AuthMethod::Jwt,
            Grant::UnrestrictedIdentity,
            user_id_str,
            authentication_binding,
        );
        return Ok(next.run(request).await);
    }

    // ── Path 2: explicit X-API-Key header ────────────────────────────────────
    if has_api_key {
        let provided = extract_single_api_key(&request)?;

        // Keys without the canonical prefix are always invalid.
        if !provided.starts_with(API_KEY_PREFIX) {
            return Err(StatusCode::UNAUTHORIZED);
        }

        // Fold over ALL keys without short-circuiting so the number of keys and
        // which key matched cannot be inferred from elapsed time (timing oracle).
        let matched = state.api_keys.iter().fold(false, |found, k| {
            found | constant_time_eq(k.as_bytes(), provided.as_bytes())
        });

        if !matched {
            return Err(StatusCode::UNAUTHORIZED);
        }

        let authentication_binding = pending_authentication_binding("api-key", provided);
        insert_authenticated_extensions(
            &mut request,
            nebula_core::Principal::System,
            AuthMethod::ApiKey,
            Grant::SystemInternal,
            "api_key".to_owned(),
            authentication_binding,
        );
        return Ok(next.run(request).await);
    }

    // ── Path 3: ambient session cookie ──────────────────────────────────────
    let session_id =
        extract_unique_cookie(&request, SESSION_COOKIE)?.ok_or(StatusCode::UNAUTHORIZED)?;
    let backend = state
        .auth_backend
        .as_ref()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let session = backend
        .get_principal_by_session(&session_id)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user_id = principal_user_id(&session.principal);
    insert_authenticated_extensions(
        &mut request,
        session.principal,
        AuthMethod::Session {
            authenticated_at: session.authenticated_at,
        },
        Grant::UnrestrictedIdentity,
        user_id,
        pending_authentication_binding("session", &session_id),
    );
    Ok(next.run(request).await)
}

fn insert_authenticated_extensions(
    request: &mut Request,
    principal: nebula_core::Principal,
    auth_method: AuthMethod,
    grant: Grant,
    legacy_user_id: String,
    authentication_binding: String,
) {
    let authenticated = AuthenticatedPrincipal::from_core(&principal, authentication_binding);
    request.extensions_mut().insert(authenticated);
    request.extensions_mut().insert(AuthenticatedUser {
        user_id: legacy_user_id,
    });
    request.extensions_mut().insert(AuthContext {
        principal,
        auth_method,
        grant,
    });
}

/// Derive a non-reversible, method-separated pending-flow binding from the
/// exact Plane-A credential presented on this request.
fn pending_authentication_binding(method: &str, credential: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nebula:plane-a:credential-pending:v1\0");
    hasher.update(method.as_bytes());
    hasher.update([0]);
    hasher.update(credential.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Extract exactly one non-empty Bearer token from `Authorization`.
fn extract_single_bearer(request: &Request) -> Result<&str, StatusCode> {
    let mut values = request.headers().get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_str()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if values.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)
}

/// Extract exactly one non-empty API key value.
fn extract_single_api_key(request: &Request) -> Result<&str, StatusCode> {
    let mut values = request.headers().get_all(&X_API_KEY).iter();
    let value = values
        .next()
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_str()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if value.is_empty() || values.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(value)
}

/// Extract one non-empty named cookie, rejecting duplicate-name shadowing.
fn extract_unique_cookie(request: &Request, name: &str) -> Result<Option<String>, StatusCode> {
    let mut found = None;
    for header in request.headers().get_all(header::COOKIE) {
        let cookie_str = header.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        for pair in cookie_str.split(';') {
            let Some((cookie_name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if cookie_name != name {
                continue;
            }
            if value.is_empty() || found.is_some() {
                return Err(StatusCode::UNAUTHORIZED);
            }
            found = Some(value.to_owned());
        }
    }
    Ok(found)
}

/// Extract a user-facing ID string from a [`Principal`](nebula_core::Principal).
fn principal_user_id(principal: &nebula_core::Principal) -> String {
    match principal {
        nebula_core::Principal::User(uid) => uid.to_string(),
        nebula_core::Principal::ServiceAccount(sid) => sid.to_string(),
        nebula_core::Principal::Workflow { workflow_id, .. } => workflow_id.to_string(),
        nebula_core::Principal::System => "system".to_string(),
        // Non-exhaustive: future principal kinds fall back to a "system" sentinel.
        _ => "system".to_string(),
    }
}

/// Constant-time byte-slice equality.
///
/// Both slices are compared in O(max(a.len(), b.len())) regardless of where
/// they first differ, preventing timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still touch every byte of `a` to avoid length oracle leaks.
        let _ = a.iter().fold(0u8, |acc, x| acc ^ x);
        return false;
    }
    // XOR all bytes together; any difference leaves a non-zero result.
    let diff = a
        .iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use axum::{
        Extension, Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
        routing::post,
    };
    use jsonwebtoken::{EncodingKey, Header, encode};
    use tower::ServiceExt;

    use super::{
        AuthContext, AuthMethod, Claims, X_API_KEY, auth_middleware, pending_authentication_binding,
    };
    use crate::{
        ApiConfig, AppState,
        domain::auth::backend::{
            AuthBackend, CSRF_COOKIE, CSRF_HEADER, CreatePatParams, InMemoryAuthBackend,
            SESSION_COOKIE, SignupRequest, dto::SecretString,
        },
        middleware::csrf::csrf_middleware,
    };

    const API_KEY: &str = "nbl_sk_session-contract-test-key";

    #[test]
    fn pending_binding_is_credential_and_method_specific_without_echoing_input() {
        const CANARY: &str = "plane-a-binding-secret-never-store-raw";
        let session = pending_authentication_binding("session", CANARY);
        let other_session = pending_authentication_binding("session", "other-credential");
        let pat = pending_authentication_binding("pat", CANARY);

        assert!(!session.contains(CANARY));
        assert_ne!(session, other_session);
        assert_ne!(session, pat);
        assert_eq!(session.len(), 43, "SHA-256 base64url without padding");
    }

    struct Fixture {
        app: Router,
        session_id: String,
        csrf_token: String,
        pat: String,
        jwt: String,
    }

    async fn auth_kind(Extension(auth): Extension<AuthContext>) -> &'static str {
        match auth.auth_method {
            AuthMethod::Session { .. } => "session",
            AuthMethod::Pat => "pat",
            AuthMethod::ApiKey => "api_key",
            AuthMethod::Jwt => "jwt",
        }
    }

    async fn fixture() -> Fixture {
        let config = ApiConfig::for_test();
        let backend = Arc::new(InMemoryAuthBackend::new());
        let profile = backend
            .register_user(SignupRequest {
                email: "cookie-contract@example.test".to_owned(),
                password: SecretString::new("correct horse battery staple".to_owned()),
                display_name: "Cookie Contract".to_owned(),
            })
            .await
            .expect("seed user");
        let mut session = backend
            .create_session(&profile.user_id)
            .await
            .expect("mint session");
        let pat = backend
            .create_pat(
                &profile.user_id,
                CreatePatParams {
                    name: "contract probe".to_owned(),
                    scopes: vec!["full_access".to_owned()],
                    ttl_seconds: None,
                },
            )
            .await
            .expect("mint PAT")
            .plaintext;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_secs();
        let jwt = encode(
            &Header::default(),
            &Claims {
                sub: profile.user_id,
                exp: now + 3_600,
                iat: now,
            },
            &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
        )
        .expect("encode JWT");
        let backend: Arc<dyn AuthBackend> = backend;
        let state = AppState::in_memory(config.jwt_secret.clone())
            .with_api_keys(vec![API_KEY.to_owned()])
            .with_auth_backend(backend);
        let app = Router::new()
            .route("/", post(auth_kind))
            .layer(middleware::from_fn(csrf_middleware))
            .layer(middleware::from_fn_with_state(state, auth_middleware));

        let session_id = std::mem::take(&mut session.id);
        let csrf_token = std::mem::take(&mut session.csrf_token);
        Fixture {
            app,
            session_id,
            csrf_token,
            pat,
            jwt,
        }
    }

    async fn body(response: axum::response::Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 64)
                .await
                .expect("read response body")
                .to_vec(),
        )
        .expect("UTF-8 response")
    }

    #[tokio::test]
    async fn explicit_credentials_override_ambient_session_and_bypass_csrf() {
        let fixture = fixture().await;
        let session_cookie = format!("{SESSION_COOKIE}={}", fixture.session_id);
        for (header_name, header_value, expected) in [
            (
                header::AUTHORIZATION,
                format!("Bearer {}", fixture.pat),
                "pat",
            ),
            (
                header::AUTHORIZATION,
                format!("Bearer {}", fixture.jwt),
                "jwt",
            ),
            (X_API_KEY.clone(), API_KEY.to_owned(), "api_key"),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/")
                        .header(header::COOKIE, &session_cookie)
                        .header(header_name, header_value)
                        .body(Body::empty())
                        .expect("explicit credential request"),
                )
                .await
                .expect("auth response");
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(body(response).await, expected);
        }
    }

    #[tokio::test]
    async fn invalid_explicit_credential_never_downgrades_to_valid_session() {
        let fixture = fixture().await;
        let session_cookie = format!("{SESSION_COOKIE}={}", fixture.session_id);
        for (header_name, header_value) in [
            (header::AUTHORIZATION, "Basic invalid"),
            (header::AUTHORIZATION, "Bearer invalid"),
            (X_API_KEY.clone(), "nbl_sk_unknown"),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/")
                        .header(header::COOKIE, &session_cookie)
                        .header(header_name, header_value)
                        .body(Body::empty())
                        .expect("invalid explicit credential request"),
                )
                .await
                .expect("auth response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let ambiguous = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, session_cookie)
                    .header(header::AUTHORIZATION, format!("Bearer {}", fixture.jwt))
                    .header(X_API_KEY.clone(), API_KEY)
                    .body(Body::empty())
                    .expect("ambiguous explicit credential request"),
            )
            .await
            .expect("auth response");
        assert_eq!(ambiguous.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn session_requires_one_matching_csrf_pair() {
        let fixture = fixture().await;
        let session_cookie = format!("{SESSION_COOKIE}={}", fixture.session_id);

        let missing = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, &session_cookie)
                    .body(Body::empty())
                    .expect("session request"),
            )
            .await
            .expect("auth response");
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);

        let accepted = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(
                        header::COOKIE,
                        format!("{session_cookie}; {CSRF_COOKIE}={}", fixture.csrf_token),
                    )
                    .header(CSRF_HEADER, &fixture.csrf_token)
                    .body(Body::empty())
                    .expect("session CSRF request"),
            )
            .await
            .expect("auth response");
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(body(accepted).await, "session");
    }

    #[test]
    fn duplicate_session_or_explicit_headers_are_rejected_by_parsers() {
        let duplicate_cookie = Request::builder()
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE}=one; {SESSION_COOKIE}=two"),
            )
            .body(Body::empty())
            .expect("duplicate cookie request");
        assert_eq!(
            super::extract_unique_cookie(&duplicate_cookie, SESSION_COOKIE),
            Err(StatusCode::UNAUTHORIZED)
        );

        let mut duplicate_bearer = Request::new(Body::empty());
        duplicate_bearer
            .headers_mut()
            .append(header::AUTHORIZATION, "Bearer one".parse().expect("header"));
        duplicate_bearer
            .headers_mut()
            .append(header::AUTHORIZATION, "Bearer two".parse().expect("header"));
        assert_eq!(
            super::extract_single_bearer(&duplicate_bearer),
            Err(StatusCode::UNAUTHORIZED)
        );
    }
}
