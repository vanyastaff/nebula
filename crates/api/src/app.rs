//! Application Builder
//!
//! Сборка Router с middleware (Production-Grade).

use std::{future::Future, sync::Arc, time::Duration};

use axum::{Router, body::Body, extract::DefaultBodyLimit, middleware, response::Response};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    trace::{DefaultMakeSpan, MakeSpan, TraceLayer},
};
use utoipa_swagger_ui::SwaggerUi;

#[cfg(any(test, feature = "test-util"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    config::{ApiConfig, IdempotencyApiConfig},
    domain,
    middleware::{
        IdempotencyLayer, idempotency::IdempotencyConfig, rate_limit::RateLimitState,
        security_headers::security_headers_middleware,
    },
    state::AppState,
    transport::webhook::resume::{RESUME_BODY_LIMIT_BYTES, resume_handler},
};

/// Build the main application router with middleware
pub fn build_app(state: AppState, config: &ApiConfig) -> Router {
    // Materialise the full route tree alongside the merged OpenAPI 3.1
    // document (stub-endpoint policy). `OpenApiRouter::split_for_parts()` is the
    // single mounting path that ties the served `axum::Router` to the
    // generated `OpenApi` value, so any handler missing
    // `#[utoipa::path]` would fail to pass through `routes!()` at compile
    // time — drift detection is structural rather than review-time.
    let (api_routes, openapi_spec) = domain::create_routes(state.clone(), config);

    let path_count = openapi_spec.paths.paths.len();
    // `OpenApiVersion` does not implement `Display`/`Debug`; serde always
    // round-trips it to the canonical version string. stub-endpoint policy pins the
    // generator to 3.1.0, so the assertion in T7 catches accidental drift.
    // A serialization failure here is unexpected — it would mean the
    // generated `OpenApi` cannot be represented in JSON, which the served
    // `/api/v1/openapi.json` endpoint depends on. Log the error so the
    // root cause is recoverable from logs, then fall back to the pinned
    // string so startup proceeds (the typed assertion in T7 catches the
    // real problem).
    let spec_version = match serde_json::to_value(&openapi_spec.openapi) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => {
            tracing::warn!(
                ?other,
                "openapi: OpenApiVersion did not serialize as JSON string; falling back"
            );
            "3.1.0".to_owned()
        },
        Err(err) => {
            tracing::error!(
                error = %err,
                "openapi: failed to serialize OpenApiVersion; falling back"
            );
            "3.1.0".to_owned()
        },
    };
    tracing::info!(
        spec.version = %spec_version,
        paths = path_count,
        "openapi: spec compiled"
    );

    // Self-hosted Swagger UI — `utoipa_swagger_ui` ships every static
    // asset (HTML, CSS, JS) embedded in the binary, so `/api/v1/docs/`
    // never reaches a third-party CDN. Spec is served back to the UI
    // from `/api/v1/openapi.json`. The `Router::from(SwaggerUi)` impl
    // is provided by the `axum` feature on `utoipa-swagger-ui`.
    let api_routes = api_routes.merge(Router::<()>::from(
        SwaggerUi::new("/api/v1/docs").url("/api/v1/openapi.json", openapi_spec),
    ));

    // Apply the REST body-limit layer BEFORE merging the webhook
    // router. The webhook transport already attaches its own
    // `DefaultBodyLimit` inside `transport.router()`; layering after
    // the merge would override it for webhook routes too, and the
    // REST default is not the right default for every webhook
    // provider. Operators tune the REST cap via `API_MAX_BODY_SIZE`
    // (defaulting to `config::REST_BODY_LIMIT_BYTES`).
    let api_routes = api_routes.layer(DefaultBodyLimit::max(config.max_body_size));

    // Test-only echo / fail routes (used by `tests/idempotency_e2e.rs`).
    // Merged BEFORE the idempotency layer so the dedup contract covers
    // them; gated behind `test-util` so production builds never include
    // these endpoints. Per stub-endpoint policy they go through `axum::Router::merge`
    // (not `OpenApiRouter`) so the spec stays clean — no `_test/*` paths
    // in `/api/v1/openapi.json`.
    #[cfg(any(test, feature = "test-util"))]
    let api_routes = api_routes.merge(test_echo_router::<()>());

    // Mount idempotency on API routes ONLY — webhook ingress has its own
    // dedup contract (ROADMAP §M3.3 — provider signature + replay-window
    // timestamp). Routing webhook traffic through the API idempotency
    // cache would inflate `nebula_api_idempotency_misses_total` with
    // provider traffic that never carries `Idempotency-Key` and conflate
    // two distinct dedup surfaces. See idempotency backend.
    let api_routes = if let Some(store) = state.idempotency_store.as_ref() {
        // `ttl_secs > 0` is structurally enforced at config load time
        // by `parse_positive_u64_env` (`ApiConfigError::ZeroValue`) so
        // a release build cannot reach here with a zero-TTL cache.
        tracing::info!(
            layer = "idempotency",
            store_kind = store.store_kind(),
            "build_app: mounting idempotency layer"
        );
        api_routes.layer(
            IdempotencyLayer::new(Arc::clone(store))
                .with_config(layer_config_from(&config.idempotency))
                .with_metrics(state.metrics_registry.clone()),
        )
    } else {
        tracing::warn!(
            "build_app: idempotency_store not configured — POST endpoints lack replay protection"
        );
        api_routes
    };

    // Merge the webhook transport router (if attached). Webhook
    // routes live alongside REST API routes on the same axum app,
    // so external providers only hit one port. `Router::merge`
    // works because the webhook router carries its own state type
    // (`WebhookTransport`) that does not collide with `AppState`.
    let routes = match state.webhook_transport.clone() {
        Some(transport) => api_routes.merge(transport.router()),
        None => api_routes,
    };

    // W-S3d: `POST /resume` — attacker-reachable wait-state surface.
    // Mounted BEFORE tenancy middleware (no TenantContext extractor);
    // scope is derived from the consumed token row, not the request.
    // Uses `AppState` as router state so it can access `resume_token_store`
    // and `resume_handler_components`.
    //
    // `DefaultBodyLimit::max` is applied as a tower layer on this sub-router so
    // axum enforces the cap BEFORE buffering the body — preventing a large-body DoS
    // from reaching the handler.  The handler also checks the cap after buffering
    // (defense-in-depth for test paths that bypass the layer).
    let routes = routes.merge(
        Router::new()
            .route(
                "/resume",
                axum::routing::post(resume_handler)
                    .layer(DefaultBodyLimit::max(RESUME_BODY_LIMIT_BYTES)),
            )
            .with_state(state.clone()),
    );

    // Internal routes (webhook activation — E3): /internal/v1/...
    // Mounted on the plain axum `Router` so they never appear in
    // `/api/v1/openapi.json`. Auth is the shared-token middleware
    // gated by `AppState.internal_shared_token`.
    let routes = routes.merge(domain::internal::router(state));

    // Build per-IP rate limiter from config.
    let rate_limit = RateLimitState::new(config.rate_limit_per_second);

    // Build middleware stack (tower `ServiceBuilder`: **first** `.layer()` sees the request
    // first — outermost). Order is therefore:
    // `TraceLayer` → `inject_w3c_trace_response_headers` → compression → CORS → merged routes
    // (public API, webhooks, `/internal/v1/*` — same W3C response policy everywhere; internal
    // routes are not in OpenAPI but still emit trace headers for operators).
    let middleware_stack = ServiceBuilder::new()
        // 1. Request tracing — link to inbound W3C parent when `InboundW3cTraceContext` is present.
        // Span level is **INFO** (not the `DefaultMakeSpan` `DEBUG` default): a default
        // `RUST_LOG=info` filter would otherwise drop the per-request span before
        // `tracing_opentelemetry::OpenTelemetryLayer` can observe it, leaving every response
        // without a `traceparent` even though `init_api_telemetry` wired the layer correctly.
        .layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<Body>| {
                let mut make_span = DefaultMakeSpan::new().level(tracing::Level::INFO);
                let span = make_span.make_span(request);
                if let Some(w3c) = request.extensions().get::<crate::middleware::InboundW3cTraceContext>()
                {
                    crate::middleware::trace_w3c::attach_inbound_trace_parent(&span, &w3c.0);
                }
                span
            },
        ))
        // 2. Response `traceparent` / `tracestate` (M3.5) — must stay **inside** `TraceLayer`'s
        // span scope on the async return path.
        .layer(middleware::from_fn(
            crate::middleware::inject_w3c_trace_response_headers,
        ))
        // 3. Response compression (if enabled)
        .layer(if config.enable_compression {
            CompressionLayer::new()
        } else {
            CompressionLayer::new().no_br().no_gzip().no_zstd()
        })
        // 4. CORS
        .layer(build_cors_layer(config));

    // Apply middleware to routes.
    // Layers are applied bottom-up: rate_limit runs first (outermost),
    // then request_id, then security_headers, then W3C trace extraction
    // (must run before `TraceLayer` inside `middleware_stack`), then the inner stack
    // (`TraceLayer` → response trace inject → compression → CORS).
    routes
        .layer(middleware_stack)
        .layer(middleware::from_fn(
            crate::middleware::trace_context_middleware,
        ))
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

