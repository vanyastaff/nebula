//! JSON request-body extractor whose rejections are RFC 9457 problems.
//!
//! `axum::Json` rejects a malformed, oversized or mistyped body with a
//! `text/plain` response whose text can quote the submitted value. Every REST
//! handler reads its body through [`ApiJson`] instead, so a body rejection keeps
//! axum's status code but crosses the boundary as a fixed, payload-free
//! `application/problem+json` document — the same envelope the served OpenAPI
//! document declares for these statuses.

use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// A JSON request body decoded into `T`; see the [module docs](self).
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(body_rejection)
    }
}

/// Map a body rejection onto its problem without echoing the body.
fn body_rejection(rejection: JsonRejection) -> ApiError {
    let status = rejection.status();
    tracing::debug!(
        target: "nebula_api::request_body",
        status = status.as_u16(),
        "request body rejected"
    );
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::UnsupportedMediaType,
        StatusCode::UNPROCESSABLE_ENTITY => ApiError::Unprocessable(
            "Request body does not match the operation's request schema.".to_owned(),
        ),
        _ => ApiError::validation_message("Request body is not a valid JSON document."),
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request as HttpRequest, routing::post};
    use tower::ServiceExt;

    use super::*;

    #[derive(serde::Deserialize)]
    struct Named {
        #[expect(dead_code, reason = "decoded only to exercise the extractor")]
        name: String,
    }

    async fn send(content_type: Option<&str>, body: &'static str) -> (StatusCode, String, String) {
        let app = Router::new()
            .route(
                "/",
                post(|_: ApiJson<Named>| async { StatusCode::NO_CONTENT }),
            )
            .layer(axum::extract::DefaultBodyLimit::max(64));
        let mut request = HttpRequest::builder().method("POST").uri("/");
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let response = app
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let media = response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        (status, media, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn body_rejections_keep_their_status_as_payload_free_problems() {
        let oversized = "{\"name\":\"secret-canary-secret-canary-secret-canary-secret-canary\"}";
        for (content_type, body, status) in [
            (
                Some("application/json"),
                "{\"name\":",
                StatusCode::BAD_REQUEST,
            ),
            (
                Some("application/json"),
                "{\"name\":\"secret-canary\",\"name\":1}",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                "{\"name\":7}",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                None,
                "{\"name\":\"secret-canary\"}",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                Some("application/json"),
                oversized,
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let (observed, media, text) = send(content_type, body).await;
            assert_eq!(observed, status, "{body}");
            assert_eq!(media, "application/problem+json", "{body}");
            assert!(!text.contains("secret-canary"), "{text}");
            let problem: nebula_api_contract::v1::problem::ProblemDetails =
                serde_json::from_str(&text).unwrap();
            assert_eq!(problem.status, status.as_u16());
        }
    }
}
