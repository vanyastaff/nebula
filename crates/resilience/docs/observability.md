# nebula-resilience — Observability

`nebula-resilience` has two independent observability layers:

1. **`MetricsSink` / `ResilienceEvent`** — primary sink for structured events emitted by
   patterns (circuit breaker transitions, retry attempts, bulkhead rejections, etc.).
   This is the preferred integration point for production metrics pipelines.

2. **`ObservabilityHooks` / `PatternEvent`** — legacy hook system providing `Event<C>`
   typed events and built-in `LoggingHook` / `MetricsHook` for `tracing`-based
   observability.

---

## Table of Contents

- [MetricsSink](#metricssink)
- [ResilienceEvent](#resilienceevent)
- [RecordingSink (testing)](#recordingsink-testing)
- [Injecting a Sink](#injecting-a-sink)
- [ObservabilityHooks (legacy)](#observabilityhooks-legacy)
- [PatternEvent](#patternevent)
- [Typed Events](#typed-events)
- [Tracing Spans](#tracing-spans)
- [MetricsCollector](#metricscollector)
- [Wire-Up Example](#wire-up-example)

---

## MetricsSink

The primary observability trait. Implemented by `NoopSink` (default) and `RecordingSink`
(testing). In production, implement `MetricsSink` to forward events to your metrics
backend (Prometheus, EventBus, etc.).

```rust
pub trait MetricsSink: Send + Sync {
    fn record(&self, event: ResilienceEvent);
}

/// Default — discards all events. Zero cost.
pub struct NoopSink;
```

All implementations are called synchronously. Keep them fast; offload heavy I/O to a
background channel.

---

## ResilienceEvent

Typed events emitted by patterns:

```rust
#[derive(Debug, Clone)]
pub enum ResilienceEvent {
    /// Circuit breaker transitioned between states.
    CircuitStateChanged {
        from: CircuitState,
        to: CircuitState,
    },
    /// A retry attempt was made (1-based).
    RetryAttempt {
        attempt: u32,
        will_retry: bool,
    },
    /// Bulkhead rejected a request (at capacity).
    BulkheadRejected,
    /// A timeout elapsed.
    TimeoutElapsed { duration: Duration },
    /// A hedge request was fired (1-based).
    HedgeFired { hedge_number: u32 },
    /// Rate limit was exceeded.
    RateLimitExceeded,
    /// Load shed — request rejected due to overload.
    LoadShed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}
```

---

## RecordingSink (testing)

For test assertions — records all events in memory:

```rust
let sink = Arc::new(RecordingSink::new());

// Inject into CircuitBreaker
let cb = CircuitBreaker::with_sink(config, sink.clone())?;

// ... run operations ...

// Assert
assert_eq!(sink.count("retry_attempt"), 3);
assert!(sink.has_state_change(CircuitState::Open));

// Inspect all events
for event in sink.events() {
    println!("{event:?}");
}
```

Available `count()` kind strings:

| Kind string | Event |
|-------------|-------|
| `"circuit_state_changed"` | `CircuitStateChanged` |
| `"retry_attempt"` | `RetryAttempt` |
| `"bulkhead_rejected"` | `BulkheadRejected` |
| `"timeout_elapsed"` | `TimeoutElapsed` |
| `"hedge_fired"` | `HedgeFired` |
| `"rate_limit_exceeded"` | `RateLimitExceeded` |
| `"load_shed"` | `LoadShed` |

---

## Injecting a Sink

Currently `CircuitBreaker` accepts a sink via `with_sink()`. Other patterns use the
sink at construction time where applicable.

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::sink::{MetricsSink, ResilienceEvent};
use std::sync::Arc;

struct PrometheusSink { /* ... */ }

impl MetricsSink for PrometheusSink {
    fn record(&self, event: ResilienceEvent) {
        match event {
            ResilienceEvent::CircuitStateChanged { to, .. } => {
                // increment circuit_state counter
            }
            ResilienceEvent::RetryAttempt { attempt, .. } => {
                // increment retry_total counter
            }
            _ => {}
        }
    }
}

let sink = Arc::new(PrometheusSink { /* ... */ });
let cb = CircuitBreaker::with_sink(CircuitBreakerConfig::default(), sink)?;
```

---

## ObservabilityHooks (legacy)

The older hook system. Useful for `tracing`-integrated logging. Separate from `MetricsSink`.

```rust
pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &PatternEvent);
}

pub struct ObservabilityHooks { … }

impl ObservabilityHooks {
    pub fn new() -> Self;
    pub fn add(&mut self, hook: Arc<dyn ObservabilityHook>);
    pub fn emit(&self, event: PatternEvent);
    pub fn hook_count(&self) -> usize;
}
```

Built-in hooks:

**`LoggingHook`** — logs every event via `tracing` at a configurable level:

```rust
let hook = LoggingHook::new(LogLevel::Info);
hooks.add(Arc::new(hook));
```

**`MetricsHook`** — forwards events to a `MetricsCollector`:

```rust
let collector = Arc::new(MetricsCollector::new(true));
let hook = MetricsHook::new(collector.clone());
hooks.add(Arc::new(hook));
```

---

## PatternEvent

Untyped event for legacy hooks:

```rust
pub struct PatternEvent {
    pub pattern: String,    // e.g. "retry", "circuit_breaker"
    pub operation: String,
    pub duration: Option<Duration>,
    pub success: bool,
    pub error: Option<String>,
    pub metadata: HashMap<String, String>,
}
```

---

## Typed Events

`Event<C: EventCategory>` provides compile-time category tagging:

```rust
use nebula_resilience::hooks::{Event, RetryEventCategory};

let event = Event::<RetryEventCategory>::new("payment_api")
    .with_duration(Duration::from_millis(250))
    .with_error("connection refused")
    .with_context("attempt", "2");

// event.category() → "retry"
// event.is_error() → true
```

### Event category markers (sealed)

| Type | `name()` | Default log level |
|------|---------|------------------|
| `RetryEventCategory` | `"retry"` | `Info` |
| `CircuitBreakerEventCategory` | `"circuit_breaker"` | `Warn` |
| `BulkheadEventCategory` | `"bulkhead"` | `Info` |
| `TimeoutEventCategory` | `"timeout"` | `Warn` |
| `RateLimiterEventCategory` | `"rate_limiter"` | `Info` |

### `Event<C>` builder API

```rust
impl<C: EventCategory> Event<C> {
    pub fn new(operation: impl Into<String>) -> Self;
    pub fn with_duration(self, duration: Duration) -> Self;
    pub fn with_error(self, error: impl Into<String>) -> Self;
    pub fn with_context(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn category(&self) -> &'static str;
    pub fn is_error(&self) -> bool;
    pub fn is_sampled(&self) -> bool;
}
```

---

## Tracing Spans

### `SpanGuard`

RAII tracing span that records success or error on drop:

```rust
use nebula_resilience::spans::{create_span, record_success, record_error};

let span = create_span("my_operation", "retry");
let result = do_work().await;
match &result {
    Ok(_)  => record_success(&span),
    Err(e) => record_error(&span, e),
}
```

### `PatternSpanGuard<C: PatternCategory>`

Typed span guard — pattern category is a compile-time constant:

```rust
use nebula_resilience::spans::{PatternSpanGuard, PatternCategory};

let span = PatternSpanGuard::<RetryPattern>::new("payment_api");
// span.category_name() → "retry"

match do_work().await {
    Ok(_)    => span.success(),
    Err(ref e) => span.failure(e),
}
```

---

## MetricsCollector

In-process accumulator for numeric metrics. Security-hardened: names over 256 chars
are silently dropped; more than 10 000 unique keys cause subsequent new keys to be dropped.

```rust
pub struct MetricsCollector { … }
pub type Metrics = Arc<MetricsCollector>;

impl MetricsCollector {
    pub fn new(enabled: bool) -> Self;

    pub fn record(&self, name: impl Into<String>, value: f64);
    pub fn increment(&self, name: impl Into<String>);
    pub fn record_duration(&self, name: impl Into<String>, duration: Duration);
    pub fn start_timer(&self, name: impl Into<String>) -> MetricTimer;

    pub fn snapshot(&self, name: &str) -> Option<MetricSnapshot>;
    pub fn all_snapshots(&self) -> HashMap<String, MetricSnapshot>;
    pub fn reset(&self, name: &str);
    pub fn clear(&self);
}

pub struct MetricSnapshot {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}
```

---

## Wire-Up Example

Complete observability setup for a production service:

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::sink::{MetricsSink, NoopSink, RecordingSink, ResilienceEvent};
use nebula_resilience::hooks::{ObservabilityHooks, LoggingHook, MetricsHook, LogLevel};
use nebula_resilience::metrics::MetricsCollector;
use std::sync::Arc;
use std::time::Duration;

// 1. MetricsSink for structured events (primary)
let sink = Arc::new(RecordingSink::new()); // or your custom impl

let cb = CircuitBreaker::with_sink(
    CircuitBreakerConfig {
        failure_threshold: 5,
        reset_timeout: Duration::from_secs(30),
        ..Default::default()
    },
    sink.clone(),
)?;

// 2. Legacy hooks for tracing-based logging (optional)
let collector = Arc::new(MetricsCollector::new(true));
let mut hooks = ObservabilityHooks::new();
hooks.add(Arc::new(LoggingHook::new(LogLevel::Debug)));
hooks.add(Arc::new(MetricsHook::new(collector.clone())));

// 3. After running operations, inspect
let events = sink.events();
println!("Circuit state changes: {}", sink.count("circuit_state_changed"));

if let Some(snap) = collector.snapshot("retry.attempt") {
    println!("Total retry attempts: {}", snap.count);
}
```
