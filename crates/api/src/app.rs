//! Application Builder
//!
//! Сборка Router с middleware (Production-Grade).

use std::time::Duration;

use axum::{Router, extract::DefaultBodyLimit, middleware, response::Response};
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

use crate::{
    config::ApiConfig,
    middleware::{rate_limit::RateLimitState, security_headers::security_headers_middleware},
    routes,
    state::AppState,
};

/// Build the main application router with middleware
pub fn build_app(state: AppState, config: &ApiConfig) -> Router {
    // Apply the REST body-limit layer BEFORE merging the webhook
    // router. The webhook transport already attaches its own
    // `DefaultBodyLimit` inside `transport.router()`; layering after
    // the merge would override it for webhook routes too, and the
    // REST default is not the right default for every webhook
    // provider. Operators tune the REST cap via `API_MAX_BODY_SIZE`
    // (defaulting to `config::REST_BODY_LIMIT_BYTES`).
    let routes = routes::create_routes(state.clone(), config)
        .layer(DefaultBodyLimit::max(config.max_body_size));

    // Merge the webhook transport router (if attached). Webhook
    // routes live alongside REST API routes on the same axum app,
    // so external providers only hit one port. `Router::merge`
    // works because the webhook router carries its own state type
    // (`WebhookTransport`) that does not collide with `AppState`.
    let routes = match state.webhook_transport {
        Some(transport) => routes.merge(transport.router()),
        None => routes,
    };

    // Build per-IP rate limiter from config.
    let rate_limit = RateLimitState::new(config.rate_limit_per_second);

    // Build middleware stack (ServiceBuilder — сверху вниз)
    let middleware_stack = ServiceBuilder::new()
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

    // Apply middleware to routes.
    // Layers are applied bottom-up: rate_limit runs first (outermost),
    // then request_id, then security_headers, then the inner stack.
    routes
        .layer(middleware_stack)
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(request_id_middleware))
        // Global per-IP rate limiting — placed outermost so it runs first
        // and rejects excess traffic before any heavier processing begins.
        .layer(middleware::from_fn(move |req, next| {
            let rl = rate_limit.clone();
            async move { rl.handle(req, next).await }
        }))
}

/// Request ID middleware
async fn request_id_middleware(
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    use uuid::Uuid;

    use crate::middleware::request_id::{RequestId, X_REQUEST_ID};

    let request_id = request
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|h| h.to_str().ok())
        .map_or_else(|| Uuid::new_v4().to_string(), ToString::to_string);

    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

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
    // Allowed headers must match the auth middleware's accepted
    // headers — if the middleware accepts `X-API-Key` but CORS
    // rejects the preflight, browser clients never get past the
    // OPTIONS probe. The policy must match the *protocol surface*,
    // not the *current tenant config*: enabling API keys later
    // should not need a restart for preflight to work.
    .allow_headers([
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::ACCEPT,
        header::HeaderName::from_static(X_REQUEST_ID),
        crate::middleware::auth::X_API_KEY.clone(),
    ])
    .expose_headers([header::HeaderName::from_static(X_REQUEST_ID)])
    .max_age(Duration::from_hours(1))
}

/// Build router with graceful shutdown signal
pub async fn serve(
    app: Router,
    addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Server listening on {}", addr);

    axum::serve(
        listener,
        // `into_make_service_with_connect_info` populates `ConnectInfo<SocketAddr>`
        // in request extensions so the rate-limit middleware can read the real peer IP.
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown");
}
