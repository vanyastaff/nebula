//! Resource data for the Resources page.
//!
//! Supports mock data (default) and live data from `nebula_resource::Manager`
//! when set via [`set_resource_data_source`]. The Resources page shows active
//! resources and their state (health, pool utilization) from the selected source.

use once_cell::sync::OnceCell;
use std::sync::Arc;

/// Resource health status.
#[derive(Clone, Copy, PartialEq)]
pub enum ResourceHealth {
    Healthy,
    Degraded,
    Down,
}

/// Resource scope for display.
#[derive(Clone, Copy, PartialEq)]
pub enum ResourceScope {
    Global,
    Workflow,
    Execution,
}

/// Single resource row.
#[derive(Clone)]
pub struct Resource {
    pub name: String,
    pub health: ResourceHealth,
    pub resource_type: String,
    pub scope: ResourceScope,
    pub connections: (u32, u32), // current / max
    pub latency_ms: Option<f32>,
    pub uptime_pct: f32,
    pub activity: Vec<f32>,
    pub last_check: String,
}

/// Summary counts for resource health.
#[derive(Clone)]
pub struct ResourceSummary {
    #[allow(dead_code)]
    pub total: u32,
    pub healthy: u32,
    pub degraded: u32,
    pub down: u32,
}

/// Source of resource data for the Resources page (mock or live from Manager).
pub trait ResourceDataSource: Send + Sync {
    /// Returns health summary counts.
    fn summary(&self) -> ResourceSummary;
    /// Returns the list of resources to display.
    fn resources(&self) -> Vec<Resource>;
}

/// Mock data source (default when no Manager is set).
struct MockDataSource;

impl ResourceDataSource for MockDataSource {
    fn summary(&self) -> ResourceSummary {
        ResourceSummary {
            total: 6,
            healthy: 4,
            degraded: 1,
            down: 1,
        }
    }

    fn resources(&self) -> Vec<Resource> {
        vec![
            Resource {
                name: "postgres-prod".into(),
                health: ResourceHealth::Healthy,
                resource_type: "PostgreSQL".into(),
                scope: ResourceScope::Global,
                connections: (12, 50),
                latency_ms: Some(3.0),
                uptime_pct: 99.0,
                activity: vec![45., 52., 48., 61., 55., 58.],
                last_check: "5s ago".into(),
            },
            Resource {
                name: "redis-cache".into(),
                health: ResourceHealth::Healthy,
                resource_type: "Redis".into(),
                scope: ResourceScope::Global,
                connections: (8, 100),
                latency_ms: Some(0.3),
                uptime_pct: 100.0,
                activity: vec![120., 115., 130., 125., 118., 122.],
                last_check: "3m ago".into(),
            },
            Resource {
                name: "smtp-sendgrid".into(),
                health: ResourceHealth::Healthy,
                resource_type: "SMTP".into(),
                scope: ResourceScope::Workflow,
                connections: (2, 10),
                latency_ms: Some(45.0),
                uptime_pct: 99.7,
                activity: vec![12., 8., 15., 10., 14., 11.],
                last_check: "12m ago".into(),
            },
            Resource {
                name: "s3-primary".into(),
                health: ResourceHealth::Degraded,
                resource_type: "S3".into(),
                scope: ResourceScope::Global,
                connections: (5, 20),
                latency_ms: Some(120.0),
                uptime_pct: 99.2,
                activity: vec![88., 92., 85., 90., 78., 82.],
                last_check: "5m ago".into(),
            },
            Resource {
                name: "logger-exec".into(),
                health: ResourceHealth::Healthy,
                resource_type: "Logger".into(),
                scope: ResourceScope::Execution,
                connections: (0, 0),
                latency_ms: None,
                uptime_pct: 100.0,
                activity: vec![200., 210., 195., 205., 198., 202.],
                last_check: "1m ago".into(),
            },
            Resource {
                name: "mongodb-analytics".into(),
                health: ResourceHealth::Down,
                resource_type: "MongoDB".into(),
                scope: ResourceScope::Global,
                connections: (0, 50),
                latency_ms: None,
                uptime_pct: 81.1,
                activity: vec![90., 85., 70., 55., 40., 20.],
                last_check: "2m ago".into(),
            },
        ]
    }
}

/// Live data source from `nebula_resource::Manager`.
///
/// Currently returns empty data because `Manager` does not yet expose a
/// monitoring snapshot API. Once `Manager::monitoring_snapshot()` is
/// implemented, this will provide real pool/health data.
struct LiveDataSource {
    #[allow(dead_code)]
    manager: Arc<nebula_resource::Manager>,
}

impl LiveDataSource {
    #[must_use]
    pub fn new(manager: Arc<nebula_resource::Manager>) -> Self {
        Self { manager }
    }
}

impl ResourceDataSource for LiveDataSource {
    fn summary(&self) -> ResourceSummary {
        // TODO: implement once Manager exposes monitoring_snapshot()
        ResourceSummary {
            total: 0,
            healthy: 0,
            degraded: 0,
            down: 0,
        }
    }

    fn resources(&self) -> Vec<Resource> {
        // TODO: implement once Manager exposes monitoring_snapshot()
        Vec::new()
    }
}

static RESOURCE_DATA_SOURCE: OnceCell<Arc<dyn ResourceDataSource>> = OnceCell::new();

/// Returns the current resource data source (mock if never set).
#[must_use]
pub fn get_resource_data_source() -> Arc<dyn ResourceDataSource> {
    RESOURCE_DATA_SOURCE
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(MockDataSource))
}

/// Sets the resource data source (e.g. when running with runtime + Manager).
///
/// Call this at startup when you have a `Manager` so the Resources page shows
/// live pool state. If never set, the app uses mock data.
pub fn set_resource_data_source(source: Arc<dyn ResourceDataSource>) {
    let _ = RESOURCE_DATA_SOURCE.set(source);
}

/// Sets the live data source from a `Manager`. Use when the app runs with the engine.
pub fn set_resource_manager(manager: Arc<nebula_resource::Manager>) {
    set_resource_data_source(Arc::new(LiveDataSource::new(manager)));
}

/// Returns the current resource summary (from mock or live source).
#[must_use]
pub fn resource_summary() -> ResourceSummary {
    get_resource_data_source().summary()
}

/// Returns the current resource list (from mock or live source).
#[must_use]
pub fn resources() -> Vec<Resource> {
    get_resource_data_source().resources()
}
