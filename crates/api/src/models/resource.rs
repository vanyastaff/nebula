//! Response models for resource endpoints.

use serde::Serialize;

/// Summary of a single registered resource.
#[derive(Debug, Serialize)]
pub struct ResourceSummary {
    /// The resource's unique key (e.g. `"postgres"`, `"redis"`).
    pub key: String,
    /// Topology kind: pool, resident, service, transport, exclusive, etc.
    pub topology: String,
    /// Current lifecycle phase: initializing, ready, draining, etc.
    pub phase: String,
    /// Config generation counter (bumped on hot-reload).
    pub generation: u64,
}

/// Aggregate operation counters across all resources.
#[derive(Debug, Serialize)]
pub struct ResourceMetrics {
    /// Total successful acquires.
    pub acquire_total: u64,
    /// Total failed acquire attempts.
    pub acquire_errors: u64,
    /// Total releases (handle drops).
    pub release_total: u64,
    /// Total resource instances created.
    pub create_total: u64,
    /// Total resource instances destroyed.
    pub destroy_total: u64,
}

/// Response for `GET /api/v1/resources`.
#[derive(Debug, Serialize)]
pub struct ResourceListResponse {
    /// All registered resources.
    pub resources: Vec<ResourceSummary>,
    /// Number of registered resources.
    pub count: usize,
    /// Whether the resource manager is shutting down.
    pub is_shutdown: bool,
    /// Aggregate metrics (present when a metrics registry is configured).
    pub metrics: Option<ResourceMetrics>,
}