/// Test-only echo / fail router used by `tests/idempotency_e2e.rs`.
///
/// Two routes:
/// - `POST /api/v1/_test/echo` — returns `200 OK` with body
///   `echo:<call_count>:<request_payload>`. The counter is per
///   `build_app` invocation (process-local, fresh per test) and lets
///   tests prove a hit bypassed the inner handler.
/// - `POST /api/v1/_test/fail` — returns `500 Internal Server Error`
///   with body `boom:<call_count>`. Used to assert that 5xx responses
///   are not cached and the inner handler runs every call.
///
/// Mounted via plain `axum::Router::merge` (NOT `OpenApiRouter`) so the
/// fixture stays out of the served OpenAPI spec — production builds
/// don't include this module at all (gated by `feature = "test-util"`),
/// and tests get predictable handlers without the auth/RBAC stack.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn test_echo_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    use axum::{Extension, body::Bytes, http::StatusCode, response::IntoResponse, routing::post};

    let counter = Arc::new(AtomicUsize::new(0));

    Router::new()
        .route(
            "/api/v1/_test/echo",
            post(
                |Extension(c): Extension<Arc<AtomicUsize>>, body: Bytes| async move {
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    let payload = String::from_utf8_lossy(&body);
                    (StatusCode::OK, format!("echo:{n}:{payload}")).into_response()
                },
            ),
        )
        .route(
            "/api/v1/_test/fail",
            post(|Extension(c): Extension<Arc<AtomicUsize>>| async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                (StatusCode::INTERNAL_SERVER_ERROR, format!("boom:{n}")).into_response()
            }),
        )
        .layer(Extension(counter))
}

