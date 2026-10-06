//! Query-string extractor whose rejections are RFC 9457 problems.
//!
//! `axum::extract::Query` rejects an unparsable or mistyped query string with
//! a `text/plain` response that can quote the submitted value. List handlers
//! read their parameters through [`ApiQuery`] instead, so the rejection crosses
//! the boundary as a fixed, payload-free `application/problem+json` 400 — the
//! envelope the served OpenAPI document declares for that status.

use axum::{
    extract::{FromRequestParts, Query},
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// A query string decoded into `T`; see the [module docs](self).
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| {
                tracing::debug!(target: "nebula_api::request_query", "query string rejected");
                ApiError::validation_message(
                    "Query string does not match the operation's parameters.",
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, http::StatusCode, routing::get};
    use tower::ServiceExt;

    use super::*;

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Limited {
        #[expect(dead_code, reason = "decoded only to exercise the extractor")]
        limit: Option<u32>,
    }

    #[tokio::test]
    async fn query_rejections_are_payload_free_problems() {
        for uri in ["/?limit=secret-canary", "/?secret-canary=1"] {
            let app = Router::new().route(
                "/",
                get(|_: ApiQuery<Limited>| async { StatusCode::NO_CONTENT }),
            );
            let response = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(
                response.headers()["content-type"],
                "application/problem+json",
                "{uri}"
            );
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(!text.contains("secret-canary"), "{text}");
        }
    }
}
