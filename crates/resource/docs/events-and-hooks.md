# nebula-resource — Events and Hooks

`nebula-resource` exposes two observability extension points:

- **EventBus** — a broadcast channel that emits structured `ResourceEvent` values
  on every lifecycle transition. Subscribe from any upstream crate without coupling
  to internals.
- **HookRegistry** — synchronous pre/post callbacks for acquire, release, create,
  and cleanup. Hooks can cancel create operations. Registered globally per `Manager`.

---

## Table of Contents

- [EventBus](#eventbus)
- [ResourceEvent Catalog](#resourceevent-catalog)
- [Subscribing to Events](#subscribing-to-events)
- [Filtered and Scoped Subscriptions](#filtered-and-scoped-subscriptions)
- [HookRegistry](#hookregistry)
- [ResourceHook Trait](#resourcehook-trait)
- [Built-in Hooks](#built-in-hooks)
- [Writing Custom Hooks](#writing-custom-hooks)
- [EventBus Backpressure](#eventbus-backpressure)

---

## EventBus

```rust
pub struct EventBus { /* ... */ }

impl EventBus {
    /// Create a bus with a fixed broadcast buffer.
    pub fn new(buffer_size: usize) -> Self;

    /// Create with an explicit backpressure policy.
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self;

    /// Fire-and-forget emit. Returns PublishOutcome (dropped / delivered count).
    pub fn emit(&self, event: ResourceEvent) -> PublishOutcome;

    /// Async emit — respects BackPressurePolicy::Block for bounded subscribers.
    pub async fn emit_async(&self, event: ResourceEvent) -> PublishOutcome;

    /// Subscribe to all events.
    pub fn subscribe(&self) -> EventSubscriber<ResourceEvent>;

    /// Subscribe with a filter closure.
    pub fn subscribe_filtered(
        &self,
        filter: EventFilter<ResourceEvent>,
    ) -> ScopedSubscriber;

    /// Subscribe to events for a specific SubscriptionScope.
    pub fn subscribe_scoped(&self, scope: SubscriptionScope) -> ScopedSubscriber;

    pub fn stats(&self) -> EventBusStats;
    pub fn buffer_size(&self) -> usize;
    pub fn policy(&self) -> &BackPressurePolicy;
    pub fn subscriber_count(&self) -> usize;
}
```

The `EventBus` used by `Manager` is accessible via `Manager::event_bus()`.
Pass a custom bus via `ManagerBuilder::event_bus(Arc::new(...))` to share it
across multiple managers or other crates.

---

## ResourceEvent Catalog

All events carry a `resource_key: ResourceKey` identifying the resource.

```rust
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// A new pool was registered.
    Created {
        resource_key: ResourceKey,
        scope: Scope,
    },

    /// An instance was checked out of the pool.
    Acquired {
        resource_key: ResourceKey,
        /// Time the caller waited for a semaphore permit.
        wait_duration: Duration,
    },

    /// An instance was returned to the pool.
    Released {
        resource_key: ResourceKey,
        /// Total time the instance was checked out.
        usage_duration: Duration,
    },

    /// Health state changed.
    HealthChanged {
        resource_key: ResourceKey,
        from: HealthState,
        to: HealthState,
    },

    /// All permits are taken and callers are waiting.
    PoolExhausted {
        resource_key: ResourceKey,
        waiters: usize,
    },

    /// An instance was permanently removed from the pool.
    CleanedUp {
        resource_key: ResourceKey,
        reason: CleanupReason,
    },

    /// A circuit breaker opened for an operation.
    CircuitBreakerOpen {
        resource_key: ResourceKey,
        operation: &'static str,  // "create" | "recycle"
        retry_after: Duration,
    },

    /// A circuit breaker closed (recovered).
    CircuitBreakerClosed {
        resource_key: ResourceKey,
        operation: &'static str,
    },

    /// Resource entered quarantine.
    Quarantined {
        resource_key: ResourceKey,
        reason: String,
        trigger: QuarantineTrigger,
        from_health: HealthState,
        to_health: HealthState,
    },

    /// Resource was released from quarantine after recovery.
    QuarantineReleased {
        resource_key: ResourceKey,
        recovery_attempts: u32,
    },

    /// Config was hot-reloaded successfully.
    ConfigReloaded {
        resource_key: ResourceKey,
        scope: Scope,
    },

    /// Config reload was rejected (e.g. pool had active connections).
    ConfigReloadRejected {
        resource_key: ResourceKey,
        error: String,
        had_existing_pool: bool,
    },

    /// An operation failed.
    Error {
        resource_key: ResourceKey,
        error: String,
    },

    /// A credential was rotated and the pool was updated.
    CredentialRotated {
        resource_key: ResourceKey,
        credential_key: nebula_core::CredentialKey,
        strategy: String,
    },
}
```

### `CleanupReason`

```rust
pub enum CleanupReason {
    Expired,          // max_lifetime exceeded
    IdleTimeout,      // idle_timeout exceeded
    HealthCheckFailed,
    Shutdown,
    Evicted,          // manually evicted via Manager API
    CredentialRotated,
    RecycleFailed,    // Resource::recycle returned Err
    Tainted,          // guard.taint() was called before drop
}
```

### `QuarantineTrigger`

```rust
pub enum QuarantineTrigger {
    HealthThresholdExceeded { consecutive_failures: u32 },
    Manual { reason: String },
}
```

---

## Subscribing to Events

Each call to `subscribe()` returns an independent `broadcast::Receiver`.
Slow subscribers do not affect the acquire path; see
[EventBus Backpressure](#eventbus-backpressure) for drop policy.

```rust
let mut rx = manager.event_bus().subscribe();

tokio::spawn(async move {
    loop {
        match rx.recv().await {
            Ok(event) => handle_event(event),
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "event subscriber lagged");
            }
            Err(RecvError::Closed) => break,
        }
    }
});
```

### SSE streaming (axum)

Each SSE connection gets its own receiver — no shared mutable state:

```rust
use axum::response::sse::{Event, Sse};
use futures::stream;

async fn resource_events(
    State(manager): State<Arc<Manager>>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = manager.event_bus().subscribe();
    let s = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_default();
                Some((Ok(Event::default().data(data)), rx))
            }
            Err(_) => None,
        }
    });
    Sse::new(s)
}
```

---

## Filtered and Scoped Subscriptions

Subscribe to a subset of events using a filter closure:

```rust
use nebula_resource::EventFilter;

// Only receive PoolExhausted events
let filter = EventFilter::new(|event: &ResourceEvent| {
    matches!(event, ResourceEvent::PoolExhausted { .. })
});
let mut rx = manager.event_bus().subscribe_filtered(filter);
```

Subscribe to events for a specific `SubscriptionScope`:

```rust
use nebula_resource::SubscriptionScope;

// Only events for a specific resource key prefix
let mut rx = manager.event_bus().subscribe_scoped(
    SubscriptionScope::Prefix("postgres".into())
);
```

---

## HookRegistry

Hooks run synchronously (but may be `async`) at fixed points in the acquire
and release lifecycle. Before-hooks can cancel the operation; after-hooks are
best-effort and cannot cancel.

```rust
pub struct HookRegistry { /* ... */ }

impl HookRegistry {
    pub fn new() -> Self;

    /// Register a hook. Hooks are stored in priority order (lower = first).
    pub fn register(&self, hook: Arc<dyn ResourceHook>);

    /// Snapshot of registered hooks in priority order.
    pub fn snapshot(&self) -> SmallVec<[Arc<dyn ResourceHook>; 4]>;

    /// Run all before-hooks. Returns Err if any hook cancels.
    pub async fn run_before(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
    ) -> Result<()>;

    /// Run all after-hooks. Errors are logged, never propagated.
    pub async fn run_after(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
        success: bool,
    );
}
```

Attach a `HookRegistry` to a `Manager` via `ManagerBuilder`:

```rust
use nebula_resource::{HookRegistry, ManagerBuilder, SlowAcquireHook};
use std::sync::Arc;

let mut registry = HookRegistry::new();
registry.register(Arc::new(SlowAcquireHook::new(Duration::from_millis(500))));

let manager = ManagerBuilder::new()
    // hooks are shared across all pools in this manager
    .build();
// For now pass the registry directly:
// manager.set_hooks(Arc::new(registry));
```

---

## ResourceHook Trait

```rust
pub trait ResourceHook: Send + Sync {
    fn name(&self) -> &str;

    /// Lower priority runs first. Default: 100.
    fn priority(&self) -> u32 { 100 }

    /// Which events this hook applies to.
    fn events(&self) -> Vec<HookEvent>;

    /// Which resources this hook applies to.
    fn filter(&self) -> HookFilter { HookFilter::All }

    /// Called before the event. Return Cancel to abort the operation.
    async fn before(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
    ) -> HookResult;

    /// Called after the event. `success` indicates whether the operation succeeded.
    /// Errors in after-hooks are logged and ignored.
    async fn after(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
        success: bool,
    );
}
```

### `HookEvent`

```rust
pub enum HookEvent {
    /// Before/after Pool::acquire (or Resource::create when no idle instance).
    Acquire,
    /// Before/after instance returned to pool.
    Release,
    /// Before/after Resource::create — before-hook can cancel.
    Create,
    /// Before/after Resource::cleanup — result ignored (irrevocable).
    Cleanup,
}
```

### `HookFilter`

```rust
pub enum HookFilter {
    /// Hook applies to all resources.
    All,
    /// Hook applies to exactly this resource ID.
    Resource(String),
    /// Hook applies to resources whose ID starts with this prefix.
    Prefix(String),
}

impl HookFilter {
    pub fn matches(&self, resource_id: &str) -> bool;
}
```

### `HookResult`

```rust
pub enum HookResult {
    /// Allow the operation to continue.
    Continue,
    /// Abort the operation with this error. Only effective in before-hooks.
    Cancel(Error),
}
```

---

## Built-in Hooks

### `AuditHook` (priority 10)

Logs all lifecycle events using `tracing`. Runs before any user hooks
due to its low priority value.

```rust
use nebula_resource::AuditHook;
use std::sync::Arc;

registry.register(Arc::new(AuditHook));
// Emits tracing::info! on before/after for Acquire, Release, Create, Cleanup.
```

### `SlowAcquireHook` (priority 90)

Emits a `tracing::warn!` when acquire wait time exceeds a threshold:

```rust
use nebula_resource::SlowAcquireHook;
use std::time::Duration;

registry.register(Arc::new(SlowAcquireHook::new(Duration::from_millis(200))));
// Warns: "slow acquire for 'postgres': waited 342ms (threshold: 200ms)"
```

---

## Writing Custom Hooks

```rust
use nebula_resource::{Context, Error, HookEvent, HookFilter, HookResult, ResourceHook};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Rate-limit hook: cancel acquire if active count exceeds per-tenant limit.
pub struct TenantRateLimitHook {
    max_per_tenant: u64,
    active: Arc<dashmap::DashMap<String, AtomicU64>>,
}

impl ResourceHook for TenantRateLimitHook {
    fn name(&self) -> &str { "tenant-rate-limit" }
    fn priority(&self) -> u32 { 50 }
    fn events(&self) -> Vec<HookEvent> { vec![HookEvent::Acquire, HookEvent::Release] }
    fn filter(&self) -> HookFilter { HookFilter::All }

    async fn before(
        &self,
        event: &HookEvent,
        _resource_id: &str,
        ctx: &Context,
    ) -> HookResult {
        if !matches!(event, HookEvent::Acquire) {
            return HookResult::Continue;
        }
        let tenant = match &ctx.tenant_id {
            Some(t) => t.clone(),
            None => return HookResult::Continue,
        };
        let count = self.active
            .entry(tenant.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);

        if count >= self.max_per_tenant {
            HookResult::Cancel(Error::configuration(
                format!("rate limit exceeded for tenant {tenant}"),
            ))
        } else {
            HookResult::Continue
        }
    }

    async fn after(
        &self,
        event: &HookEvent,
        _resource_id: &str,
        ctx: &Context,
        _success: bool,
    ) {
        if !matches!(event, HookEvent::Release) { return; }
        if let Some(tenant) = &ctx.tenant_id {
            if let Some(entry) = self.active.get(tenant) {
                entry.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}
```

---

## EventBus Backpressure

The `EventBus` uses a Tokio broadcast channel internally. When a subscriber
falls behind, events are dropped for that subscriber:

```rust
pub enum BackPressurePolicy {
    /// Drop events for lagging subscribers (broadcast default). Never blocks emit.
    DropOldest,
    /// Block emit until all subscribers catch up (only for bounded channels).
    Block,
}
```

`EventBus::emit` is always non-blocking. `EventBus::emit_async` respects the
policy for async contexts.

Subscribers that lag receive `RecvError::Lagged(n)` indicating how many
events were dropped. Handle this in your subscriber loop:

```rust
loop {
    match rx.recv().await {
        Ok(event) => process(event),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            metrics::counter!("resource_events_dropped").increment(n);
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
    }
}
```

For production SSE handlers or metrics collectors, use a buffer of at least
`4 * expected_peak_events_per_second` to avoid lagging under burst load.
The default `Manager` creates a bus with a buffer of 1024.
