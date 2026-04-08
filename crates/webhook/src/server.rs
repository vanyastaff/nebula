//! Webhook HTTP server implementation

use crate::{
    Error, Result, TriggerCtx, TriggerHandle, WebhookPayload,
    queue::InboundQueue,
    route_map::{RouteMap, SharedRouteMap},
};
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{any, get},
};
use bytes::Bytes;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::{Level, debug, error, info, warn};

/// Configuration for the webhook server
#[derive(Debug, Clone)]
pub struct WebhookServerConfig {
    /// Address to bind to (e.g., "0.0.0.0:8080")
    pub bind_addr: SocketAddr,

    /// Base URL for webhooks (e.g., <https://nebula.example.com>)
    pub base_url: String,

    /// Path prefix for all webhooks (e.g., "/webhooks")
    pub path_prefix: String,

    /// Enable request compression
    pub enable_compression: bool,

    /// Enable CORS
    pub enable_cors: bool,

    /// Request body size limit in bytes
    pub body_limit: usize,
}

impl Default for WebhookServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".parse().unwrap(),
            base_url: "http://localhost:8080".to_string(),
            path_prefix: "/webhooks".to_string(),
            enable_compression: true,
            enable_cors: true,
            body_limit: 10 * 1024 * 1024, // 10 MB
        }
    }
}

/// Shared server state (used by both standalone and embedded mode).
#[derive(Clone)]
pub(crate) struct ServerState {
    pub(crate) routes: SharedRouteMap,
    pub(crate) config: Arc<WebhookServerConfig>,
    /// Optional durable queue: events are enqueued before HTTP 200 is sent.
    pub(crate) inbound_queue: Option<Arc<dyn InboundQueue>>,
}

/// Singleton HTTP webhook server
///
/// Accepts incoming webhook requests and routes them to registered triggers.
/// Only one server instance should run per Nebula runtime.
pub struct WebhookServer {
    config: Arc<WebhookServerConfig>,
    routes: SharedRouteMap,
    server_handle: Arc<Mutex<Option<JoinHandle<Result<()>>>>>,
    shutdown: CancellationToken,
    /// Optional durable queue wired in via [`with_inbound_queue`](WebhookServer::with_inbound_queue).
    inbound_queue: Option<Arc<dyn InboundQueue>>,
}

impl WebhookServer {
    /// Create a new webhook server.
    ///
    /// This starts the HTTP server immediately.
    pub async fn new(config: WebhookServerConfig) -> Result<Arc<Self>> {
        let routes = Arc::new(RouteMap::new());
        let shutdown = CancellationToken::new();
        let config = Arc::new(config);

        info!(
            bind_addr = %config.bind_addr,
            base_url = %config.base_url,
            path_prefix = %config.path_prefix,
            "Starting webhook server"
        );

        let server = Arc::new(Self {
            config: config.clone(),
            routes: routes.clone(),
            server_handle: Arc::new(Mutex::new(None)),
            shutdown: shutdown.clone(),
            inbound_queue: None,
        });

        // Start the HTTP server
        let handle = Self::start_server(config, routes, None, shutdown).await?;

        // Store the handle
        *server.server_handle.lock().await = Some(handle);

        info!("Webhook server started successfully");
        Ok(server)
    }

    /// Create a webhook server in **embedded** mode (no bind, no spawn).
    ///
    /// Use [`router`](Self::router) to get an Axum `Router` and merge it into
    /// another server (e.g. unified API server on one port).
    pub fn new_embedded(config: WebhookServerConfig) -> Result<Arc<Self>> {
        let routes = Arc::new(RouteMap::new());
        let config = Arc::new(config);
        info!(
            base_url = %config.base_url,
            path_prefix = %config.path_prefix,
            "Webhook server created in embedded mode"
        );
        Ok(Arc::new(Self {
            config: config.clone(),
            routes: routes.clone(),
            server_handle: Arc::new(Mutex::new(None)),
            shutdown: CancellationToken::new(),
            inbound_queue: None,
        }))
    }

    /// Attach a durable inbound queue.
    ///
    /// When configured, each accepted webhook event is enqueued **before** the
    /// HTTP 200 response is sent to the caller.  If the queue call fails the
    /// server responds with HTTP 500 instead, ensuring the sender retries.
    ///
    /// The queue field is `Option` so existing users are unaffected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use nebula_webhook::{WebhookServer, WebhookServerConfig, queue::MemoryInboundQueue};
    ///
    /// # async fn run() -> nebula_webhook::Result<()> {
    /// let queue = Arc::new(MemoryInboundQueue::new());
    /// let server = WebhookServer::new(WebhookServerConfig::default()).await?;
    /// // Note: attach before first request — see `new_with_queue` for a combined constructor.
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_inbound_queue(self: Arc<Self>, queue: Arc<dyn InboundQueue>) -> Arc<Self> {
        // SAFETY: We are the only Arc holder at construction time when this is
        // called immediately after `new_embedded`. For the general case we need
        // to reconstruct. We do it via a new Arc wrapping a fresh struct because
        // WebhookServer is meant to be constructed once.
        Arc::new(Self {
            config: self.config.clone(),
            routes: self.routes.clone(),
            server_handle: self.server_handle.clone(),
            shutdown: self.shutdown.clone(),
            inbound_queue: Some(queue),
        })
    }

