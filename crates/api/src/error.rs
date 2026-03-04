//! Unified HTTP error envelope for API handlers.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::contracts::ApiErrorResponse;

/// Canonical result type for API handlers.
pub(crate) type ApiResult<T> = Result<T, ApiHttpError>;

/// HTTP error with standard `{ error, message }` JSON payload.
#[derive(Debug, Clone)]
pub(crate) struct ApiHttpError {
    status: StatusCode,
    error: String,
    message: String,
    retry_after_seconds: Option<u64>,
}

impl ApiHttpError {
    pub(crate) fn new(
        status: StatusCode,
        error: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            error: error.into(),
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub(crate) fn bad_request(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error, message)
    }

    pub(crate) fn unauthorized(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, error, message)
    }

    pub(crate) fn not_found(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, error, message)
    }

    pub(crate) fn conflict(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, error, message)
    }

    pub(crate) fn service_unavailable(
        error: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, error, message)
    }

    pub(crate) fn internal(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error, message)
    }

    pub(crate) fn too_many_requests(
        error: impl Into<String>,
        message: impl Into<String>,
        retry_after_seconds: u64,
    ) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            error: error.into(),
            message: message.into(),
            retry_after_seconds: Some(retry_after_seconds),
        }
    }
}

impl IntoResponse for ApiHttpError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ApiErrorResponse::new(self.error, self.message)),
        )
            .into_response();

        if let Some(retry_after_seconds) = self.retry_after_seconds
            && let Ok(v) = HeaderValue::from_str(&retry_after_seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, v);
        }

        response
    }
}
