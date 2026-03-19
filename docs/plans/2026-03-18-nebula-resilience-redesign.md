# nebula-resilience Redesign

**Date:** 2026-03-18
**Status:** Approved
**Breaking changes:** Yes — full API redesign

---

## Context

The current `nebula-resilience` crate has several structural problems identified during review:

- Const generics (`CircuitBreaker<5, 30_000>`) make runtime configuration from `nebula-config` impossible
- Two parallel retry APIs (`RetryStrategy<N>` vs `RetryLayer`) that diverge in features
- `HedgeLayer` silently drops the cancellation context (`_cancellation`)
- `CircuitBreakerLayer` records `Cancelled` as a circuit failure
- Hedge does not cancel losing futures on first success — resource leak
- No integration with `nebula-eventbus` (violates ADR-010)
- Five different entry points for the same patterns — poor DX
- No `Retry Budget` or `Load Shedding`
- `CacheFallback` has no TTL

Since breaking changes are acceptable, this document defines the full redesign.

---

## Goals

1. **Single, ergonomic functional API** as the primary entry point
2. **Plain struct configs** — validated at construction time, readable from `nebula-config`
3. **`CallError<E>`** — action errors stay typed, no forced mapping to `ResilienceError`
4. **`PolicySource<C>` trait** — extension point for adaptive policies, zero cost today
5. **Three injectable dependencies** — `Clock`, `MetricsSink`, `LoadSignal` — for full testability
6. **EventBus integration** via `MetricsSink` implementation in `nebula-engine` — no direct dep
7. **Fix all known bugs** — hedge cancellation, CB recording cancelled as failure

---

## Non-Goals

- OS-process / WASM isolation (Phase 3, ADR-008)
- Distributed circuit breaker state (future, RSL-N006)
- Per-key rate limiting (future iteration)
- Multi-tier cache (future iteration)

---

## Architecture

### Error Type

```rust
/// Returned by all resilience operations.
/// E is the action's own error type — never forced to map into ResilienceError.
pub enum CallError<E> {
    /// The operation itself returned an error (after all retries exhausted).
    Operation(E),
    /// Circuit breaker is open — request rejected immediately.
    CircuitOpen,
    /// Bulkhead at capacity — request rejected.
    BulkheadFull,
    /// Timeout elapsed before operation completed.
    Timeout(Duration),
    /// All retry attempts exhausted; contains the last operation error.
    RetriesExhausted { attempts: u32, last: E },
    /// Operation was cancelled via CancellationContext.
    Cancelled { reason: Option<String> },
}

impl<E> CallError<E> {
    pub fn is_retriable(&self) -> bool { ... }
    pub fn map_operation<F, E2>(self, f: F) -> CallError<E2> { ... }
}
```

`ResilienceError` is **removed**. Pattern-internal errors (invalid config) return `ConfigError` from `new()`.

---

### Config structs (plain, no const generics)

```rust
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,        // min 1
    pub reset_timeout: Duration,       // min 1ms
    pub half_open_max_ops: u32,        // default 1
    pub min_operations: u32,           // before tripping, default 5
    pub failure_rate_threshold: f64,   // 0.0..=1.0, default 0.5
    pub sliding_window: Duration,      // default 60s
    pub count_timeouts_as_failures: bool, // default true
}

pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff: BackoffConfig,
    pub jitter: JitterConfig,
}

impl RetryConfig {
    pub fn new(max_attempts: u32) -> Self { ... }
    /// Only retry when this predicate returns true.
    pub fn retry_if<E, F>(self, f: F) -> RetryConfigWithPredicate<E, F>
    where F: Fn(&E) -> bool + Send + Sync + 'static { ... }
}

pub enum BackoffConfig {
    Fixed(Duration),
    Linear { base: Duration, max: Duration },
    Exponential { base: Duration, multiplier: f64, max: Duration },
}

pub struct BulkheadConfig {
    pub max_concurrent: usize,
    pub queue_size: usize,       // 0 = no queue (shed immediately)
    pub acquire_timeout: Duration,
}

pub struct RateLimitConfig {
    pub requests_per_second: f64,
    pub burst: u32,
    pub algorithm: RateLimitAlgorithm,
}

pub enum RateLimitAlgorithm { TokenBucket, LeakyBucket, SlidingWindow, Gcra }
```

All `Config::new()` / struct literals are validated in `CircuitBreaker::new(config)?` — returns `ConfigError` on invalid values.

---

### PolicySource — adaptive extension point

```rust
/// A source that provides the current policy config.
/// Static configs implement this automatically via blanket impl.
pub trait PolicySource<C>: Send + Sync {
    fn current(&self) -> C;
}

// Blanket impl: any Clone config is a static PolicySource
impl<C: Clone + Send + Sync> PolicySource<C> for C {
    fn current(&self) -> C { self.clone() }
}
```

Patterns accept `impl PolicySource<Config>` — today this is transparent, tomorrow it enables adaptive.

**Future AdaptiveRetrySource (not implemented now):**
```rust
pub struct AdaptiveRetrySource {
    base: RetryConfig,
    signal: Arc<dyn LoadSignal>,
}
impl PolicySource<RetryConfig> for AdaptiveRetrySource {
    fn current(&self) -> RetryConfig {
        let load = self.signal.load_factor();
        RetryConfig::new(if load > 0.8 { 1 } else { self.base.max_attempts })
    }
}
```

