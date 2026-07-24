//! CSRF protection middleware using double-submit cookie pattern.
//!
//! For state-changing requests (POST, PUT, PATCH, DELETE):
//! - Verifies `X-CSRF-Token` header matches the CSRF cookie for session auth
//! - Skips for bearer/header auth (PAT, JWT, API key: no ambient cookie authority)
//! - GET/HEAD/OPTIONS requests are exempt

use axum::{
    extract::Request,
    http::{Method, header},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use crate::{
    domain::auth::backend::{CSRF_COOKIE, CSRF_HEADER},
    error::ApiError,
    middleware::auth::{AuthContext, AuthMethod},
};

/// CSRF verification middleware.
///
/// Must run AFTER auth middleware (needs [`AuthContext`] to check auth method).
///
/// Enforces double-submit cookie verification for state-changing requests
/// only when the caller authenticated via a session cookie. PAT, JWT, and
/// API-key requests are exempt because their credentials are explicit headers,
/// not ambient browser authority.
pub async fn csrf_middleware(request: Request, next: Next) -> Result<Response, ApiError> {
    // Only check state-changing methods
    let needs_csrf = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );

    if !needs_csrf {
        return Ok(next.run(request).await);
    }

    // CSRF protects ambient cookie authority only. Bearer/header credentials
    // are not attached cross-origin by the browser and must not require a
    // second, unrelated cookie to authorize a request.
    if let Some(auth_ctx) = request.extensions().get::<AuthContext>() {
        match &auth_ctx.auth_method {
            AuthMethod::Pat | AuthMethod::ApiKey | AuthMethod::Jwt => {
                return Ok(next.run(request).await);
            },
            AuthMethod::Session { .. } => {
                // Session authentication uses ambient cookies — verify the
                // matching double-submit token below.
            },
        }
    }

    let csrf_header = extract_unique_header(&request);
    let csrf_cookie = extract_unique_cookie(&request, CSRF_COOKIE);

    // Verify they match
    match (csrf_header, csrf_cookie) {
        (Ok(Some(header_val)), Ok(Some(cookie_val)))
            if bool::from(header_val.as_bytes().ct_eq(cookie_val.as_bytes())) =>
        {
            Ok(next.run(request).await)
        },
        (Ok(None), _) | (_, Ok(None)) => {
            // Missing token
            Err(ApiError::Forbidden("CSRF token missing".to_string()))
        },
        _ => {
            // Mismatch
            Err(ApiError::Forbidden("CSRF token mismatch".to_string()))
        },
    }
}

/// Extract one non-empty CSRF header, rejecting ambiguous duplicates.
fn extract_unique_header(request: &Request) -> Result<Option<String>, ()> {
    let mut found = None;
    for header in request.headers().get_all(CSRF_HEADER) {
        let value = header.to_str().map_err(|_| ())?;
        if value.is_empty() || found.is_some() {
            return Err(());
        }
        found = Some(value.to_owned());
    }
    Ok(found)
}

/// Extract one non-empty named cookie, rejecting duplicate-name shadowing.
fn extract_unique_cookie(request: &Request, name: &str) -> Result<Option<String>, ()> {
    let mut found = None;
    for header in request.headers().get_all(header::COOKIE) {
        let cookie_str = header.to_str().map_err(|_| ())?;
        for pair in cookie_str.split(';') {
            let Some((cookie_name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if cookie_name != name {
                continue;
            }
            if value.is_empty() || found.is_some() {
                return Err(());
            }
            found = Some(value.to_owned());
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::post,
    };
    use chrono::Utc;
    use nebula_core::Principal;
    use tower::ServiceExt;

    use super::csrf_middleware;
    use crate::{
        access::Grant,
        domain::auth::backend::{CSRF_COOKIE, CSRF_HEADER},
        middleware::auth::{AuthContext, AuthMethod},
    };

    fn request(auth_method: AuthMethod) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/")
            .body(Body::empty())
            .expect("valid test request");
        request.extensions_mut().insert(AuthContext {
            principal: Principal::System,
            auth_method,
            grant: Grant::SystemInternal,
        });
        request
    }

    fn app() -> Router {
        Router::new()
            .route("/", post(|| async { StatusCode::NO_CONTENT }))
            .layer(middleware::from_fn(csrf_middleware))
    }

    fn session_auth() -> AuthMethod {
        AuthMethod::Session {
            authenticated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn jwt_bearer_does_not_require_ambient_csrf_cookie() {
        let response = app()
            .oneshot(request(AuthMethod::Jwt))
            .await
            .expect("middleware response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn session_auth_requires_matching_double_submit_token() {
        let missing = app()
            .oneshot(request(session_auth()))
            .await
            .expect("middleware response");
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);

        let mut matching = request(session_auth());
        matching
            .headers_mut()
            .insert(CSRF_HEADER, "csrf-test".parse().expect("header"));
        matching.headers_mut().insert(
            axum::http::header::COOKIE,
            format!("{CSRF_COOKIE}=csrf-test")
                .parse()
                .expect("cookie header"),
        );
        let accepted = app().oneshot(matching).await.expect("middleware response");
        assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn session_auth_rejects_duplicate_or_empty_double_submit_authority() {
        for (header_values, cookie) in [
            (
                vec!["csrf-test", "csrf-test"],
                format!("{CSRF_COOKIE}=csrf-test"),
            ),
            (
                vec!["csrf-test"],
                format!("{CSRF_COOKIE}=csrf-test; {CSRF_COOKIE}=csrf-test"),
            ),
            (vec![""], format!("{CSRF_COOKIE}=")),
        ] {
            let mut request = request(session_auth());
            for value in header_values {
                request
                    .headers_mut()
                    .append(CSRF_HEADER, value.parse().expect("valid test header"));
            }
            request.headers_mut().insert(
                axum::http::header::COOKIE,
                cookie.parse().expect("valid test cookie header"),
            );

            let response = app().oneshot(request).await.expect("middleware response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
}
