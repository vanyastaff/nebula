# Nebula Observability Best Practices

**Version**: 1.0
**Last Updated**: October 2025
**Audience**: Nebula crate maintainers and contributors

## Table of Contents

1. [Metric Design](#metric-design)
2. [Event Design](#event-design)
3. [Hook Implementation](#hook-implementation)
4. [Performance Considerations](#performance-considerations)
5. [Testing Observability](#testing-observability)
6. [Naming Conventions](#naming-conventions)
7. [Common Patterns](#common-patterns)
8. [Anti-Patterns](#anti-patterns)

---

## Metric Design

### Choosing the Right Metric Type

| Metric Type | Use When | Example |
|-------------|----------|---------|
| **Counter** | Value only increases | Request count, error count |
| **Gauge** | Value can go up/down | Active connections, queue size |
| **Histogram** | Need distribution | Request duration, payload size |

### Counter Best Practices

✅ **DO**:
```rust
// Good: counts total requests
counter!("nebula.api.requests_total",
    "method" => "GET",
    "status" => "200"
).increment(1);
```

❌ **DON'T**:
```rust
// Bad: using counter for current value
counter!("active_connections").set(42); // Use gauge!
```

### Gauge Best Practices

✅ **DO**:
```rust
// Good: tracks current state
gauge!("nebula.pool.connections_active").set(pool.active_count() as f64);
gauge!("nebula.cache.memory_bytes").set(cache.size_bytes() as f64);
```

❌ **DON'T**:
```rust
// Bad: using gauge for累积值
gauge!("total_requests").increment(1); // Use counter!
```

### Histogram Best Practices

✅ **DO**:
```rust
// Good: records distribution
let start = Instant::now();
let result = process_request();
histogram!("nebula.request.duration_ms")
    .record(start.elapsed().as_millis() as f64);
```

❌ **DON'T**:
```rust
// Bad: recording individual values
histogram!("user_id").record(user_id); // Use labels on counter!
```

### Label Cardinality

**Golden Rule**: Keep label cardinality LOW (< 100 unique values per label).

✅ **Low Cardinality** (Good):
- HTTP method: GET, POST, PUT, DELETE (4 values)
- Status code: 200, 404, 500, etc. (~20 values)
- Resource type: database, cache, file (< 10 values)

❌ **High Cardinality** (Bad):
- User ID: millions of unique values
- Request ID: unbounded
- Timestamps: infinite
- Email addresses: unbounded

**Example**:
```rust
// ✅ GOOD: Low cardinality labels
counter!("nebula.requests",
    "method" => "GET",      // ~10 values
    "endpoint" => "/users"  // ~50 values
).increment(1);

// ❌ BAD: High cardinality labels
counter!("nebula.requests",
    "user_id" => user_id,     // Millions!
    "request_id" => req_id    // Infinite!
).increment(1);
```

---

## Event Design

### Event Granularity

**Rule of Thumb**: One event = one meaningful thing happened.

✅ **Good Granularity**:
```rust
// Clear, actionable events
emit_event(&DatabaseQueryStarted { query_id });
emit_event(&DatabaseQueryCompleted { query_id, rows });
emit_event(&DatabaseQueryFailed { query_id, error });
```

❌ **Too Granular**:
```rust
// Too much noise
emit_event(&VariableAssigned { var: "x", value: 42 });
emit_event(&FunctionEntered { name: "foo" });
emit_event(&LoopIteration { iteration: 1 });
```

❌ **Too Coarse**:
```rust
// Not enough detail
emit_event(&SomethingHappened);
```

### Event Data

Include **just enough context** to be useful, not everything.

✅ **Good**:
```rust
struct UserLoginEvent {
    user_id: String,        // Who
    timestamp: i64,         // When
    ip_address: String,     // Where
    success: bool,          // What happened
}
```

❌ **Too Much**:
```rust
struct UserLoginEvent {
    user_id: String,
    username: String,       // Redundant with user_id
    email: String,          // PII, not needed
    password_hash: String,  // NEVER include!
    full_request: String,   // Too much data
    session_data: String,   // Not relevant
}
```

### Sensitive Data

**Never include**:
- Passwords (even hashed)
- API keys / tokens
- Credit card numbers
- SSN / personal IDs
- Full email addresses (use hash or domain only)

✅ **Safe**:
```rust
struct PaymentEvent {
    transaction_id: String,
    amount_cents: u64,
    currency: String,
    card_last_four: String,  // Only last 4 digits
    success: bool,
}
```

❌ **Dangerous**:
```rust
struct PaymentEvent {
    card_number: String,     // NEVER!
    cvv: String,             // NEVER!
    full_name: String,       // PII
}
```

---

## Hook Implementation

### Panic Safety

**All hooks MUST be panic-safe**. The registry catches panics, but follow these guidelines:

✅ **DO**:
```rust
impl ObservabilityHook for MyHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Safe: no unwrap, no panic
        if let Some(data) = event.data() {
            self.process(data);
        }
    }
}
```

❌ **DON'T**:
```rust
impl ObservabilityHook for MyHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Risky: can panic!
        let data = event.data().unwrap();
        self.process(data);
    }
}
```

### Performance

Hooks are called **synchronously** on the emitting thread. Keep them FAST.

**Target**: < 1µs per event

✅ **Fast**:
```rust
impl ObservabilityHook for FastHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Fast: atomic increment
        self.counter.fetch_add(1, Ordering::Relaxed);

        // Fast: lock-free channel send
        let _ = self.sender.try_send(event.name());
    }
}
```

❌ **Slow**:
```rust
impl ObservabilityHook for SlowHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // SLOW: Network I/O
        let _ = reqwest::blocking::get("http://collector/event");

        // SLOW: Database write
        let _ = db.insert(event);

        // SLOW: Heavy computation
        compute_analytics(event);
    }
}
```

**Solution for Slow Operations**: Use async processing:

```rust
impl ObservabilityHook for AsyncHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Fast: enqueue for async processing
        let event_data = event.data().cloned();
        self.tx.try_send(event_data).ok();
        // Background thread processes queue
    }
}
```

### Thread Safety

Hooks are called from **multiple threads concurrently**. Use appropriate synchronization:

✅ **Thread-Safe**:
```rust
struct ThreadSafeHook {
    counter: AtomicUsize,           // ✅ Atomic
    queue: Sender<Event>,           // ✅ Channel
    cache: DashMap<String, Value>,  // ✅ Concurrent map
}
```

❌ **NOT Thread-Safe**:
```rust
struct UnsafeHook {
    counter: Cell<usize>,           // ❌ Not thread-safe
    data: RefCell<Vec<Event>>,      // ❌ Not thread-safe
}
```

---

## Performance Considerations

### Event Emission Cost

**Measured overhead per event**:
- Event creation: ~10ns
- Registry dispatch: ~50ns
- LoggingHook processing: ~500ns
- **Total**: ~560ns per event

### High-Frequency Events

For events in hot paths (> 10,000/sec), use **sampling**:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn emit_sampled(event: &impl ObservabilityEvent) {
    let count = SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Emit 1% of events
    if count % 100 == 0 {
        emit_event(event);
    }
}
```

### Conditional Compilation

For ultra-hot paths, use feature flags:

```rust
#[cfg(feature = "observability")]
emit_event(&AllocationEvent { size });

#[cfg(not(feature = "observability"))]
let _ = (); // No-op in release builds
```

### Zero-Cost Events

Design events to be **zero-allocation** when possible:

```rust
// ✅ Zero-allocation event
struct SimpleEvent<'a> {
    name: &'a str,
    count: u64,
}

impl ObservabilityEvent for SimpleEvent<'_> {
    fn name(&self) -> &str { self.name }
    fn data(&self) -> Option<Value> { None } // No JSON!
}
```

---

## Testing Observability

### Unit Testing Events

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let event = MyEvent {
            field: "test".to_string(),
            value: 42,
        };

        assert_eq!(event.name(), "my_event");
        assert!(event.data().is_some());
    }

    #[test]
    fn test_event_data_format() {
        let event = MyEvent {
            field: "test".to_string(),
            value: 42,
        };

        let data = event.data().unwrap();
        assert_eq!(data["field"], "test");
        assert_eq!(data["value"], 42);
    }
}
```

### Integration Testing Hooks

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    struct TestHook {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl ObservabilityHook for TestHook {
        fn on_event(&self, event: &dyn ObservabilityEvent) {
            self.events.lock().unwrap()
                .push(event.name().to_string());
        }
    }

    #[test]
    fn test_hook_receives_events() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let hook = Arc::new(TestHook { events: events.clone() });

        register_hook(hook);
        emit_event(&MyEvent);

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], "my_event");

        shutdown_hooks();
    }
}
```

### Performance Testing

```rust
#[bench]
fn bench_event_emission(b: &mut Bencher) {
    register_hook(Arc::new(NoopHook));

    b.iter(|| {
        emit_event(&SimpleEvent);
    });
}
```

---

## Naming Conventions

### Metric Names

**Format**: `nebula.{crate}.{component}.{metric}`

**Examples**:
```
nebula.memory.allocator.bytes_allocated
nebula.memory.allocator.bytes_freed
nebula.memory.cache.hit_rate
nebula.resource.pool.connections_active
nebula.resource.pool.connections_idle
nebula.validator.combinators.cache_hits
nebula.resilience.circuit_breaker.state
```

### Event Names

Use **past tense**, dot-separated lowercase:

**Good**:
- `user.logged_in`
- `database.query.completed`
- `cache.entry.evicted`
- `validation.failed`

**Bad**:
- `UserLogIn` (not snake_case)
- `database_query` (present tense)
- `CACHE_HIT` (not lowercase)

### Label Names

Use **lowercase, underscores**:

**Good**:
- `method`, `status_code`, `endpoint`
- `resource_type`, `pool_id`
- `error_type`, `severity`

**Bad**:
- `Method`, `StatusCode` (not lowercase)
- `resourceType`, `poolId` (not snake_case)

---

## Common Patterns

### Pattern: Resource Lifecycle

```rust
impl Pool {
    pub fn acquire(&self) -> Result<Resource> {
        emit_event(&ResourceAcquireStarted { pool_id: self.id });

        let resource = self.try_acquire()?;

        emit_event(&ResourceAcquireCompleted {
            pool_id: self.id,
            resource_id: resource.id,
        });

        gauge!("nebula.pool.resources_active").increment(1.0);

        Ok(resource)
    }

    pub fn release(&self, resource: Resource) {
        emit_event(&ResourceReleased {
            pool_id: self.id,
            resource_id: resource.id,
        });

        gauge!("nebula.pool.resources_active").decrement(1.0);
    }
}
```

### Pattern: Operation Timing

```rust
pub fn process_request(req: Request) -> Result<Response> {
    let start = Instant::now();

    let result = do_work(req);

    let duration_ms = start.elapsed().as_millis() as f64;

    histogram!("nebula.request.duration_ms",
        "endpoint" => req.path.as_str()
    ).record(duration_ms);

    result
}
```

### Pattern: Error Tracking

```rust
pub fn fallible_operation() -> Result<()> {
    match dangerous_work() {
        Ok(val) => {
            counter!("nebula.operation.success").increment(1);
            Ok(val)
        }
        Err(e) => {
            counter!("nebula.operation.errors",
                "error_type" => error_type(&e)
            ).increment(1);

            emit_event(&OperationFailed {
                error: e.to_string(),
            });

            Err(e)
        }
    }
}
```

---

## Anti-Patterns

### ❌ Anti-Pattern: Event Spam

```rust
// DON'T: Emit events in tight loops
for item in items {
    emit_event(&ItemProcessed { item }); // Could be millions!
}
```

**Fix**: Batch or sample:
```rust
// DO: Emit batch event
emit_event(&ItemsBatchProcessed {
    count: items.len(),
    total_size: items.iter().map(|i| i.size).sum(),
});
```

### ❌ Anti-Pattern: High-Cardinality Labels

```rust
// DON'T: Use unbounded values as labels
counter!("requests", "user_id" => user_id).increment(1);
```

**Fix**: Use fixed categories:
```rust
// DO: Use user tier instead
counter!("requests", "user_tier" => "premium").increment(1);
```

### ❌ Anti-Pattern: Blocking in Hooks

```rust
// DON'T: Block in hook
impl ObservabilityHook for BadHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Blocks event emission!
        let _ = self.http_client.post("/events")
            .json(&event)
            .send();
    }
}
```

**Fix**: Use async channel:
```rust
// DO: Async processing
impl ObservabilityHook for GoodHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        let _ = self.tx.try_send(event.data());
        // Background thread sends to HTTP
    }
}
```

### ❌ Anti-Pattern: Over-Engineering Events

```rust
// DON'T: Overly complex events
struct MegaEvent {
    // 50+ fields
    field1: String,
    field2: i64,
    // ... many more
    nested_data: HashMap<String, Vec<ComplexType>>,
}
```

**Fix**: Simplify:
```rust
// DO: Focused events
struct SimpleEvent {
    operation: String,
    success: bool,
    duration_ms: u64,
}
```

---

## Summary Checklist

Before releasing observability code, verify:

- [ ] Events have clear, past-tense names
- [ ] Events contain no sensitive data
- [ ] Metrics follow naming convention: `nebula.{crate}.{component}.{metric}`
- [ ] Labels have low cardinality (< 100 values)
- [ ] Hooks are panic-safe
- [ ] Hooks complete in < 1µs
- [ ] High-frequency events use sampling
- [ ] Tests cover event emission and hook processing
- [ ] Documentation explains when events are emitted
- [ ] Performance impact measured and acceptable

---

## References

- [Prometheus Best Practices](https://prometheus.io/docs/practices/naming/)
- [OpenTelemetry Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/)
- [Google SRE Book: Monitoring](https://sre.google/sre-book/monitoring-distributed-systems/)

---

**Last Updated**: October 2025
**Maintainers**: Nebula Team
