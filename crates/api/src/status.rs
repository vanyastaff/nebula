//! API response types for status and workers.

use serde::Serialize;

/// Single node worker status (for `/api/v1/status`).
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStatus {
    /// Worker id, e.g. `wrk-1`.
    pub id: String,
    /// `"active"` or `"idle"`.
    pub status: String,
    /// Current queue length for this worker.
    pub queue_len: u32,
}

/// Webhook server section in status.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookStatus {
    /// Always `"running"` when embedded in API server.
    pub status: String,
    /// Number of registered webhook routes.
    pub route_count: usize,
    /// Registered paths (e.g. `/webhooks/...`).
    pub paths: Vec<String>,
}

impl WebhookStatus {
    /// Build from a live webhook server.
    pub fn from_server(server: &nebula_webhook::WebhookServer) -> Self {
        Self {
            status: "running".to_string(),
            route_count: server.route_count(),
            paths: server.paths(),
        }
    }
}
