//! OpenAPI specification endpoints.

use axum::Json;

use crate::errors::{ApiError, ApiResult};

/// GET /api/v1/openapi.json
pub async fn openapi_spec() -> ApiResult<Json<serde_json::Value>> {
    // TODO: Return generated OpenAPI 3.1 spec
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/docs
pub async fn docs_ui() -> ApiResult<String> {
    // TODO: Return Swagger UI HTML
    Err(ApiError::Internal("not implemented".to_string()))
}