/// Map `ApiConfig.idempotency` body-byte caps onto the middleware's
/// internal [`IdempotencyConfig`].
///
/// The middleware ignores the TTL / max-entries / sweep-cadence fields
/// from [`IdempotencyApiConfig`] — those are properties of the
/// **store**, set at composition-root time when the operator builds
/// `InMemoryIdempotencyStore::with_ttl_and_capacity` /
/// `PgIdempotencyStore::new`. Only the body-size guards live on the
/// per-request layer config.
fn layer_config_from(cfg: &IdempotencyApiConfig) -> IdempotencyConfig {
    IdempotencyConfig {
        max_request_body_bytes: cfg.max_request_body_bytes,
        max_response_body_bytes: cfg.max_response_body_bytes,
    }
}

/// Build CORS layer from config
fn build_cors_layer(config: &ApiConfig) -> CorsLayer {
    use axum::http::{HeaderValue, Method, header};

    use crate::middleware::{
        idempotency::{IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAY_HEADER},
        request_id::X_REQUEST_ID,
    };

    let mut cors = CorsLayer::new();

    let cors_cfg = &config.cors_config;

    if cors_cfg.allowed_origins.contains(&"*".to_string()) {
        cors = cors.allow_origin(tower_http::cors::Any);
    } else {
        // Parse specific origins
        for origin in &cors_cfg.allowed_origins {
            if let Ok(parsed) = origin.parse::<HeaderValue>() {
                cors = cors.allow_origin(parsed);
            }
        }
        if cors_cfg.allow_credentials {
            cors = cors.allow_credentials(true);
        }
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
    //
    // `Idempotency-Key` is here so cross-origin POSTs that opt into
    // replay protection clear the preflight; without it browsers
    // strip the header before the server sees it (idempotency backend).
    .allow_headers([
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::ACCEPT,
        header::HeaderName::from_static(X_REQUEST_ID),
        crate::middleware::auth::X_API_KEY.clone(),
        header::HeaderName::from_static(IDEMPOTENCY_KEY_HEADER),
        // W3C Trace Context (M3.5) — browser preflight must allow clients to send `traceparent`.
        header::HeaderName::from_static("traceparent"),
        header::HeaderName::from_static("tracestate"),
    ])
    // `Idempotent-Replay` is exposed so JS clients can read it on the
    // response and tell a cache-hit replay apart from a fresh handler
    // run; non-exposed headers are stripped by the browser before
    // `fetch().headers` sees them.
    .expose_headers([
        header::HeaderName::from_static(X_REQUEST_ID),
        IDEMPOTENT_REPLAY_HEADER,
        header::HeaderName::from_static("traceparent"),
        header::HeaderName::from_static("tracestate"),
    ])
    .max_age(Duration::from_secs(cors_cfg.max_age_secs))
}

/// Build router with graceful shutdown signal
pub async fn serve(app: Router, addr: std::net::SocketAddr) -> Result<(), std::io::Error> {
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

/// Run the server with a caller-supplied graceful-shutdown future.
///
/// Used by composition roots that must coordinate axum's shutdown with
/// background tasks (e.g. the M3.4 idempotency sweep task). The `shutdown`
/// future fires on either OS signals or when the caller's
/// `CancellationToken` is flipped — the example pattern is:
///
/// ```ignore
/// let token = tokio_util::sync::CancellationToken::new();
/// let token_for_signal = token.clone();
/// tokio::spawn(async move {
///     // wait for OS signal, flip token
///     wait_for_os_signal().await;
///     token_for_signal.cancel();
/// });
/// // sweep task observes `token.cancelled()`
/// // tokio::spawn(...);
/// app::serve_with_shutdown(app, addr, async move { token.cancelled().await }).await
/// ```
pub async fn serve_with_shutdown<F>(
    app: Router,
    addr: std::net::SocketAddr,
    shutdown: F,
) -> Result<(), std::io::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Server listening on {}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
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
