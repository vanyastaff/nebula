# nebula-resource — Health Checking and Quarantine

`nebula-resource` includes an active health monitoring system that runs
background checks against registered resource instances, transitions them
through health states, and automatically quarantines unhealthy resources
with exponential-backoff recovery probes.

---

## Table of Contents

- [Health States](#health-states)
- [HealthCheckable Trait](#healthcheckable-trait)
- [HealthChecker](#healthchecker)
- [Health Pipeline](#health-pipeline)
- [Quarantine Lifecycle](#quarantine-lifecycle)
- [QuarantineManager](#quarantinemanager)
- [RecoveryStrategy](#recoverystrategy)
- [Integration with Manager](#integration-with-manager)
- [Custom Health Checks](#custom-health-checks)

---

## Health States

```rust
pub enum HealthState {
    /// Instance is operating normally.
    Healthy,

    /// Instance is functioning but with reduced capacity or elevated latency.
    Degraded {
        reason: String,
        /// Fraction of capacity lost: 0.0 = fully healthy, 1.0 = completely failed.
        performance_impact: f64,
    },

    /// Instance cannot serve requests.
    Unhealthy {
        reason: String,
        /// True if a recovery probe may restore the instance.
        recoverable: bool,
    },

    /// Health has not been checked yet (initial state).
    Unknown,
}
```

A `HealthStatus` wraps `HealthState` with optional latency and metadata:

```rust
pub struct HealthStatus {
    pub state: HealthState,
    pub latency: Option<Duration>,
    pub metadata: Option<HashMap<String, String>>,
}

impl HealthStatus {
    pub fn healthy() -> Self;
    pub fn unhealthy(reason: impl Into<String>) -> Self;
    pub fn degraded(reason: impl Into<String>, performance_impact: f64) -> Self;

    pub fn with_latency(self, latency: Duration) -> Self;
    pub fn with_metadata(self, key: impl Into<String>, value: impl Into<String>) -> Self;

    /// True when state is Healthy, or Degraded with performance_impact < 0.8.
    pub fn is_usable(&self) -> bool;

    /// Numeric score: 1.0 = Healthy, 0.0 = Unhealthy.
    pub fn score(&self) -> f64;
}
```

---

## HealthCheckable Trait

Implement `HealthCheckable` on `R::Instance` (or on a wrapper) to enable
active monitoring:

```rust
pub trait HealthCheckable: Send + Sync {
    /// Perform a lightweight health check.
    async fn health_check(&self) -> Result<HealthStatus>;

    /// Perform a detailed check with full execution context (optional).
    async fn detailed_health_check(&self, context: &Context) -> Result<HealthStatus> {
        self.health_check().await
    }

    /// How often HealthChecker polls this instance. Default: 30s.
    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Per-check timeout. Default: 5s.
    fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}
```

**Example — database connection:**

```rust
impl HealthCheckable for DbConnection {
    async fn health_check(&self) -> Result<HealthStatus> {
        let start = std::time::Instant::now();
        match self.execute("SELECT 1").await {
            Ok(_) => Ok(HealthStatus::healthy().with_latency(start.elapsed())),
            Err(e) => Ok(HealthStatus::unhealthy(e.to_string())),
        }
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}
```

---

## HealthChecker

`HealthChecker` spawns a background Tokio task per monitored instance.
Each task runs `health_check()` at the instance's reported interval,
enforces the per-check timeout, and records consecutive failure counts.

```rust
pub struct HealthCheckConfig {
    /// Global polling interval if the instance does not override it. Default: 30s.
    pub default_interval: Duration,
    /// Number of consecutive failures before triggering quarantine. Default: 3.
    pub failure_threshold: u32,
    /// Global check timeout if the instance does not override it. Default: 5s.
    pub check_timeout: Duration,
    /// Exponential backoff multiplier applied to the interval after each failure.
    /// Values ≤ 1.0 disable backoff. Default: 1.5.
    pub backoff_multiplier: f64,
    /// Upper bound on the backoff-extended interval. Default: 150s.
    pub max_check_interval: Duration,
    /// Random jitter fraction added to each interval (0.0–1.0). Default: 0.1.
    pub jitter_factor: f64,
}

pub struct HealthChecker { /* ... */ }

impl HealthChecker {
    pub fn new(config: HealthCheckConfig) -> Self;

    /// Attach an EventBus to emit HealthChanged events on state transitions.
    pub fn with_event_bus(config: HealthCheckConfig, event_bus: Arc<EventBus>) -> Self;

    /// Register a threshold callback invoked after each consecutive failure.
    pub fn set_threshold_callback<F>(&mut self, callback: F)
    where
        F: Fn(&str, u32) + Send + Sync + 'static;

    /// Start monitoring a specific instance.
    pub fn start_monitoring<T: HealthCheckable + 'static>(
        &self,
        instance_id: uuid::Uuid,
        resource_id: String,
        instance: Arc<T>,
    );

    /// Stop monitoring one instance.
    pub fn stop_monitoring(&self, instance_id: &uuid::Uuid);

    /// Stop monitoring all instances of a resource. Returns count stopped.
    pub fn stop_monitoring_resource(&self, resource_id: &str) -> usize;

    pub fn get_health(&self, instance_id: &uuid::Uuid) -> Option<HealthRecord>;
    pub fn get_all_health(&self) -> Vec<HealthRecord>;
    pub fn get_unhealthy_instances(&self) -> Vec<HealthRecord>;

    /// Returns instances with consecutive_failures >= failure_threshold.
    pub fn get_critical_instances(&self) -> Vec<HealthRecord>;

    pub fn shutdown(&self);
}

pub struct HealthRecord {
    pub resource_id: String,
    pub instance_id: uuid::Uuid,
    pub status: HealthStatus,
    pub checked_at: chrono::DateTime<chrono::Utc>,
    pub consecutive_failures: u32,
}
```

### State transitions and events

| Transition | Event emitted |
|-----------|---------------|
| Any state → `Healthy` | `ResourceEvent::HealthChanged { from, to: Healthy }` |
| `Healthy` → `Degraded` | `ResourceEvent::HealthChanged { from: Healthy, to: Degraded }` |
| consecutive_failures >= threshold | `ResourceEvent::Quarantined { trigger: HealthThresholdExceeded, ... }` |
| Quarantine released | `ResourceEvent::QuarantineReleased { recovery_attempts }` |

---

## Health Pipeline

For multi-step checks, compose `HealthStage` implementations:

```rust
pub trait HealthStage: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, ctx: &Context) -> Result<HealthStatus>;
}

pub struct HealthPipeline {
    /* ... */
}

impl HealthPipeline {
    pub fn new() -> Self;
    pub fn add_stage<S: HealthStage + 'static>(&mut self, stage: S);
    /// Run all stages. Returns the worst health state seen.
    pub async fn run(&self, ctx: &Context) -> Result<HealthStatus>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

### Built-in stages

**`ConnectivityStage`** — checks TCP/TLS reachability:

```rust
let stage = ConnectivityStage::new(|host: &str| async move {
    // Return true if the host is reachable
    tokio::net::TcpStream::connect((host, 5432)).await.is_ok()
});
```

**`PerformanceStage`** — measures probe latency:

```rust
let stage = PerformanceStage::new(
    Duration::from_millis(50),   // warn above 50ms
    Duration::from_millis(200),  // fail above 200ms
)
.with_probe(|host: &str| async move {
    let start = std::time::Instant::now();
    // ... execute lightweight operation
    start.elapsed()
});
```

### `ResourceHealthAdapter`

Wraps a `Resource` as a `HealthCheckable`. On each check it calls
`Resource::create → Resource::is_reusable → Resource::cleanup` to verify
the resource can produce a fresh working instance:

```rust
// Manager uses this internally when HealthCheckable is not implemented
// on the Instance type. Not typically needed in adapter crates.
```

---

## Quarantine Lifecycle

```
HealthChecker detects consecutive_failures >= failure_threshold
  │
  ▼
QuarantineManager::quarantine(resource_id, QuarantineReason::HealthCheckFailed { consecutive_failures })
  │  Creates QuarantineEntry with next_recovery_at = now + base_delay
  │  EventBus::emit(Quarantined)
  │
  ▼
Pool::acquire sees resource is quarantined
  └─ returns Error::Unavailable { retryable: false }

  ▼ (at next_recovery_at)
Recovery probe:
  Resource::create → Resource::is_reusable → Resource::cleanup
  │
  ├─ success:
  │    QuarantineManager::release(resource_id)
  │    EventBus::emit(QuarantineReleased { recovery_attempts })
  │    resource returns to normal service
  │
  └─ failure:
       QuarantineEntry::record_failed_recovery
       next_recovery_at = now + base_delay * multiplier^attempt (capped at max_delay)
       │
       ├─ recovery_attempts < max_recovery_attempts → schedule next probe
       └─ exhausted → log permanent failure, resource remains quarantined
```

---

## QuarantineManager

```rust
pub struct QuarantineConfig {
    /// Failures before automatic quarantine. Default: 3.
    pub failure_threshold: u32,
    /// Max recovery probes before giving up. Default: 5.
    pub max_recovery_attempts: u32,
    pub recovery_strategy: RecoveryStrategy,
}

pub struct QuarantineManager { /* ... */ }

impl QuarantineManager {
    pub fn new(config: QuarantineConfig) -> Self;

    /// Quarantine a resource. Returns true if newly quarantined, false if already quarantined.
    pub fn quarantine(&self, resource_id: &str, reason: QuarantineReason) -> bool;

    /// Release from quarantine. Returns the entry if it was quarantined.
    pub fn release(&self, resource_id: &str) -> Option<QuarantineEntry>;

    pub fn is_quarantined(&self, resource_id: &str) -> bool;
    pub fn get(&self, resource_id: &str) -> Option<QuarantineEntry>;

    /// Record a failed recovery probe. Returns true if max attempts exhausted.
    pub fn record_failed_recovery(&self, resource_id: &str) -> bool;

    pub fn quarantined_ids(&self) -> Vec<String>;
    pub fn entries(&self) -> Vec<QuarantineEntry>;

    /// Resources whose next_recovery_at <= now.
    pub fn due_for_recovery(&self) -> Vec<QuarantineEntry>;

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn failure_threshold(&self) -> u32;
    pub fn recovery_strategy(&self) -> &RecoveryStrategy;
}

pub struct QuarantineEntry {
    pub resource_id: String,
    pub reason: QuarantineReason,
    pub quarantined_at: chrono::DateTime<chrono::Utc>,
    pub recovery_attempts: u32,
    pub max_recovery_attempts: u32,
    pub next_recovery_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl QuarantineEntry {
    /// True if recovery_attempts >= max_recovery_attempts.
    pub fn is_exhausted(&self) -> bool;
    pub fn record_failed_recovery(&mut self, strategy: &RecoveryStrategy);
}

pub enum QuarantineReason {
    HealthCheckFailed { consecutive_failures: u32 },
    ManualQuarantine  { reason: String },
}
```

---

## RecoveryStrategy

Exponential backoff for recovery probes:

```rust
pub struct RecoveryStrategy {
    /// Initial delay before first recovery probe. Default: 1s.
    pub base_delay: Duration,
    /// Maximum delay cap. Default: 60s.
    pub max_delay: Duration,
    /// Backoff multiplier. Default: 2.0.
    pub multiplier: f64,
}

impl RecoveryStrategy {
    /// Delay before probe attempt `attempt` (0-based):
    ///   delay = min(base_delay * multiplier^attempt, max_delay)
    pub fn delay_for(&self, attempt: u32) -> Duration;
}
```

**Backoff schedule with defaults (base=1s, max=60s, mult=2.0):**

| Attempt | Delay |
|---------|-------|
| 0 | 1s |
| 1 | 2s |
| 2 | 4s |
| 3 | 8s |
| 4 | 16s |
| 5+ | 60s (capped) |

---

## Integration with Manager

`Manager::new()` and `ManagerBuilder::build()` create a `HealthChecker` and a
`QuarantineManager` automatically. No explicit setup is needed:

```rust
// Custom thresholds:
let manager = ManagerBuilder::new()
    .health_config(HealthCheckConfig {
        default_interval: Duration::from_secs(10),
        failure_threshold: 2,
        check_timeout: Duration::from_secs(3),
        ..Default::default()
    })
    .quarantine_config(QuarantineConfig {
        failure_threshold: 2,
        max_recovery_attempts: 10,
        recovery_strategy: RecoveryStrategy {
            base_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(120),
            multiplier: 1.5,
        },
    })
    .build();
```

To observe health transitions:

```rust
let mut rx = manager.event_bus().subscribe();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            ResourceEvent::HealthChanged { resource_key, from, to } => {
                tracing::warn!(%resource_key, ?from, ?to, "health state changed");
            }
            ResourceEvent::Quarantined { resource_key, reason, .. } => {
                tracing::error!(%resource_key, %reason, "resource quarantined");
            }
            ResourceEvent::QuarantineReleased { resource_key, recovery_attempts } => {
                tracing::info!(%resource_key, recovery_attempts, "resource recovered");
            }
            _ => {}
        }
    }
});
```

---

## Custom Health Checks

To add a custom health check to your `Resource::Instance`:

```rust
use nebula_resource::{Context, HealthCheckable, HealthStatus, Result};

pub struct RedisClient { /* ... */ }

impl HealthCheckable for RedisClient {
    async fn health_check(&self) -> Result<HealthStatus> {
        let start = std::time::Instant::now();
        match self.ping().await {
            Ok(_) => Ok(HealthStatus::healthy().with_latency(start.elapsed())),
            Err(e) if e.is_connection_dropped() => {
                Ok(HealthStatus::unhealthy(format!("connection dropped: {e}")))
            }
            Err(e) if e.is_timeout() => Ok(HealthStatus::degraded(
                format!("slow response: {e}"),
                0.4, // 40% performance impact
            )),
            Err(e) => Ok(HealthStatus::unhealthy(e.to_string())),
        }
    }

    fn health_check_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
}
```

Register the instance with the health checker after creation (done automatically
by `Manager` when the instance type implements `HealthCheckable`).
