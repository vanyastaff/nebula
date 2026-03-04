//! CORS configuration for the public API.

use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub(crate) fn cors_layer() -> CorsLayer {
    let configured = std::env::var("NEBULA_CORS_ALLOW_ORIGINS")
        .ok()
        .unwrap_or_default();

    let mut origins: Vec<axum::http::HeaderValue> = configured
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| axum::http::HeaderValue::from_str(value).ok())
        .collect();

    if origins.is_empty() {
        origins = vec![
            axum::http::HeaderValue::from_static("http://localhost:5173"),
            axum::http::HeaderValue::from_static("http://127.0.0.1:5173"),
            axum::http::HeaderValue::from_static("http://tauri.localhost"),
            axum::http::HeaderValue::from_static("tauri://localhost"),
        ];
    }

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any)
}
