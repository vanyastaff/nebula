# Nebula Observability Integration Guide

**Version**: 1.0
**Last Updated**: October 2025
**Status**: Stable

## Table of Contents

1. [Overview](#overview)
2. [Quick Start](#quick-start)
3. [Core Concepts](#core-concepts)
4. [Events](#events)
5. [Hooks](#hooks)
6. [Metrics](#metrics)
7. [Operation Tracking](#operation-tracking)
8. [Integration Patterns](#integration-patterns)
9. [Examples](#examples)
10. [Best Practices](#best-practices)

---

## Overview

Nebula's unified observability system provides a **zero-cost abstraction** for monitoring, metrics, and event tracking across all nebula crates. Built on top of `nebula-log`, it offers:

✅ **Lock-free event emission** - High-throughput, low-latency
✅ **Panic-safe hooks** - Hooks cannot crash your application
✅ **Composable architecture** - Mix and match hooks as needed
✅ **Type-safe events** - Compile-time guarantees
✅ **Prometheus integration** - Built-in metrics export

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Your Application                        │
└────────────────────────────┬────────────────────────────────┘
                             │ emit_event()
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                   Observability Registry                     │
│                    (Lock-Free, Panic-Safe)                   │
└───┬────────────┬─────────────┬──────────────┬───────────────┘
    │            │             │              │
    ▼            ▼             ▼              ▼
┌────────┐  ┌─────────┐  ┌──────────┐  ┌──────────────┐
│Logging │  │ Metrics │  │ Tracing  │  │ Custom Hooks │
│ Hook   │  │  Hook   │  │   Hook   │  │   (yours)    │
└────────┘  └─────────┘  └──────────┘  └──────────────┘
```

---

## Quick Start

### 5-Minute Setup

**Step 1**: Add dependency
```toml
[dependencies]
nebula-log = { version = "0.1", features = ["observability"] }
```

**Step 2**: Register hooks at startup
```rust
use nebula_log::observability::{register_hook, LoggingHook};
use std::sync::Arc;

fn main() {
    // Register logging hook
    register_hook(Arc::new(LoggingHook::default()));

    // Your application code...
}
```

**Step 3**: Emit events
```rust
use nebula_log::observability::{emit_event, OperationStarted};

emit_event(&OperationStarted {
    operation_id: "my_op_123",
    operation_type: "validation",
});
```

That's it! Events are now logged automatically.

---

## Core Concepts

### Events

**Events** represent things that happen in your application. They are:
- **Immutable** - Once created, cannot change
- **Type-safe** - Implement `ObservabilityEvent` trait
- **Zero-cost** - No heap allocation for simple events
- **Composable** - Can carry structured data

### Hooks

**Hooks** process events. They are:
- **Registered globally** - Single registration at startup
- **Panic-safe** - Cannot crash the application
- **Concurrent** - Process events in parallel
- **Composable** - Multiple hooks can coexist

### Registry

The **Registry** is the central coordinator:
- **Lock-free reads** - No contention on event emission
- **ArcSwap-based** - Atomic pointer swapping
- **Thread-safe** - Safe to use from multiple threads
- **Performance-first** - Designed for high-throughput

---

## Events

### Built-in Events

Nebula provides standard events for common operations:

#### Operation Lifecycle Events

```rust
use nebula_log::observability::{
    OperationStarted,
    OperationCompleted,
    OperationFailed,
    emit_event,
};

// Operation start
emit_event(&OperationStarted {
    operation_id: "op_123",
    operation_type: "database_query",
});

// Operation success
emit_event(&OperationCompleted {
    operation_id: "op_123",
    duration_ms: 42,
});

// Operation failure
emit_event(&OperationFailed {
    operation_id: "op_123",
    error_message: "Connection timeout",
    duration_ms: 5000,
});
```

### Custom Events

Create domain-specific events by implementing `ObservabilityEvent`:

```rust
use nebula_log::observability::ObservabilityEvent;
use serde_json::Value;

/// Custom event for cache operations
struct CacheEvent {
    cache_name: String,
    operation: CacheOperation,
    key: String,
    hit: bool,
}

enum CacheOperation {
    Get,
    Set,
    Delete,
}

impl ObservabilityEvent for CacheEvent {
    fn name(&self) -> &str {
        "cache_operation"
    }

    fn data(&self) -> Option<Value> {
        Some(serde_json::json!({
            "cache": self.cache_name,
            "operation": format!("{:?}", self.operation),
            "key": self.key,
            "hit": self.hit,
        }))
    }
}

// Usage
emit_event(&CacheEvent {
    cache_name: "user_cache".to_string(),
    operation: CacheOperation::Get,
    key: "user_123".to_string(),
    hit: true,
});
```

### Event Design Guidelines

✅ **DO**:
- Use descriptive event names (`user_login`, `cache_miss`)
- Include relevant context in event data
- Keep events immutable
- Use enums for operation types

❌ **DON'T**:
- Emit events in hot loops (use sampling)
- Include sensitive data (passwords, tokens)
- Create events with unbounded data
- Emit events synchronously in critical paths

---

## Hooks

### Built-in Hooks

#### LoggingHook

Logs events using `tracing`:

```rust
use nebula_log::observability::{register_hook, LoggingHook};
use std::sync::Arc;

// Default: logs at INFO level
register_hook(Arc::new(LoggingHook::default()));

// Custom level
register_hook(Arc::new(LoggingHook::new(tracing::Level::DEBUG)));
```

**Output**:
```
INFO observability_event: validation field="email" valid=true
```

#### MetricsHook (Coming Soon)

Exports metrics to Prometheus:

```rust
use nebula_log::observability::{register_hook, MetricsHook};

register_hook(Arc::new(MetricsHook::new()));
```

### Custom Hooks

Implement `ObservabilityHook` for custom processing:

```rust
use nebula_log::observability::{ObservabilityHook, ObservabilityEvent};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Hook that counts events by type
struct CountingHook {
    total_events: AtomicUsize,
}

impl ObservabilityHook for CountingHook {
    fn initialize(&self) {
        println!("CountingHook initialized");
    }

    fn on_event(&self, event: &dyn ObservabilityEvent) {
        self.total_events.fetch_add(1, Ordering::Relaxed);

        println!("Event #{}: {}",
            self.total_events.load(Ordering::Relaxed),
            event.name()
        );
    }

    fn shutdown(&self) {
        println!("Total events processed: {}",
            self.total_events.load(Ordering::Relaxed)
        );
    }
}

// Register
register_hook(Arc::new(CountingHook {
    total_events: AtomicUsize::new(0),
}));
```

### Hook Panic Safety

**All hooks are panic-safe** - if a hook panics, it's caught and logged:

```rust
impl ObservabilityHook for PanickingHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {
        panic!("Oops!"); // This won't crash your app!
    }
}
```

**Output**:
```
ERROR Hook panicked while processing event: event_name="validation"
```

Other hooks continue processing normally.

---

## Metrics

### Using Metrics in Your Crate

```rust
use metrics::{counter, gauge, histogram};

// Counter: monotonically increasing
counter!("nebula.myapp.requests_total", "endpoint" => "api").increment(1);

// Gauge: value that can go up or down
gauge!("nebula.myapp.active_connections").set(42.0);

// Histogram: distribution of values
histogram!("nebula.myapp.request_duration_ms").record(123.45);
```

### Metric Naming Convention

Follow the pattern: `nebula.{crate}.{component}.{metric}`

**Examples**:
- `nebula.memory.allocator.bytes_allocated`
- `nebula.resource.pool.connections_active`
- `nebula.validator.cache.hit_rate`
- `nebula.resilience.circuit_breaker.state`

### Labels

Use labels sparingly to avoid high cardinality:

✅ **Good** (low cardinality):
```rust
counter!("nebula.api.requests", "method" => "GET").increment(1);
```

❌ **Bad** (high cardinality):
```rust
// DON'T: user_id creates millions of metrics
counter!("requests", "user_id" => user_id).increment(1);
```

---

## Operation Tracking

### OperationTracker

Track operation lifecycle automatically:

```rust
use nebula_log::observability::OperationTracker;

fn process_request(request_id: &str) -> Result<(), String> {
    // Automatically emits OperationStarted
    let tracker = OperationTracker::start(request_id, "http_request");

    // Do work...
    let result = perform_work();

    // Automatically emits OperationCompleted or OperationFailed on drop
    match result {
        Ok(data) => Ok(data),
        Err(e) => {
            tracker.fail(&e.to_string());
            Err(e)
        }
    }
}
```

**Benefits**:
- Automatic timing
- Guaranteed completion events (even on panic)
- No manual cleanup needed

---

## Integration Patterns

### Pattern 1: Resource Lifecycle

```rust
use nebula_log::observability::emit_event;

struct DatabasePool {
    // ...
}

impl DatabasePool {
    pub fn acquire(&self) -> Result<Connection, Error> {
        emit_event(&ResourceAcquired {
            resource_type: "database_connection",
            pool_id: self.id,
        });

        // Acquire connection...
    }

    pub fn release(&self, conn: Connection) {
        emit_event(&ResourceReleased {
            resource_type: "database_connection",
            pool_id: self.id,
        });

        // Release connection...
    }
}
```

### Pattern 2: Validation Pipeline

```rust
use nebula_log::observability::emit_event;

fn validate_user_input(input: &UserInput) -> Result<(), ValidationError> {
    let tracker = OperationTracker::start("validation", "user_input");

    // Validate each field
    for field in &input.fields {
        let result = validate_field(field);

        emit_event(&FieldValidation {
            field_name: field.name,
            valid: result.is_ok(),
            rule: field.rule,
        });

        result?;
    }

    Ok(())
}
```

### Pattern 3: Cache Operations

```rust
impl Cache {
    pub fn get(&self, key: &str) -> Option<Value> {
        let result = self.inner.get(key);

        emit_event(&CacheAccess {
            operation: "get",
            key: key.to_string(),
            hit: result.is_some(),
        });

        result
    }
}
```

---

## Examples

### Complete Example: API Server

```rust
use nebula_log::observability::{
    register_hook, LoggingHook, emit_event, OperationTracker,
    ObservabilityEvent,
};
use std::sync::Arc;

// Custom event
struct ApiRequest {
    method: String,
    path: String,
    status: u16,
}

impl ObservabilityEvent for ApiRequest {
    fn name(&self) -> &str { "api_request" }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "method": self.method,
            "path": self.path,
            "status": self.status,
        }))
    }
}

fn main() {
    // Setup observability
    register_hook(Arc::new(LoggingHook::default()));

    // Handle request
    handle_request("GET", "/users/123");
}

fn handle_request(method: &str, path: &str) {
    let tracker = OperationTracker::start("api_request", method);

    // Process request
    let status = process(method, path);

    // Emit custom event
    emit_event(&ApiRequest {
        method: method.to_string(),
        path: path.to_string(),
        status,
    });
}

fn process(method: &str, path: &str) -> u16 {
    // Simulate processing
    200
}
```

---

## Best Practices

### Performance

✅ **Event emission is fast** (~50ns overhead)
✅ **Use in hot paths** - designed for high throughput
❌ **Don't emit millions of events per second** - use sampling

### Sampling

For high-frequency events, use sampling:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn maybe_emit_event(event: &impl ObservabilityEvent) {
    let count = SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Emit only 1 in 100 events
    if count % 100 == 0 {
        emit_event(event);
    }
}
```

### Testing

Mock hooks for testing:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestHook {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl ObservabilityHook for TestHook {
        fn on_event(&self, event: &dyn ObservabilityEvent) {
            self.events.lock().unwrap().push(event.name().to_string());
        }
    }

    #[test]
    fn test_emits_event() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let hook = TestHook { events: events.clone() };

        register_hook(Arc::new(hook));
        emit_event(&MyEvent);

        assert_eq!(events.lock().unwrap().len(), 1);
    }
}
```

### Documentation

Document your events:

```rust
/// Event emitted when a user logs in successfully.
///
/// # Fields
/// - `user_id`: Unique identifier for the user
/// - `ip_address`: Client IP address
/// - `timestamp`: Login timestamp
///
/// # Hook Recommendations
/// - LoggingHook: Logs at INFO level
/// - SecurityHook: Triggers anomaly detection
/// - MetricsHook: Increments login counter
struct UserLoginEvent {
    user_id: String,
    ip_address: String,
    timestamp: i64,
}
```

---

## Next Steps

- 📖 Read [OBSERVABILITY_MIGRATION.md](./OBSERVABILITY_MIGRATION.md) for migrating existing code
- 📖 Read [OBSERVABILITY_BEST_PRACTICES.md](./OBSERVABILITY_BEST_PRACTICES.md) for advanced patterns
- 💻 Check [examples/observability_hooks.rs](../crates/nebula-log/examples/observability_hooks.rs)
- 💻 Check [examples/prometheus_integration.rs](../crates/nebula-log/examples/prometheus_integration.rs)

---

## Support

- **Issues**: https://github.com/vanyastaff/nebula/issues
- **Discussions**: https://github.com/vanyastaff/nebula/discussions
- **Documentation**: https://docs.rs/nebula-log

---

**Last Updated**: October 2025
**Maintainers**: Nebula Team
