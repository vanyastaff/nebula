# Pool Configuration Guide

This guide covers how to configure and use the `Pool<R>` type from the nebula-resource crate.

## PoolConfig Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `min_size` | `usize` | 1 | Minimum idle instances the pool maintains via maintenance |
| `max_size` | `usize` | 10 | Maximum concurrent instances (idle + active). Must be > 0 |
| `acquire_timeout` | `Duration` | 30s | How long `acquire()` waits for a permit before returning `PoolExhausted` |
| `idle_timeout` | `Duration` | 600s | Idle instances older than this are evicted during maintenance |
| `max_lifetime` | `Duration` | 3600s | Instances older than this are destroyed on next use or maintenance |
| `validation_interval` | `Duration` | 30s | Interval hint for external validation (not used internally by Pool) |
| `maintenance_interval` | `Option<Duration>` | None | If set, spawns a background task that evicts expired instances and replenishes to `min_size` |
| `strategy` | `PoolStrategy` | Fifo | How idle instances are selected on acquire |

## FIFO vs LIFO Strategy

### FIFO (First In, First Out) -- Default

Returns the **oldest** idle instance first. Every instance gets used roughly equally.

**Use when:**
- You want even wear across all instances (e.g., database connections with query caches)
- Resource cost is uniform and you want predictable behavior
- You do not benefit from temporal locality

```rust
let config = PoolConfig {
    strategy: PoolStrategy::Fifo,
    ..Default::default()
};
```

### LIFO (Last In, First Out)

Returns the **most recently used** idle instance first. Keeps a "hot" working set while letting unused instances expire naturally via `idle_timeout`.

**Use when:**
- You want a small hot working set (e.g., under bursty traffic)
- Instances benefit from warm caches or keep-alive
- You set `min_size` low relative to `max_size` and want idle instances to self-evict

```rust
let config = PoolConfig {
    strategy: PoolStrategy::Lifo,
    min_size: 1,
    max_size: 20,
    idle_timeout: Duration::from_secs(60),
    ..Default::default()
};
```

## Maintenance

When `maintenance_interval` is set, the pool spawns a background task that:

1. **Evicts** idle instances that exceed `idle_timeout` or `max_lifetime`
2. **Replenishes** the pool back to `min_size` if total instances (idle + active) have fallen below it

The task is cancelled automatically on `pool.shutdown()`.

```rust
let config = PoolConfig {
    min_size: 3,
    max_size: 20,
    maintenance_interval: Some(Duration::from_secs(30)),
    idle_timeout: Duration::from_secs(120),
    ..Default::default()
};
```

Without `maintenance_interval`, eviction only happens lazily when an expired instance is selected during `acquire()`.

## Acquire / Release Flow

1. `acquire()` obtains a semaphore permit (bounded to `max_size`)
2. Tries to pop an idle instance (FIFO: front, LIFO: back)
3. If the instance is expired, it is cleaned up and the next one is tried
4. If the instance is valid (`is_valid()` returns `Ok(true)`), it is returned in a `Guard`
5. If no idle instance is available, `create()` builds a new one
6. When the `Guard` is dropped, `recycle()` runs; on success the instance goes back to idle; on failure it is cleaned up

If no permit is available within `acquire_timeout`, the error `Error::PoolExhausted` is returned.

## Sizing Recommendations

### Connection pools (databases, Redis)

```rust
PoolConfig {
    min_size: 2,          // keep warm connections ready
    max_size: 20,         // match your DB's max_connections / app instances
    acquire_timeout: Duration::from_secs(5),
    idle_timeout: Duration::from_secs(300),
    max_lifetime: Duration::from_secs(1800),
    maintenance_interval: Some(Duration::from_secs(60)),
    strategy: PoolStrategy::Fifo,  // even distribution
    ..Default::default()
}
```

### Bursty HTTP clients

```rust
PoolConfig {
    min_size: 0,          // no idle cost when quiet
    max_size: 50,         // handle bursts
    acquire_timeout: Duration::from_secs(10),
    idle_timeout: Duration::from_secs(30),
    max_lifetime: Duration::from_secs(600),
    maintenance_interval: Some(Duration::from_secs(15)),
    strategy: PoolStrategy::Lifo,  // reuse hot connections
    ..Default::default()
}
```

### Heavyweight resources (ML models, large caches)

```rust
PoolConfig {
    min_size: 1,          // always keep one loaded
    max_size: 3,          // expensive, limit concurrency
    acquire_timeout: Duration::from_secs(30),
    idle_timeout: Duration::from_secs(3600),
    max_lifetime: Duration::from_secs(7200),
    maintenance_interval: Some(Duration::from_secs(300)),
    strategy: PoolStrategy::Lifo,
    ..Default::default()
}
```

## Event Bus Integration

Wire an `EventBus` into the pool to observe lifecycle events:

```rust
use std::sync::Arc;
use nebula_resource::events::EventBus;

let event_bus = Arc::new(EventBus::new(256));
let mut rx = event_bus.subscribe();

let pool = Pool::with_event_bus(resource, config, pool_config, Some(event_bus))?;

// In another task:
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            ResourceEvent::Acquired { resource_id, .. } => { /* log */ }
            ResourceEvent::Released { resource_id, usage_duration, .. } => { /* metrics */ }
            ResourceEvent::PoolExhausted { resource_id, .. } => { /* alert */ }
            ResourceEvent::CleanedUp { resource_id, reason, .. } => { /* audit */ }
            _ => {}
        }
    }
});
```

Events emitted by the pool:

| Event | When |
|---|---|
| `Acquired` | Instance successfully checked out |
| `Released` | Instance returned to pool (includes usage duration) |
| `PoolExhausted` | No permit available, caller will wait or timeout |
| `CleanedUp` | Instance permanently removed (with reason: Expired, Shutdown, RecycleFailed, Evicted) |
| `Error` | Any error during acquire |

## Shutdown

Call `pool.shutdown()` to:
1. Cancel the maintenance background task
2. Close the semaphore (new acquires fail immediately with `Error::Internal`)
3. Clean up all idle instances via `Resource::cleanup()`
4. Mark the pool as shut down so any in-flight guards clean up their instances on drop instead of returning them to idle

When using `Manager`, `manager.shutdown()` shuts down all pools in reverse dependency order.
