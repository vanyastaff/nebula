# nebula-resilience — Observability

`nebula-resilience` provides observability via **`MetricsSink` / `ResilienceEvent`** — a
structured event sink for pattern events (circuit breaker transitions, retry attempts,
bulkhead rejections, etc.). This is the integration point for production metrics pipelines.

---

## Table of Contents

- [MetricsSink](#metricssink)
- [ResilienceEvent](#resilienceevent)
- [RecordingSink (testing)](#recordingsink-testing)
- [Injecting a Sink](#injecting-a-sink)
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
let sink = RecordingSink::new();

// Inject into CircuitBreaker
let cb = CircuitBreaker::new(config)?.with_sink(sink.clone());

// ... run operations ...

// Assert
assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
assert!(sink.has_state_change(CircuitState::Open));

// Inspect all events
for event in sink.events() {
    println!("{event:?}");
}
```

`count()` takes a `ResilienceEventKind` enum variant:

| `ResilienceEventKind` variant | Event |
|-------------------------------|-------|
| `ResilienceEventKind::CircuitStateChanged` | `CircuitStateChanged` |
| `ResilienceEventKind::RetryAttempt` | `RetryAttempt` |
| `ResilienceEventKind::BulkheadRejected` | `BulkheadRejected` |
| `ResilienceEventKind::TimeoutElapsed` | `TimeoutElapsed` |
| `ResilienceEventKind::HedgeFired` | `HedgeFired` |
| `ResilienceEventKind::RateLimitExceeded` | `RateLimitExceeded` |
| `ResilienceEventKind::LoadShed` | `LoadShed` |

Use `event.kind()` to get the `ResilienceEventKind` from a `ResilienceEvent`.

---

## Injecting a Sink

Sink injection is available on the pattern types that own their own observability
(`CircuitBreaker`, `Bulkhead`, `RetryConfig`, `HedgeExecutor`, `AdaptiveHedgeExecutor`,
`TimeoutExecutor`) and on `ResiliencePipeline::with_sink()` for pipeline-level timeout /
rate-limit / load-shed events.

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::sink::{MetricsSink, ResilienceEvent};

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

let cb = CircuitBreaker::new(CircuitBreakerConfig::default())?
    .with_sink(PrometheusSink { /* ... */ });
```

---

## Wire-Up Example

Complete observability setup for a production service:

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::sink::{MetricsSink, RecordingSink, ResilienceEventKind};
use std::time::Duration;

// MetricsSink for structured events
let sink = RecordingSink::new(); // or your custom impl

let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 5,
        reset_timeout: Duration::from_secs(30),
        ..Default::default()
    })?
    .with_sink(sink.clone());

// After running operations, inspect
let events = sink.events();
println!("Circuit state changes: {}", sink.count(ResilienceEventKind::CircuitStateChanged));
```
