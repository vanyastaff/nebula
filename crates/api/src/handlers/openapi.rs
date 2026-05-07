//! OpenAPI specification endpoints.
//!
//! Bodies are wired in T6 (cached `Arc<OpenApi>` from `AppState`); for now
//! the handlers return 500 "not implemented". The annotations below describe
//! the **post-T6 contract**, matching ADR-0047: `/api/v1/openapi.json`
//! returns the served spec, `/api/v1/docs` renders Swagger UI HTML.

use axum::Json;

use crate::errors::{ApiError, ApiResult};

/// `GET /api/v1/openapi.json` — generated OpenAPI 3.1 specification document.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "system",
    security(()),
    responses(
        (
            status = 200,
            description = "OpenAPI 3.1 specification document for this API. Body shape follows the OpenAPI 3.1 schema; consumers should treat it as `application/json` per RFC 8259.",
            body = serde_json::Value,
            content_type = "application/json",
        ),
    ),
)]
pub async fn openapi_spec() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::Internal("not implemented".to_string()))
}

/// `GET /api/v1/docs` — Swagger UI rendering of the served spec.
#[utoipa::path(
    get,
    path = "/docs",
    tag = "system",
    security(()),
    responses(
        (
            status = 200,
            description = "Swagger UI HTML page that renders the spec served at `/api/v1/openapi.json`.",
            content_type = "text/html",
        ),
    ),
)]
pub async fn docs_ui() -> ApiResult<String> {
    Err(ApiError::Internal("not implemented".to_string()))
}
