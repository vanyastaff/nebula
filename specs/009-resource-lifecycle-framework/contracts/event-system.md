# Contract: Event System

**Status**: Planned (Phase 3)
**Location**: `crates/resource/src/events.rs` (to be created)

## ResourceEvent Enum

All lifecycle events emitted by the resource system.

```rust
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// New resource type registered with the manager
    Created {
        resource_id: String,
        scope: Scope,
    },

    /// Instance acquired from pool
    Acquired {
        resource_id: String,
        pool_stats: PoolStats,
    },

    /// Instance returned to pool
    Released {
        resource_id: String,
        usage_duration: Duration,
    },

    /// Health state changed
    HealthChanged {
        resource_id: String,
        from: HealthState,
        to: HealthState,
    },

    /// Pool capacity reached, callers waiting
    PoolExhausted {
        resource_id: String,
        waiters: usize,
    },

    /// Instance permanently removed from pool
    CleanedUp {
        resource_id: String,
        reason: CleanupReason,
    },

    /// Error during lifecycle operation
    Error {
        resource_id: String,
        error: String,  // Stringified to avoid Clone requirement on Error
    },
}
```

## CleanupReason

```rust
#[derive(Debug, Clone)]
pub enum CleanupReason {
    Expired,           // max_lifetime exceeded
    IdleTimeout,       // idle_timeout exceeded
    HealthCheckFailed, // Failed is_valid() or health_check()
    Shutdown,          // Manager shutting down
    Evicted,           // Removed by auto-scaler [P8]
    RecycleFailed,     // recycle() returned error
}
```

## EventBus

```rust
pub struct EventBus {
    sender: broadcast::Sender<ResourceEvent>,
}

impl EventBus {
    /// Create event bus with configurable buffer size.
    /// Default buffer: 1024 events.
    pub fn new(buffer_size: usize) -> Self;

    /// Emit an event to all subscribers.
    /// Non-blocking — if no subscribers, event is dropped.
    pub fn emit(&self, event: ResourceEvent);

    /// Subscribe to all events. Returns a receiver.
    /// Lagging subscribers receive RecvError::Lagged.
    pub fn subscribe(&self) -> broadcast::Receiver<ResourceEvent>;
}
```

## Emission Points

| Source | Event | Trigger |
|--------|-------|---------|
| Manager::register | `Created` | Successful registration |
| Pool::acquire | `Acquired` | Instance handed to caller |
| Pool::release | `Released` | Instance returned to pool |
| HealthChecker | `HealthChanged` | State transition detected |
| Pool::acquire | `PoolExhausted` | Semaphore fully acquired |
| Pool::maintain | `CleanedUp` | Instance evicted (expired, idle, failed) |
| Pool::shutdown | `CleanedUp(Shutdown)` | Shutdown cleanup |
| Manager/Pool | `Error` | Any operation failure |

## Invariants

1. Events are emitted AFTER the operation completes (not before).
2. Event emission MUST NOT block the operation — fire-and-forget via broadcast.
3. If no subscribers exist, events are silently dropped (no error).
4. Lagging subscribers lose oldest events (bounded buffer, no unbounded growth).
5. `ResourceEvent` implements `Clone` (required by broadcast channel).
6. Error events contain stringified errors (not original Error, which may not be Clone).

## Integration with Metrics (Phase 3)

```rust
// MetricsCollector subscribes to EventBus
pub struct MetricsCollector {
    receiver: broadcast::Receiver<ResourceEvent>,
}

impl MetricsCollector {
    /// Start background task that processes events and updates metrics.
    pub async fn run(mut self) {
        while let Ok(event) = self.receiver.recv().await {
            match event {
                ResourceEvent::Acquired { .. } => counter!("resource.acquire.total").increment(1),
                ResourceEvent::Released { usage_duration, .. } =>
                    histogram!("resource.usage.duration_seconds").record(usage_duration),
                // ... etc
            }
        }
    }
}
```
