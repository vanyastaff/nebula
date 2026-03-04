//! Cross-cutting HTTP middleware setup.

use tower_http::trace::TraceLayer;

/// Standard HTTP tracing layer for all API routes.
pub(crate) fn http_trace_layer() -> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>> {
    TraceLayer::new_for_http()
}
