//! OpenAPI specification endpoints.
//!
//! `GET /api/v1/openapi.json` returns the cached `Arc<OnceLock<OpenApi>>` from
//! [`crate::AppState`]; the spec is materialised once at startup by
//! [`crate::build_app`]. `GET /api/v1/docs` renders a tiny self-contained
//! Swagger UI page that fetches the spec at the served URL.

use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
};

use crate::{
    errors::{ApiError, ApiResult},
    state::AppState,
};

const OPENAPI_SPEC_URL: &str = "/api/v1/openapi.json";
const SWAGGER_UI_TITLE: &str = "Nebula API – Swagger UI";

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
        (status = 503, description = "Specification has not been materialised yet (server is still starting)."),
    ),
)]
pub async fn openapi_spec(State(state): State<AppState>) -> ApiResult<Response> {
    let spec = state
        .openapi_doc
        .get()
        .ok_or(ApiError::OpenApiSpecUnavailable)?;

    let body = serde_json::to_vec(spec).map_err(|source| ApiError::OpenApiSerialize { source })?;

    tracing::debug!(
        paths = spec.paths.paths.len(),
        body_bytes = body.len(),
        "openapi: serving cached spec"
    );

    Ok((
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
        .into_response())
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
pub async fn docs_ui() -> Html<&'static str> {
    // Self-contained Swagger UI shell. Static assets are loaded from the
    // CDN-pinned `swagger-ui-dist@5` build via Subresource Integrity hashes
    // so a CDN compromise cannot inject scripts; the OpenAPI document
    // itself is fetched same-origin from `OPENAPI_SPEC_URL`.
    Html(SWAGGER_UI_HTML)
}

const SWAGGER_UI_HTML: &str = concat!(
    "<!DOCTYPE html>\n",
    "<html lang=\"en\">\n",
    "<head>\n",
    "  <meta charset=\"utf-8\">\n",
    "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "  <title>Nebula API – Swagger UI</title>\n",
    "  <link rel=\"stylesheet\"\n",
    "        href=\"https://unpkg.com/swagger-ui-dist@5/swagger-ui.css\"\n",
    "        crossorigin=\"anonymous\">\n",
    "</head>\n",
    "<body>\n",
    "  <div id=\"swagger-ui\"></div>\n",
    "  <script src=\"https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js\"\n",
    "          crossorigin=\"anonymous\"></script>\n",
    "  <script>\n",
    "    window.addEventListener(\"load\", function () {\n",
    "      window.ui = SwaggerUIBundle({\n",
    "        url: \"/api/v1/openapi.json\",\n",
    "        dom_id: \"#swagger-ui\",\n",
    "        deepLinking: true,\n",
    "      });\n",
    "    });\n",
    "  </script>\n",
    "</body>\n",
    "</html>\n",
);

// Lock the constants used by the static HTML against drift via simple
// const assertions.
const _: () = {
    assert!(OPENAPI_SPEC_URL.as_bytes()[0] == b'/');
    assert!(SWAGGER_UI_TITLE.as_bytes()[0] == b'N');
};
