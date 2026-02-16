# Contract: HealthCheckable Trait

**Status**: Implemented (existing)
**Location**: `crates/resource/src/health.rs`

## HealthCheckable Trait

Optional trait for resources that support periodic health monitoring. Resources without this trait are assumed healthy.

```rust
pub trait HealthCheckable: Send + Sync {
    /// Perform a health check. Returns detailed status.
    async fn health_check(&self) -> Result<HealthStatus, Error>;

    /// Context-aware health check (access to scope, cancellation, metadata).
    async fn detailed_health_check(&self, context: &Context) -> Result<HealthStatus, Error>;

    /// Recommended interval between checks.
    /// Default: 30 seconds.
    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Maximum time for a single check before it is considered failed.
    /// Default: 5 seconds.
    fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}
```

## HealthStatus

```rust
pub struct HealthStatus {
    pub state: HealthState,
    pub latency: Option<Duration>,
    pub metadata: HashMap<String, String>,
}
```

## HealthState

```rust
pub enum HealthState {
    Healthy,
    Degraded { reason: String, performance_impact: f64 },  // 0.0-1.0
    Unhealthy { reason: String, recoverable: bool },
    Unknown,
}
```

## Behavioral Contract

1. `health_check()` MUST complete within `health_check_timeout()` or be cancelled.
2. `health_check()` SHOULD be lightweight (ping, simple query) — not a full integration test.
3. `performance_impact` in `Degraded` MUST be in range 0.0 (no impact) to 1.0 (severe).
4. If `Unhealthy { recoverable: true }`, the system will attempt recovery.
5. If `Unhealthy { recoverable: false }`, the instance is immediately cleaned up.
6. `Unknown` is used when health cannot be determined (e.g., timeout without response).

## HealthChecker (Background Monitor)

```rust
// Existing — runs per-instance monitoring tasks
pub struct HealthChecker { ... }

impl HealthChecker {
    pub fn new() -> Self;
    pub async fn start_monitoring(&self, instance_id, checkable, interval, timeout);
    pub async fn stop_monitoring(&self, instance_id);
    pub async fn get_health(&self, instance_id) -> Option<HealthStatus>;
    pub async fn get_all_health(&self) -> HashMap<String, HealthStatus>;
    pub async fn get_unhealthy_instances(&self) -> Vec<String>;
    pub async fn get_critical_instances(&self, failure_threshold: u32) -> Vec<String>;
    pub async fn shutdown(&self);
}
```

## Phase 4 Extension: HealthPipeline

```rust
// Planned — multi-stage health checks
pub struct HealthPipeline {
    stages: Vec<Box<dyn HealthStage>>,
}

pub trait HealthStage: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, instance: &dyn Any) -> StageResult;
}

// Built-in stages:
// 1. Connectivity — TCP ping / simple query
// 2. Performance — latency < threshold
// 3. DependencyHealth — all dependencies healthy
```