**LoadSignal trait (defined now, no production impl yet):**
```rust
pub trait LoadSignal: Send + Sync {
    fn load_factor(&self) -> f64;    // 0.0..=1.0
    fn error_rate(&self) -> f64;
    fn p99_latency(&self) -> Duration;
}

pub struct ConstantLoad { pub factor: f64 }  // for tests
```

---

### Injectable dependencies

Every pattern accepts optional overrides; defaults are production-grade:

```rust
CircuitBreaker::new(config)?
    .with_clock(mock_clock)        // default: SystemClock
    .with_sink(recording_sink)     // default: NoopSink
```

**MetricsSink:**
```rust
pub trait MetricsSink: Send + Sync {
    fn record(&self, event: ResilienceEvent);
}

pub enum ResilienceEvent {
    CircuitStateChanged { from: CircuitState, to: CircuitState },
    RetryAttempt { attempt: u32, will_retry: bool },
    BulkheadRejected,
    TimeoutElapsed { duration: Duration },
    HedgeFired { hedge_number: u32 },
}

pub struct NoopSink;                  // default
pub struct RecordingSink { ... }      // for tests: stores events in Vec
```

**EventBus integration (in nebula-engine, not here):**
```rust
// nebula-engine wires this up — no direct dep from resilience → eventbus
pub struct EventBusSink { bus: Arc<EventBus> }
impl MetricsSink for EventBusSink {
    fn record(&self, event: ResilienceEvent) {
        self.bus.emit(event.into());
    }
}
```

---

### Functional API (primary, via nebula-sdk::resilience)

```rust
// Minimal
let result = resilience::retry(3, || async {
    db.fetch(id).await
}).await?;

// Full control
let result = resilience::retry_with(
    RetryConfig::new(3)
        .backoff(BackoffConfig::Exponential { base: ms(100), multiplier: 2.0, max: secs(30) })
        .retry_if(|e: &DbError| matches!(e, DbError::Connection | DbError::Timeout)),
    || async { db.fetch(id).await },
).await?;

// Circuit breaker (shared state — Arc)
let breaker = Arc::new(CircuitBreaker::new(config)?);
let result = breaker.call(|| async { api.post(req).await }).await?;

// Timeout
let result = resilience::with_timeout(Duration::from_secs(5), || async {
    slow_service.get().await
}).await?;
```

---

### ResiliencePipeline (composition, for engine and complex actions)

```rust
let pipeline = ResiliencePipeline::builder()
    .timeout(Duration::from_secs(10))
    .retry(RetryConfig::new(3).backoff(BackoffConfig::exponential_default()))
    .circuit_breaker(breaker.clone())
    .bulkhead(BulkheadConfig { max_concurrent: 20, queue_size: 0, acquire_timeout: ms(50) })
    .rate_limit(RateLimitConfig { requests_per_second: 100.0, burst: 10, algorithm: Gcra })
    .with_sink(sink.clone())
    .build();

// Layer order is validated at build() — warns via tracing::warn! (not debug!)
// Recommended: timeout → retry → circuit_breaker → bulkhead → rate_limit → hedge → fallback

let result = pipeline.call(|| async { ... }).await?;
```

Pipeline internals use **typed composition** (not `Arc<dyn ResilienceLayer>` chain) for the common cases — dynamic dispatch only when custom layers are added via `.with_layer()`.

---

### Bug fixes included in this redesign

| Bug | Fix |
|-----|-----|
| `HedgeLayer` ignores cancellation | Pass `cancellation` to all hedge futures; cancel losers via `CancellationToken` on first success |
| CB records `Cancelled` as failure | `record_failure()` only called when `!error.is_cancellation()` |
| Hedge leaks losing futures | `CancellationToken` sent to all in-flight hedges when first succeeds |
| Layer order warning = `debug!` | Changed to `tracing::warn!` |

---

### Gate (unchanged)

`Gate` is well-designed and stays as-is. Re-exported from `nebula-sdk` explicitly.

---

## Testability checklist

- [ ] `MockClock::advance()` controls time in CB reset, retry backoff, timeout
- [ ] `RecordingSink` captures all `ResilienceEvent`s — assertable in tests
- [ ] `ConstantLoad` provides deterministic `LoadSignal` for adaptive tests
- [ ] All patterns constructible without tokio runtime (sync construction)
- [ ] No global state — every instance is independent

---

## Migration from current API

| Old | New |
|-----|-----|
| `CircuitBreaker::<5, 30_000>::new()` | `CircuitBreaker::new(CircuitBreakerConfig { failure_threshold: 5, reset_timeout: secs(30), .. Default::default() })?` |
| `exponential_retry::<3>()` | `retry_with(RetryConfig::new(3).backoff(BackoffConfig::exponential_default()), \|\| ...)` |
| `ResilienceError::*` | `CallError<E>` |
| `LayerBuilder::new().with_retry().build()` | `ResiliencePipeline::builder().retry(...).build()` |
| `ObservabilityHook` | `MetricsSink` |

---

## Open questions (resolved)

- ~~Const generics: keep or remove?~~ **Removed.**
- ~~Two retry APIs: unify how?~~ **Single `RetryConfig` + functional API.**
- ~~Error type: generic or mapped?~~ **`CallError<E>` — generic.**
- ~~EventBus: direct dep or bridge?~~ **`MetricsSink` impl in engine — no direct dep.**