    /// Build the webhook HTTP router for embedding into another server.
    ///
    /// Mount this router at the root (it already uses [`path_prefix`](WebhookServerConfig::path_prefix)
    /// for webhook routes). Example: `api_app.merge(webhook_server.router())`.
    pub fn router(&self) -> Router {
        let state = ServerState {
            routes: self.routes.clone(),
            config: self.config.clone(),
            inbound_queue: self.inbound_queue.clone(),
        };
        let app = Router::new()
            .route(
                &format!("{}/{{*path}}", self.config.path_prefix),
                any(webhook_handler),
            )
            .with_state(state)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            );
        let app = if self.config.enable_compression {
            app.layer(CompressionLayer::new())
        } else {
            app
        };
        if self.config.enable_cors {
            app.layer(CorsLayer::permissive())
        } else {
            app
        }
    }

    /// Start the HTTP server (standalone mode).
    async fn start_server(
        config: Arc<WebhookServerConfig>,
        routes: SharedRouteMap,
        inbound_queue: Option<Arc<dyn InboundQueue>>,
        shutdown: CancellationToken,
    ) -> Result<JoinHandle<Result<()>>> {
        let state = ServerState {
            routes: routes.clone(),
            config: config.clone(),
            inbound_queue,
        };

        let app = Router::new()
            .route("/health", get(health_check))
            .route(
                &format!("{}/{{*path}}", config.path_prefix),
                any(webhook_handler),
            )
            .with_state(state)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            );

        let app = if config.enable_compression {
            app.layer(CompressionLayer::new())
        } else {
            app
        };

        let app = if config.enable_cors {
            app.layer(CorsLayer::permissive())
        } else {
            app
        };

        let listener = TcpListener::bind(&config.bind_addr)
            .await
            .map_err(|e| Error::bind_failed(config.bind_addr.to_string(), e))?;

        info!(addr = %config.bind_addr, "Webhook server listening");

        let handle = tokio::spawn(async move {
            tokio::select! {
                result = axum::serve(listener, app) => {
                    result.map_err(Error::from)?;
                }
                _ = shutdown.cancelled() => {
                    info!("Webhook server received shutdown signal");
                }
            }

            info!("Webhook server shut down gracefully");
            Ok(())
        });

        Ok(handle)
    }

    /// Subscribe to webhooks for a trigger
    ///
    /// Returns a handle that receives incoming webhook payloads.
    /// When the handle is dropped, the webhook path is automatically unregistered.
    pub async fn subscribe(
        &self,
        ctx: &TriggerCtx,
        capacity: Option<usize>,
    ) -> Result<TriggerHandle> {
        let path = ctx.webhook_path();

        debug!(path = %path, trigger_id = %ctx.trigger_id, "Subscribing to webhooks");

        // Register the route
        let receiver = self.routes.register(&path, capacity)?;

        // Create cancellation token
        let cancel = ctx.child_cancellation();

        // Create cleanup callback
        let routes = self.routes.clone();
        let path_clone = path.clone();
        let cleanup = move || {
            if let Err(e) = routes.unregister(&path_clone) {
                warn!(path = %path_clone, error = %e, "Failed to unregister route during cleanup");
            }
        };

        let handle = TriggerHandle::new(path.clone(), receiver, cancel).with_cleanup(cleanup);

        info!(path = %path, trigger_id = %ctx.trigger_id, "Subscribed to webhooks");

        Ok(handle)
    }

    /// Get the server configuration
    pub fn config(&self) -> &WebhookServerConfig {
        &self.config
    }

    /// Get the number of registered routes
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Get all registered webhook paths
    pub fn paths(&self) -> Vec<String> {
        self.routes.paths()
    }

    /// Check if a specific path is registered
    pub fn is_registered(&self, path: &str) -> bool {
        self.routes.contains(path)
    }

    /// Shutdown the webhook server gracefully
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down webhook server");
        self.shutdown.cancel();

        // Take the handle out of the mutex
        let handle = self.server_handle.lock().await.take();

        if let Some(mut handle) = handle {
            tokio::select! {
                result = &mut handle => {
                    match result {
                        Ok(Ok(())) => {
                            info!("Webhook server shut down successfully");
                            Ok(())
                        }
                        Ok(Err(e)) => {
                            error!(error = %e, "Webhook server encountered an error during shutdown");
                            Err(e)
                        }
                        Err(e) => {
                            error!(error = %e, "Webhook server task panicked");
                            Err(Error::other(format!("Server task panicked: {}", e)))
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                    warn!("Webhook server shutdown timed out after 10s");
                    Err(Error::timeout(10))
                }
            }
        } else {
            Ok(())
        }
    }
}

