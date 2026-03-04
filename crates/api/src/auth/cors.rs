//! CORS configuration for the public API.

use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::config;

pub(crate) fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(config::load_cors_allowed_origins()))
        .allow_methods(Any)
        .allow_headers(Any)
}
