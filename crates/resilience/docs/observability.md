# nebula-resilience — Observability

`nebula-resilience` exposes three observability extension points:

- **`MetricsCollector`** — in-process metrics accumulator with security-hardened key
  validation and per-key `MetricSnapshot` exports.
- **`ObservabilityHooks`** — ordered list of callbacks that receive a `PatternEvent`
  on every significant lifecycle transition.
- **Tracing spans** — RAII `SpanGuard` and `PatternSpanGuard<C>` helpers that emit
  structured `tracing` events at start, success, and failure.

---

## Table of Contents

- [MetricsCollector](#metricscollector)
- [MetricSnapshot](#metricsnapshot)
- [MetricTimer](#metrictimer)
- [ObservabilityHooks](#observabilityhooks)
- [PatternEvent](#patternevent)
- [ObservabilityHook Trait](#observabilityhook-trait)
- [Built-in Hooks](#built-in-hooks)
- [Typed Events](#typed-events)
- [Typed Metrics](#typed-metrics)
- [Tracing Spans](#tracing-spans)
- [Wire-Up Example](#wire-up-example)

---

## MetricsCollector

In-process metrics store. Keyed by arbitrary `String` names. Security-hardened:
names over 256 characters are silently dropped; more than 10 000 unique keys cause
subsequent new keys to be dropped.

```rust
pub struct MetricsCollector { /* ... */ }

impl MetricsCollector {
    /// Create a new collector. Pass `enabled: false` to make all methods no-ops.
    pub fn new(enabled: bool) -> Self;

    /// Record a raw f64 sample. NaN and ±Infinity are silently dropped.
    pub fn record(&self, name: impl Into<String>, value: f64);

    /// Increment a counter by 1.0.
    pub fn increment(&self, name: impl Into<String>);

    /// Record a `Duration` as milliseconds.
    pub fn record_duration(&self, name: impl Into<String>, duration: Duration);

    /// Start a wall-clock timer. Records when `MetricTimer` is dropped.
    pub fn start_timer(&self, name: impl Into<String>) -> MetricTimer;

    /// Read a point-in-time snapshot for one key. Returns `None` if key is unknown.
    pub fn snapshot(&self, name: &str) -> Option<MetricSnapshot>;

    /// Read snapshots for all registered keys.
    pub fn all_snapshots(&self) -> HashMap<String, MetricSnapshot>;

    /// Reset recorded values for one key (keeps the key slot).
    pub fn reset(&self, name: &str);

    /// Drop all keys and their values.
    pub fn clear(&self);
}
```

### Suggested naming convention

Use `<pattern>.<event>` to keep metrics grouped:

| Name | Meaning |
|------|---------|
| `circuit_breaker.open` | Circuit transitioned to Open |
| `circuit_breaker.half_open` | Circuit transitioned to HalfOpen |
| `circuit_breaker.closed` | Circuit transitioned to Closed |
| `retry.attempt` | Retry attempt count |
| `retry.exhausted` | Retry budget depleted |
| `bulkhead.rejected` | Request rejected due to capacity |
| `bulkhead.wait_ms` | Duration spent waiting for a permit |
| `timeout.fired` | Timeout limit exceeded |
| `rate_limiter.rejected` | Request rejected by rate limiter |

---

## MetricSnapshot

Read-only point-in-time view of accumulated values for one key:

```rust
pub struct MetricSnapshot {
    pub count: u64,   // total number of recorded samples
    pub sum: f64,     // sum of all samples
    pub min: f64,     // minimum recorded value
    pub max: f64,     // maximum recorded value
    pub mean: f64,    // arithmetic mean (sum / count)
}
```

---

## MetricTimer

RAII duration recorder. Records elapsed time (in milliseconds) to the underlying
`MetricsCollector` when dropped:

```rust
let timer = collector.start_timer("db.query_ms");
let result = db.query(sql).await;
drop(timer); // automatically records elapsed duration
```

---

## ObservabilityHooks

An ordered collection of `ObservabilityHook` implementations. Emit a `PatternEvent`
to call every registered hook in insertion order.

```rust
pub struct ObservabilityHooks {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityHooks {
    pub fn new() -> Self;
    pub fn add(&mut self, hook: Arc<dyn ObservabilityHook>);
    pub fn emit(&self, event: PatternEvent);
    pub fn hook_count(&self) -> usize;
}
```

---

## PatternEvent

Untyped event passed to legacy `ObservabilityHook` implementations:

```rust
pub struct PatternEvent {
    pub pattern: String,    // e.g. "retry", "circuit_breaker"
    pub operation: String,  // caller-provided operation name
    pub duration: Option<Duration>,
    pub success: bool,
    pub error: Option<String>,
    pub metadata: HashMap<String, String>,
}
```

---

## ObservabilityHook Trait

```rust
pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &PatternEvent);
}
```

All registered hooks are called synchronously inside `ObservabilityHooks::emit()`.
Keep hook implementations fast; defer slow I/O to a background channel if needed.

---

## Built-in Hooks

### `LoggingHook`

Logs every event via `tracing` at a configurable level. The per-category default log
level is used when no level is specified.

```rust
let hook = LoggingHook::new(LogLevel::Info);
hooks.add(Arc::new(hook));
```

`LogLevel` variants:

```rust
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
```

### `MetricsHook`

Forwards events to a `MetricsCollector`, incrementing per-pattern counters and
recording durations.

```rust
let collector = Arc::new(MetricsCollector::new(true));
let hook = MetricsHook::new(collector.clone());
hooks.add(Arc::new(hook));
```

After a run, read aggregated results:

```rust
if let Some(snap) = collector.snapshot("retry.attempt") {
    println!("total retry attempts: {}", snap.count);
    println!("mean delay ms: {:.1}", snap.mean);
}
```

---

## Typed Events

`Event<C: EventCategory>` provides compile-time category tagging:

```rust
use nebula_resilience::observability::{Event, RetryEventCategory};

let event = Event::<RetryEventCategory>::new("payment_api")
    .with_duration(Duration::from_millis(250))
    .with_context("attempt", "2")
    .with_context("max_attempts", "3");

// event.category() → "retry"
// event.log_level() → LogLevel::Info (default for retry)
```

### Event category markers

| Type | `name()` | Default log level |
|------|---------|------------------|
| `RetryEventCategory` | `"retry"` | `Info` |
| `CircuitBreakerEventCategory` | `"circuit_breaker"` | `Warn` |
| `BulkheadEventCategory` | `"bulkhead"` | `Info` |
| `TimeoutEventCategory` | `"timeout"` | `Warn` |
| `RateLimiterEventCategory` | `"rate_limiter"` | `Info` |

Categories are sealed: no external implementations are allowed. This ensures all
event classification is exhaustive within the crate.

### `Event<C>` builder API

```rust
impl<C: EventCategory> Event<C> {
    pub fn new(operation: impl Into<String>) -> Self;

    /// Attach elapsed duration.
    pub fn with_duration(self, duration: Duration) -> Self;

    /// Attach error description (marks event as error via is_error()).
    pub fn with_error(self, error: impl Into<String>) -> Self;

    /// Add arbitrary key-value context.
    pub fn with_context(self, key: impl Into<String>, value: impl Into<String>) -> Self;

    /// Returns typed category name.
    pub fn category(&self) -> &'static str { C::name() }

    /// Returns whether this should be sampled by the category's policy.
    pub fn is_sampled(&self) -> bool { C::is_sampled() }

    /// Returns true when an error was attached.
    pub fn is_error(&self) -> bool { self.error.is_some() }
}
```

---

## Typed Metrics

`Metric<DIMENSIONS>` carries a fixed set of label key-value pairs as a const-generic
array. Zero-cost at the dimension count level — no heap allocation for labels.

```rust
use nebula_resilience::observability::metrics;

// Convenience constructors from the `metrics` module:
let hist  = metrics::operation_histogram("latency_ms", "api", "get", 42.0);
let gauge = metrics::state_gauge("circuit_state", "payment_api", "open");
let count = metrics::error_counter("errors", "retry", "timeout");
```

Each constructor produces a `Metric<2>` with `("service", …)` / `("operation", …)` labels.

---

## Tracing Spans

### `SpanGuard`

Records a `tracing` event at construction and another at drop:

```rust
use nebula_resilience::observability::spans::{SpanGuard, create_span, record_success, record_error};

let span = create_span("my_operation", "retry");
// ... perform work ...
record_success(&span);  // emits a tracing event at INFO level
// or
record_error(&span, &error);  // emits a tracing event at WARN level
```

### `PatternSpanGuard<C: PatternCategory>`

Typed span guard. Pattern category is a compile-time constant:

```rust
use nebula_resilience::observability::spans::{PatternSpanGuard, RetryPattern};

let span = PatternSpanGuard::<RetryPattern>::new("payment_api");
// span.category_name() → "retry"

match do_work().await {
    Ok(_)    => span.success(),
    Err(ref e) => span.failure(e),
}
```

Pattern categories:

| Type | `name()` | Logs start? |
|------|---------|------------|
| `RetryPattern` | `"retry"` | yes |
| `CircuitBreakerPattern` | `"circuit_breaker"` | yes |
| `TimeoutPattern` | `"timeout"` | yes |
| `BulkheadPattern` | `"bulkhead"` | yes |

---

## Wire-Up Example

Complete observability setup for a production service:

```rust
use nebula_resilience::core::metrics::{MetricKind, MetricsCollector};
use nebula_resilience::observability::{ObservabilityHooks, LoggingHook, MetricsHook, LogLevel};
use std::sync::Arc;

// 1. Metrics collector
let collector = Arc::new(MetricsCollector::new(true));

// 2. Hooks
let mut hooks = ObservabilityHooks::new();
hooks.add(Arc::new(LoggingHook::new(LogLevel::Debug)));
hooks.add(Arc::new(MetricsHook::new(collector.clone())));

// 3. Emit events from pattern implementations
hooks.emit(PatternEvent {
    pattern: "circuit_breaker".to_string(),
    operation: "payment_api".to_string(),
    duration: Some(Duration::from_millis(15)),
    success: false,
    error: Some("connection refused".to_string()),
    metadata: Default::default(),
});

// 4. Export metrics at any time
let snapshots = collector.all_snapshots();
for (name, snap) in &snapshots {
    println!("{name}: count={}, mean={:.2}ms", snap.count, snap.mean);
}
```
