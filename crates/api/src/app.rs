//! Application Builder
//!
//! Сборка Router с middleware (Production-Grade).

use crate::{config::ApiConfig, routes, state::AppState, middleware::security_headers::security_headers_middleware};
use axum::{Router, http::StatusCode, error_handling::HandleErrorLayer, response::Response};
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    trace::TraceLayer,
};

/// Build the main application router with middleware
pub fn build_app(state: AppState, config: &ApiConfig) -> Router {
    let routes = routes::create_routes();

    // Build middleware stack (ServiceBuilder — сверху вниз)
    let middleware = ServiceBuilder::new()
        // 1. Request tracing
        .layer(TraceLayer::new_for_http())
        // 2. Response compression (if enabled)
        .layer(if config.enable_compression {
            CompressionLayer::new()
        } else {
            CompressionLayer::new().no_br().no_gzip().no_zstd()
        })
        // 3. CORS
        .layer(build_cors_layer(config));

    // Apply middleware to routes
    // For high-load layers that can fail (timeout, load_shed), 
    // we use HandleErrorLayer to map errors to proper API responses.
    routes
        .layer(middleware)
        .layer(axum::middleware::from_fn(security_headers_middleware))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .with_state(state)
}

/// Request ID middleware
async fn request_id_middleware(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use crate::middleware::request_id::{X_REQUEST_ID, RequestId};
    use uuid::Uuid;

    let request_id = request
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    request.extensions_mut().insert(RequestId(request_id.clone()));

    let mut response = next.run(request).await;
    
    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert(X_REQUEST_ID, value);
    }

    response
}

/// Build CORS layer from config
fn build_cors_layer(config: &ApiConfig) -> CorsLayer {
    use axum::http::{HeaderValue, Method, header};
    use crate::middleware::request_id::X_REQUEST_ID;

    let mut cors = CorsLayer::new();

    if config.cors_allowed_origins.contains(&"*".to_string()) {
        cors = cors.allow_origin(tower_http::cors::Any);
    } else {
        // Parse specific origins
        for origin in &config.cors_allowed_origins {
            if let Ok(parsed) = origin.parse::<HeaderValue>() {
                cors = cors.allow_origin(parsed);
            }
        }
        cors = cors.allow_credentials(true);
    }

    cors.allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            header::HeaderName::from_static(X_REQUEST_ID),
        ])
        .expose_headers([
            header::HeaderName::from_static(X_REQUEST_ID),
        ])
        .max_age(Duration::from_secs(3600))
}

/// Build router with graceful shutdown signal
pub async fn serve(
    app: Router,
    addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Graceful shutdown signal
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown");
}