/// Health check endpoint handler
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Main webhook handler
async fn webhook_handler(
    State(state): State<ServerState>,
    Path(path): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let full_path = format!("{}/{}", state.config.path_prefix, path);

    debug!(
        path = %full_path,
        method = %method,
        body_size = body.len(),
        "Received webhook request"
    );

    // Check body size limit
    if body.len() > state.config.body_limit {
        warn!(
            path = %full_path,
            body_size = body.len(),
            limit = state.config.body_limit,
            "Request body exceeds size limit"
        );
        return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
    }

    // Convert headers to HashMap
    let headers_map: std::collections::HashMap<String, String> = headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_lowercase(), v.to_string()))
        })
        .collect();

    // Create payload
    let payload = WebhookPayload::new(full_path.clone(), method.to_string(), headers_map, body);

    // --- Durable inbound queue ---
    // Enqueue BEFORE sending HTTP 200 so the event is persisted even if the
    // process crashes immediately after.  If the queue call fails we return
    // HTTP 500 so the sender retries.
    if let Some(ref queue) = state.inbound_queue {
        let event = serde_json::json!({
            "path": payload.path,
            "method": payload.method,
            "headers": payload.headers,
            "body": payload.body_str().unwrap_or("<binary>"),
            "received_at": payload.received_at.to_rfc3339(),
        });
        match queue.enqueue(event).await {
            Ok(task_id) => {
                debug!(path = %full_path, task_id = %task_id, "Event enqueued to durable queue");
            }
            Err(e) => {
                error!(path = %full_path, error = %e, "Failed to enqueue webhook event");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to enqueue event")
                    .into_response();
            }
        }
    }

    // Dispatch to registered route
    match state.routes.dispatch(&full_path, payload) {
        Ok(()) => {
            debug!(path = %full_path, "Webhook dispatched successfully");
            (StatusCode::OK, "Accepted").into_response()
        }
        Err(Error::RouteNotFound { .. }) => {
            warn!(path = %full_path, "Webhook path not found");
            (StatusCode::NOT_FOUND, "Webhook path not found").into_response()
        }
        Err(e) => {
            error!(path = %full_path, error = %e, "Failed to dispatch webhook");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use nebula_resource::{Context, Scope};
    use std::sync::Arc as StdArc;

    #[tokio::test]
    async fn test_server_creation() {
        let config = WebhookServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(), // Random port
            ..Default::default()
        };

        let server = WebhookServer::new(config).await;
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_subscribe_and_unsubscribe() {
        let config = WebhookServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };

        let server = WebhookServer::new(config).await.unwrap();

        let base = Context::new(
            Scope::Global,
            nebula_core::WorkflowId::new(),
            nebula_core::ExecutionId::new(),
        );
        let state = StdArc::new(crate::TriggerState::new("test-trigger"));
        let ctx = TriggerCtx::new(
            base,
            "test-trigger",
            crate::Environment::Production,
            state,
            "http://localhost:8080",
            "/webhooks",
        );

        // Subscribe
        let handle = server.subscribe(&ctx, None).await;
        assert!(handle.is_ok());

        let path = ctx.webhook_path();
        assert!(server.is_registered(&path));

        // Drop handle - should unregister
        drop(handle);

        // Give it a moment to clean up
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(!server.is_registered(&path));
    }

    #[tokio::test]
    async fn test_route_count() {
        let config = WebhookServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };

        let server = WebhookServer::new(config).await.unwrap();

        assert_eq!(server.route_count(), 0);

        let base = Context::new(
            Scope::Global,
            nebula_core::WorkflowId::new(),
            nebula_core::ExecutionId::new(),
        );
        let state = StdArc::new(crate::TriggerState::new("test-trigger"));
        let ctx = TriggerCtx::new(
            base,
            "test-trigger",
            crate::Environment::Production,
            state,
            "http://localhost:8080",
            "/webhooks",
        );

        let _handle = server.subscribe(&ctx, None).await.unwrap();
        assert_eq!(server.route_count(), 1);
    }
}
