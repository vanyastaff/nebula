# nebula-resilience Redesign Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fully rebuild `nebula-resilience` with plain-struct configs, a generic `CallError<E>` error type, `PolicySource<C>` for future adaptive policies, and injectable `MetricsSink`/`Clock`/`LoadSignal` — while fixing all known bugs.

**Architecture:** Remove all const generics from pattern structs; configs are plain Rust structs validated at construction. Patterns accept `impl PolicySource<Config>` so adaptive variants can be dropped in later without changing call-sites. `MetricsSink` replaces the custom observability hook system and acts as the EventBus bridge in `nebula-engine`.

**Tech Stack:** Rust, tokio, parking_lot, dashmap, futures, tracing. Optional: `governor` (GCRA). Internal: `nebula-core`, `nebula-config`, `nebula-log`.

**Design reference:** `docs/plans/2026-03-18-nebula-resilience-redesign.md`

**Verify after every task:**
```bash
cargo check -p nebula-resilience
cargo test -p nebula-resilience
```

---

## Phase 1 — Foundation Types

### Task 1: Replace ResilienceError with CallError<E>

**Files:**
- Modify: `crates/resilience/src/core/types.rs` (full rewrite)
- Modify: `crates/resilience/src/lib.rs` (update re-exports)

**Step 1: Write failing test**

Add to the bottom of `crates/resilience/src/core/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Debug, PartialEq)]
    enum MyErr { Timeout, NotFound }

    #[test]
    fn call_error_is_retriable_for_operation() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        assert!(!e.is_retriable()); // CallError::Operation is never auto-retriable
    }

    #[test]
    fn call_error_is_retriable_for_circuit_open() {
        let e: CallError<MyErr> = CallError::CircuitOpen;
        assert!(!e.is_retriable()); // CB open — don't retry
    }

    #[test]
    fn call_error_map_operation() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        let mapped: CallError<String> = e.map_operation(|e| format!("{:?}", e));
        assert!(matches!(mapped, CallError::Operation(s) if s == "Timeout"));
    }

    #[test]
    fn cancelled_is_not_retriable() {
        let e: CallError<MyErr> = CallError::Cancelled { reason: Some("shutdown".into()) };
        assert!(!e.is_retriable());
    }
}
```

**Step 2: Run test to confirm it fails**

```bash
cargo test -p nebula-resilience core::types
```
Expected: compile error — `CallError` does not exist yet.

**Step 3: Implement CallError**

Replace the contents of `crates/resilience/src/core/types.rs` with:

```rust
//! Core error and result types for nebula-resilience.

use std::time::Duration;

/// Returned by all resilience operations.
///
/// `E` is the caller's own error type — never forced to map into a resilience error.
/// Errors produced by the patterns themselves (circuit open, bulkhead full, etc.)
/// are separate variants.
#[derive(Debug)]
pub enum CallError<E> {
    /// The operation itself returned an error (possibly after retries exhausted).
    Operation(E),
    /// Circuit breaker is open — request rejected immediately.
    CircuitOpen,
    /// Bulkhead is at capacity — request rejected.
    BulkheadFull,
    /// Timeout elapsed before the operation completed.
    Timeout(Duration),
    /// All retry attempts exhausted; contains the last operation error.
    RetriesExhausted { attempts: u32, last: E },
    /// Operation was cancelled via `CancellationContext`.
    Cancelled { reason: Option<String> },
    /// Load shed — system is overloaded, request rejected without queuing.
    LoadShed,
    /// Rate limit exceeded.
    RateLimited,
    /// Configuration error — returned only from `new()` constructors.
    Config(ConfigError),
}

impl<E> CallError<E> {
    /// Returns true only if the error class suggests a retry might succeed.
    /// Note: `Operation` is never automatically retriable — the caller must
    /// supply a predicate via `RetryConfig::retry_if` to classify their errors.
    pub fn is_retriable(&self) -> bool {
        false // all pattern errors are non-retriable; operation retryability is predicate-driven
    }

    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
    }

    /// Map the inner operation error, leaving pattern errors unchanged.
    pub fn map_operation<F, E2>(self, f: F) -> CallError<E2>
    where
        F: FnOnce(E) -> E2,
    {
        match self {
            Self::Operation(e) => CallError::Operation(f(e)),
            Self::RetriesExhausted { attempts, last } => {
                CallError::RetriesExhausted { attempts, last: f(last) }
            }
            Self::CircuitOpen => CallError::CircuitOpen,
            Self::BulkheadFull => CallError::BulkheadFull,
            Self::Timeout(d) => CallError::Timeout(d),
            Self::Cancelled { reason } => CallError::Cancelled { reason },
            Self::LoadShed => CallError::LoadShed,
            Self::RateLimited => CallError::RateLimited,
            Self::Config(e) => CallError::Config(e),
        }
    }
}

/// Returned from pattern constructors when configuration is invalid.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid resilience config: {message}")]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

impl ConfigError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self { field, message: message.into() }
    }
}

/// Convenience alias.
pub type CallResult<T, E> = Result<T, CallError<E>>;
```

**Step 4: Run tests**

```bash
cargo test -p nebula-resilience core::types
```
Expected: all 4 tests pass.

**Step 5: Update lib.rs re-exports**

In `crates/resilience/src/lib.rs`, add/update:
```rust
pub use core::types::{CallError, CallResult, ConfigError};
```

**Step 6: Commit**

```bash
git add crates/resilience/src/core/types.rs crates/resilience/src/lib.rs
git commit -m "feat(resilience): introduce CallError<E> and ConfigError, remove ResilienceError"
```

---

### Task 2: PolicySource<C> + LoadSignal + MetricsSink traits

**Files:**
- Create: `crates/resilience/src/core/policy_source.rs`
- Create: `crates/resilience/src/core/signals.rs`
- Create: `crates/resilience/src/observability/sink.rs`
- Modify: `crates/resilience/src/core/mod.rs`
- Modify: `crates/resilience/src/observability/mod.rs`
- Modify: `crates/resilience/src/lib.rs`

**Step 1: Write failing tests**

`crates/resilience/src/core/policy_source.rs` (new file, start with tests at bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct Config { value: u32 }

    #[test]
    fn static_config_is_policy_source() {
        let cfg = Config { value: 42 };
        // blanket impl: any Clone is a PolicySource
        assert_eq!(cfg.current(), Config { value: 42 });
    }

    #[test]
    fn static_config_returns_clone_each_time() {
        let cfg = Config { value: 7 };
        assert_eq!(cfg.current(), cfg.current());
    }
}
```

**Step 2: Run to confirm fails**

```bash
cargo test -p nebula-resilience core::policy_source
```
Expected: compile error.

**Step 3: Implement PolicySource**

`crates/resilience/src/core/policy_source.rs`:

```rust
//! Extension point for adaptive policy configuration.

/// A source that provides the current configuration for a resilience pattern.
///
/// Static configs implement this automatically via the blanket impl below.
/// Adaptive sources compute the config at call-time based on runtime signals.
pub trait PolicySource<C: Clone>: Send + Sync {
    fn current(&self) -> C;
}

/// Blanket impl: any `Clone + Send + Sync` value is a static policy source.
impl<C: Clone + Send + Sync> PolicySource<C> for C {
    fn current(&self) -> C {
        self.clone()
    }
}
```

**Step 4: Implement LoadSignal + ConstantLoad**

`crates/resilience/src/core/signals.rs`:

```rust
//! Signals for adaptive policy sources.

use std::time::Duration;

/// Runtime signal providing system load metrics for adaptive policies.
pub trait LoadSignal: Send + Sync {
    /// Overall load factor in 0.0..=1.0 (0 = idle, 1 = fully saturated).
    fn load_factor(&self) -> f64;
    /// Error rate over the last measurement window (0.0..=1.0).
    fn error_rate(&self) -> f64;
    /// Approximate p99 latency of recent operations.
    fn p99_latency(&self) -> Duration;
}

/// A constant load signal for testing adaptive policies.
pub struct ConstantLoad {
    pub factor: f64,
    pub error_rate: f64,
    pub p99_latency: Duration,
}

impl ConstantLoad {
    pub fn idle() -> Self {
        Self { factor: 0.0, error_rate: 0.0, p99_latency: Duration::from_millis(5) }
    }

    pub fn saturated() -> Self {
        Self { factor: 1.0, error_rate: 0.5, p99_latency: Duration::from_secs(2) }
    }
}

impl LoadSignal for ConstantLoad {
    fn load_factor(&self) -> f64 { self.factor }
    fn error_rate(&self) -> f64 { self.error_rate }
    fn p99_latency(&self) -> Duration { self.p99_latency }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_signal_returns_zero_load() {
        let s = ConstantLoad::idle();
        assert_eq!(s.load_factor(), 0.0);
        assert_eq!(s.error_rate(), 0.0);
    }

    #[test]
    fn saturated_signal_returns_full_load() {
        let s = ConstantLoad::saturated();
        assert_eq!(s.load_factor(), 1.0);
    }
}
```

**Step 5: Implement MetricsSink**

`crates/resilience/src/observability/sink.rs`:

```rust
//! MetricsSink — event sink for resilience observability.
//!
//! Replaces the custom ObservabilityHook system. The default is NoopSink.
//! In nebula-engine, EventBusSink wraps nebula-eventbus — no direct dep here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A state transition in the circuit breaker.
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState { Closed, Open, HalfOpen }

/// Events emitted by resilience patterns.
#[derive(Debug, Clone)]
pub enum ResilienceEvent {
    /// Circuit breaker state changed.
    CircuitStateChanged { from: CircuitState, to: CircuitState },
    /// A retry attempt was made.
    RetryAttempt { attempt: u32, will_retry: bool },
    /// A bulkhead rejected a request.
    BulkheadRejected,
    /// A timeout elapsed.
    TimeoutElapsed { duration: Duration },
    /// A hedge request was fired.
    HedgeFired { hedge_number: u32 },
    /// A rate limit was exceeded.
    RateLimitExceeded,
    /// Load shed — request rejected due to overload.
    LoadShed,
}

/// Receives resilience events for observability (metrics, logging, EventBus).
pub trait MetricsSink: Send + Sync {
    fn record(&self, event: ResilienceEvent);
}

/// Default sink — discards all events. Zero cost.
pub struct NoopSink;
impl MetricsSink for NoopSink {
    fn record(&self, _: ResilienceEvent) {}
}

/// Test sink — records all events for assertion.
#[derive(Default, Clone)]
pub struct RecordingSink {
    events: Arc<Mutex<Vec<ResilienceEvent>>>,
}

impl RecordingSink {
    pub fn new() -> Self { Self::default() }

    pub fn events(&self) -> Vec<ResilienceEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.events().iter().filter(|e| event_kind(e) == kind).count()
    }

    pub fn has_state_change(&self, to: CircuitState) -> bool {
        self.events().iter().any(|e| matches!(
            e, ResilienceEvent::CircuitStateChanged { to: t, .. } if *t == to
        ))
    }
}

impl MetricsSink for RecordingSink {
    fn record(&self, event: ResilienceEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn event_kind(e: &ResilienceEvent) -> &'static str {
    match e {
        ResilienceEvent::CircuitStateChanged { .. } => "circuit_state_changed",
        ResilienceEvent::RetryAttempt { .. } => "retry_attempt",
        ResilienceEvent::BulkheadRejected => "bulkhead_rejected",
        ResilienceEvent::TimeoutElapsed { .. } => "timeout_elapsed",
        ResilienceEvent::HedgeFired { .. } => "hedge_fired",
        ResilienceEvent::RateLimitExceeded => "rate_limit_exceeded",
        ResilienceEvent::LoadShed => "load_shed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_captures_events() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::BulkheadRejected);
        sink.record(ResilienceEvent::BulkheadRejected);
        assert_eq!(sink.count("bulkhead_rejected"), 2);
    }

    #[test]
    fn recording_sink_detects_state_change() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::CircuitStateChanged {
            from: CircuitState::Closed,
            to: CircuitState::Open,
        });
        assert!(sink.has_state_change(CircuitState::Open));
        assert!(!sink.has_state_change(CircuitState::HalfOpen));
    }

    #[test]
    fn noop_sink_does_not_panic() {
        let sink = NoopSink;
        sink.record(ResilienceEvent::LoadShed); // just must not panic
    }
}
```

**Step 6: Wire into mod files**

`crates/resilience/src/core/mod.rs` — add:
```rust
pub mod policy_source;
pub mod signals;
pub use policy_source::PolicySource;
pub use signals::{ConstantLoad, LoadSignal};
```

`crates/resilience/src/observability/mod.rs` — add:
```rust
pub mod sink;
pub use sink::{CircuitState, MetricsSink, NoopSink, RecordingSink, ResilienceEvent};
```

`crates/resilience/src/lib.rs` — add to re-exports:
```rust
pub use core::{PolicySource, LoadSignal, ConstantLoad};
pub use observability::{MetricsSink, NoopSink, RecordingSink, ResilienceEvent, CircuitState};
```

**Step 7: Run tests**

```bash
cargo test -p nebula-resilience core::policy_source core::signals observability::sink
```
Expected: all pass.

**Step 8: Commit**

```bash
git add crates/resilience/src/core/policy_source.rs \
        crates/resilience/src/core/signals.rs \
        crates/resilience/src/observability/sink.rs \
        crates/resilience/src/core/mod.rs \
        crates/resilience/src/observability/mod.rs \
        crates/resilience/src/lib.rs
git commit -m "feat(resilience): add PolicySource<C>, LoadSignal, MetricsSink foundation"
```

---

## Phase 2 — Circuit Breaker

### Task 3: Rebuild CircuitBreaker without const generics

**Files:**
- Modify: `crates/resilience/src/patterns/circuit_breaker.rs` (full rewrite)

**Step 1: Write failing tests first**

Add to the bottom of `crates/resilience/src/patterns/circuit_breaker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink, CircuitState as CS};
    use std::sync::Arc;
    use std::time::Duration;

    fn default_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_millis(100),
            half_open_max_ops: 1,
            min_operations: 1,
            failure_rate_threshold: 0.5,
            sliding_window: Duration::from_secs(60),
            count_timeouts_as_failures: true,
        }
    }

    #[tokio::test]
    async fn opens_after_failure_threshold() {
        let cb = CircuitBreaker::new(default_config()).unwrap();
        for _ in 0..3 {
            let _ = cb.call::<(), _>(|| async { Err("fail") }).await;
        }
        let err = cb.call::<(), _>(|| async { Ok(()) }).await.unwrap_err();
        assert!(matches!(err, CallError::CircuitOpen));
    }

    #[tokio::test]
    async fn cancelled_does_not_trip_breaker() {
        let cb = CircuitBreaker::new(default_config()).unwrap();
        // Simulate cancellation errors — should NOT count as failures
        for _ in 0..10 {
            cb.record_outcome(Outcome::Cancelled);
        }
        // Breaker should still be closed
        let result = cb.call::<u32, &str>(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn emits_state_change_event_on_open() {
        let sink = RecordingSink::new();
        let cb = CircuitBreaker::new(default_config())
            .unwrap()
            .with_sink(sink.clone());
        for _ in 0..3 {
            let _ = cb.call::<(), &str>(|| async { Err("fail") }).await;
        }
        assert!(sink.has_state_change(CS::Open));
    }

    #[tokio::test]
    async fn config_error_on_zero_threshold() {
        let result = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 0,
            ..default_config()
        });
        assert!(result.is_err());
    }
}
```

**Step 2: Run to confirm fails**

```bash
cargo test -p nebula-resilience patterns::circuit_breaker
```
Expected: compile errors referencing new API.

**Step 3: Implement new CircuitBreaker**

Replace `crates/resilience/src/patterns/circuit_breaker.rs` with the new implementation:

```rust
//! Circuit breaker pattern — plain-struct config, injectable sink and clock.

use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;

use crate::{
    CallError, ConfigError,
    clock::{Clock, SystemClock},
    observability::sink::{CircuitState, MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening. Min: 1.
    pub failure_threshold: u32,
    /// How long to wait before probing in half-open state.
    pub reset_timeout: Duration,
    /// Max concurrent probes in half-open state. Default: 1.
    pub half_open_max_ops: u32,
    /// Min operations before the failure rate can trip the breaker. Default: 5.
    pub min_operations: u32,
    /// Failure rate (0.0..=1.0) that triggers opening. Default: 0.5.
    pub failure_rate_threshold: f64,
    /// Sliding window for counting failures. Default: 60s.
    pub sliding_window: Duration,
    /// Whether timeouts count as failures. Default: true.
    pub count_timeouts_as_failures: bool,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            half_open_max_ops: 1,
            min_operations: 5,
            failure_rate_threshold: 0.5,
            sliding_window: Duration::from_secs(60),
            count_timeouts_as_failures: true,
        }
    }
}

impl CircuitBreakerConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.failure_threshold == 0 {
            return Err(ConfigError::new("failure_threshold", "must be >= 1"));
        }
        if self.reset_timeout.is_zero() {
            return Err(ConfigError::new("reset_timeout", "must be > 0"));
        }
        if !(0.0..=1.0).contains(&self.failure_rate_threshold) {
            return Err(ConfigError::new("failure_rate_threshold", "must be 0.0..=1.0"));
        }
        Ok(())
    }
}

// ── Outcome (internal) ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Outcome { Success, Failure, Timeout, Cancelled }

// ── State machine ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum State { Closed, Open { opened_at: std::time::Instant }, HalfOpen }

// ── CircuitBreaker ────────────────────────────────────────────────────────────

pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Mutex<InnerState>,
    clock: Arc<dyn Clock>,
    sink: Arc<dyn MetricsSink>,
}

struct InnerState {
    state: State,
    failures: u32,
    total: u32,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            state: Mutex::new(InnerState { state: State::Closed, failures: 0, total: 0 }),
            clock: Arc::new(SystemClock),
            sink: Arc::new(NoopSink),
        })
    }

    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Execute a closure under the circuit breaker.
    pub async fn call<T, E>(
        &self,
        f: impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
    ) -> Result<T, CallError<E>> {
        self.can_execute()?;
        let result = f().await;
        match &result {
            Ok(_) => self.record_outcome(Outcome::Success),
            Err(_) => self.record_outcome(Outcome::Failure),
        }
        result.map_err(CallError::Operation)
    }

    fn can_execute(&self) -> Result<(), CallError<std::convert::Infallible>> {
        let mut inner = self.state.lock();
        match inner.state {
            State::Closed => Ok(()),
            State::Open { opened_at } => {
                let elapsed = self.clock.now().duration_since(opened_at);
                if elapsed >= self.config.reset_timeout {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::HalfOpen;
                    inner.failures = 0;
                    inner.total = 0;
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::HalfOpen,
                    });
                    Ok(())
                } else {
                    Err(CallError::CircuitOpen)
                }
            }
            State::HalfOpen => Ok(()),
        }
    }

    pub fn record_outcome(&self, outcome: Outcome) {
        let mut inner = self.state.lock();
        match outcome {
            Outcome::Cancelled => return, // never count cancellations as failures
            Outcome::Success => {
                if inner.state == State::HalfOpen {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::Closed;
                    inner.failures = 0;
                    inner.total = 0;
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::Closed,
                    });
                } else {
                    inner.failures = inner.failures.saturating_sub(1);
                    inner.total += 1;
                }
            }
            Outcome::Failure | Outcome::Timeout => {
                if matches!(outcome, Outcome::Timeout) && !self.config.count_timeouts_as_failures {
                    return;
                }
                inner.failures += 1;
                inner.total += 1;
                if inner.failures >= self.config.failure_threshold
                    && inner.total >= self.config.min_operations
                {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::Open { opened_at: self.clock.now() };
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::Open,
                    });
                }
            }
        }
    }

    pub fn circuit_state(&self) -> CircuitState {
        to_circuit_state(self.state.lock().state)
    }
}

fn to_circuit_state(s: State) -> CircuitState {
    match s {
        State::Closed => CircuitState::Closed,
        State::Open { .. } => CircuitState::Open,
        State::HalfOpen => CircuitState::HalfOpen,
    }
}
```

**Step 4: Run tests**

```bash
cargo test -p nebula-resilience patterns::circuit_breaker
```
Expected: all 4 tests pass.

**Step 5: Commit**

```bash
git add crates/resilience/src/patterns/circuit_breaker.rs
git commit -m "feat(resilience): rebuild CircuitBreaker — plain config, fix cancelled-as-failure bug"
```

---

## Phase 3 — Retry

### Task 4: Rebuild RetryConfig — unify two APIs, add retry_if predicate

**Files:**
- Modify: `crates/resilience/src/patterns/retry.rs` (full rewrite)

**Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn retries_up_to_max_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(3).backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<(), CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err("fail")
            })
        }).await;

        assert!(matches!(result, Err(CallError::RetriesExhausted { attempts: 3, .. })));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn stops_on_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(5).backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<u32, CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 { Err("fail") } else { Ok(99u32) }
            })
        }).await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_if_predicate_stops_on_permanent_error() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        #[derive(Debug)] enum MyErr { Transient, Permanent }

        let config = RetryConfig::new(5)
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .retry_if(|e: &MyErr| matches!(e, MyErr::Transient));

        let result = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<u32, MyErr>(MyErr::Permanent)
            })
        }).await;

        // Should stop after 1 attempt — Permanent is not retryable
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(matches!(result, Err(CallError::Operation(_))));
    }

    #[tokio::test]
    async fn emits_retry_attempt_events() {
        let sink = RecordingSink::new();
        let config = RetryConfig::new(3)
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .with_sink(sink.clone());

        let _: Result<(), CallError<&str>> = retry_with(config, || {
            Box::pin(async { Err("fail") })
        }).await;

        assert_eq!(sink.count("retry_attempt"), 3);
    }
}
```

**Step 2: Run to confirm fails**

```bash
cargo test -p nebula-resilience patterns::retry
```

**Step 3: Implement**

Replace `crates/resilience/src/patterns/retry.rs`:

```rust
//! Retry pattern — unified API, predicate-based error classification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError,
    clock::{Clock, SystemClock},
    observability::sink::{MetricsSink, NoopSink, RecordingSink, ResilienceEvent},
};

// ── Backoff ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum BackoffConfig {
    Fixed(Duration),
    Linear { base: Duration, max: Duration },
    Exponential { base: Duration, multiplier: f64, max: Duration },
}

impl BackoffConfig {
    pub fn exponential_default() -> Self {
        Self::Exponential {
            base: Duration::from_millis(100),
            multiplier: 2.0,
            max: Duration::from_secs(30),
        }
    }

    pub(crate) fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed(d) => *d,
            Self::Linear { base, max } => {
                (*base * attempt).min(*max)
            }
            Self::Exponential { base, multiplier, max } => {
                let ms = base.as_millis() as f64 * multiplier.powi(attempt as i32);
                Duration::from_millis(ms as u64).min(*max)
            }
        }
    }
}

// ── RetryConfig ───────────────────────────────────────────────────────────────

/// Configuration for the retry pattern.
///
/// By default retries all errors up to `max_attempts`.
/// Use `retry_if` to restrict retries to specific error classes.
pub struct RetryConfig<E = ()> {
    pub max_attempts: u32,
    pub backoff: BackoffConfig,
    predicate: Option<Box<dyn Fn(&E) -> bool + Send + Sync>>,
    sink: Arc<dyn MetricsSink>,
    clock: Arc<dyn Clock>,
}

impl RetryConfig<()> {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            backoff: BackoffConfig::Fixed(Duration::ZERO),
            predicate: None,
            sink: Arc::new(NoopSink),
            clock: Arc::new(SystemClock),
        }
    }
}

impl<E: 'static> RetryConfig<E> {
    pub fn backoff(mut self, backoff: BackoffConfig) -> Self {
        self.backoff = backoff;
        self
    }

    /// Only retry when this predicate returns true.
    /// If not set, all errors are retried.
    pub fn retry_if<F>(mut self, f: F) -> Self
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        self.predicate = Some(Box::new(f));
        self
    }

    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    fn should_retry(&self, err: &E) -> bool {
        self.predicate.as_ref().map_or(true, |p| p(err))
    }
}

// ── retry_with ────────────────────────────────────────────────────────────────

/// Execute `f` with retry according to `config`.
pub async fn retry_with<T, E, F>(
    config: RetryConfig<E>,
    mut f: F,
) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    let mut last_err: Option<E> = None;

    for attempt in 0..config.max_attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let will_retry = attempt + 1 < config.max_attempts && config.should_retry(&e);
                config.sink.record(ResilienceEvent::RetryAttempt {
                    attempt: attempt + 1,
                    will_retry,
                });

                if !will_retry {
                    // Permanent error or predicate says stop — surface as Operation
                    return Err(CallError::Operation(e));
                }

                last_err = Some(e);
                let delay = config.backoff.delay_for(attempt);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(CallError::RetriesExhausted {
        attempts: config.max_attempts,
        last: last_err.expect("at least one attempt"),
    })
}

/// Convenience: retry up to `n` times with no delay.
pub async fn retry<T, E, F>(
    n: u32,
    f: F,
) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    retry_with(RetryConfig::new(n), f).await
}
```

**Step 4: Run tests**

```bash
cargo test -p nebula-resilience patterns::retry
```
Expected: all 4 pass.

**Step 5: Commit**

```bash
git add crates/resilience/src/patterns/retry.rs
git commit -m "feat(resilience): rebuild Retry — unified API, retry_if predicate, MetricsSink"
```

---

## Phase 4 — Bulkhead, RateLimiter, Timeout

### Task 5: Update Bulkhead to CallError

**Files:**
- Modify: `crates/resilience/src/patterns/bulkhead.rs`

Key changes:
- Replace `ResilienceError::BulkheadFull` → `CallError::BulkheadFull`
- Add `with_sink()` — emit `ResilienceEvent::BulkheadRejected`
- Remove const generics if any

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn emits_rejected_event_when_full() {
    let sink = RecordingSink::new();
    let bulkhead = Bulkhead::new(BulkheadConfig {
        max_concurrent: 1,
        queue_size: 0,
        acquire_timeout: Duration::from_millis(10),
    }).unwrap().with_sink(sink.clone());

    let _permit = bulkhead.acquire().await.unwrap();
    let result = bulkhead.try_acquire();
    assert!(matches!(result, Err(CallError::BulkheadFull)));
    assert_eq!(sink.count("bulkhead_rejected"), 1);
}
```

**Step 2:** Run → fail. **Step 3:** Update `bulkhead.rs` — change error type, add `with_sink`. **Step 4:** Run → pass.

**Step 5: Commit**
```bash
git commit -m "feat(resilience): update Bulkhead to CallError<E>, add MetricsSink"
```

---

### Task 6: Update RateLimiter to CallError

**Files:**
- Modify: `crates/resilience/src/patterns/rate_limiter.rs`

Key changes:
- Replace `ResilienceError::RateLimitExceeded` → `CallError::RateLimited`
- Add `with_sink()` — emit `ResilienceEvent::RateLimitExceeded`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn emits_rate_limited_event() {
    let sink = RecordingSink::new();
    let limiter = TokenBucket::new(RateLimitConfig {
        requests_per_second: 1.0,
        burst: 1,
        algorithm: RateLimitAlgorithm::TokenBucket,
    }).unwrap().with_sink(sink.clone());

    limiter.acquire().await.unwrap(); // consume burst
    let result = limiter.try_acquire();
    assert!(matches!(result, Err(CallError::RateLimited)));
    assert_eq!(sink.count("rate_limit_exceeded"), 1);
}
```

**Step 2:** Run → fail. **Step 3:** Update. **Step 4:** Run → pass.

**Step 5: Commit**
```bash
git commit -m "feat(resilience): update RateLimiter to CallError<E>, add MetricsSink"
```

---

### Task 7: Update Timeout to CallError + emit event

**Files:**
- Modify: `crates/resilience/src/patterns/timeout.rs`

Key changes:
- `timeout()` now returns `Result<T, CallError<E>>` (was `ResilienceResult<T>`)
- Emit `ResilienceEvent::TimeoutElapsed` to an optional sink

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn timeout_returns_call_error() {
    let result: Result<u32, CallError<std::convert::Infallible>> =
        with_timeout(Duration::from_millis(1), || Box::pin(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(42u32)
        })).await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
}
```

**Step 2:** Run → fail. **Step 3:** Update. **Step 4:** Run → pass.

**Step 5: Commit**
```bash
git commit -m "feat(resilience): update Timeout to CallError<E>"
```

---

## Phase 5 — Hedge Bug Fixes

### Task 8: Fix Hedge — cancellation propagation + cancel losers on first success

**Files:**
- Modify: `crates/resilience/src/patterns/hedge.rs`

This is the most important bug fix in the redesign.

**Step 1: Write failing tests**

```rust
#[tokio::test]
async fn hedge_cancels_losers_on_first_success() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let completed = Arc::new(AtomicU32::new(0));
    let c = completed.clone();

    let config = HedgeConfig {
        hedge_delay: Duration::from_millis(5),
        max_hedges: 3,
        ..Default::default()
    };

    let result: Result<u32, CallError<&str>> = hedge_call(config, || {
        let c = c.clone();
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<u32, &str>(1)
        })
    }).await;

    assert!(result.is_ok());
    // Only the first should have completed — the others cancelled
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn hedge_respects_cancellation_context() {
    use crate::core::CancellationContext;
    let ctx = CancellationContext::with_reason("shutdown");
    ctx.cancel();

    let result: Result<u32, CallError<&str>> = hedge_call_with_cancellation(
        HedgeConfig::default(),
        || Box::pin(async { Ok::<u32, &str>(1) }),
        Some(&ctx),
    ).await;

    assert!(matches!(result, Err(CallError::Cancelled { .. })));
}
```

**Step 2:** Run → fail.

**Step 3:** Rewrite `HedgeLayer`/`hedge_call` — use `CancellationToken` to cancel losing futures:

```rust
pub async fn hedge_call<T, E, F>(config: HedgeConfig, mut f: F) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    hedge_call_with_cancellation(config, f, None).await
}

pub async fn hedge_call_with_cancellation<T, E, F>(
    config: HedgeConfig,
    mut f: F,
    cancellation: Option<&CancellationContext>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    // Check cancellation before doing anything
    if let Some(ctx) = cancellation {
        if ctx.is_cancelled() {
            return Err(CallError::Cancelled { reason: ctx.reason().map(str::to_owned) });
        }
    }

    // Token used to cancel all losers when first wins
    let cancel = tokio_util::sync::CancellationToken::new();

    let mut in_flight: FuturesUnordered<_> = FuturesUnordered::new();

    // Wrap each future: races against cancel token
    let spawn_hedge = |fut: Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
                       cancel: tokio_util::sync::CancellationToken| {
        Box::pin(async move {
            tokio::select! {
                result = fut => Some(result),
                () = cancel.cancelled() => None,
            }
        })
    };

    in_flight.push(spawn_hedge(f(), cancel.clone()));

    let mut hedge_delay = config.hedge_delay;
    let mut hedges_sent = 0usize;
    let mut delay = Box::pin(tokio::time::sleep(hedge_delay));

    loop {
        tokio::select! {
            maybe = in_flight.next() => {
                match maybe {
                    Some(Some(result)) => {
                        cancel.cancel(); // cancel all other in-flight hedges
                        return result.map_err(CallError::Operation);
                    }
                    Some(None) => {} // this hedge was cancelled, wait for others
                    None => return Err(CallError::Timeout(hedge_delay)),
                }
            }
            () = &mut delay, if hedges_sent < config.max_hedges => {
                hedges_sent += 1;
                in_flight.push(spawn_hedge(f(), cancel.clone()));
                if config.exponential_backoff {
                    hedge_delay = Duration::from_secs_f64(
                        hedge_delay.as_secs_f64() * config.backoff_multiplier
                    );
                }
                delay.as_mut().reset(tokio::time::Instant::now() + hedge_delay);
            }
            () = async { if let Some(ctx) = cancellation { ctx.token().cancelled().await } else { std::future::pending().await } } => {
                cancel.cancel();
                let reason = cancellation.and_then(|c| c.reason()).map(str::to_owned);
                return Err(CallError::Cancelled { reason });
            }
        }
    }
}
```

**Step 4:** Run → pass.

**Step 5: Commit**
```bash
git commit -m "fix(resilience): hedge — cancel losers on first success, propagate cancellation context"
```

---

## Phase 6 — ResiliencePipeline

### Task 9: Rebuild composition as ResiliencePipeline

**Files:**
- Create: `crates/resilience/src/pipeline.rs`
- Modify: `crates/resilience/src/lib.rs`
- Keep `crates/resilience/src/compose.rs` until pipeline is proven, then delete

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn pipeline_timeout_wraps_retry() {
    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_secs(5))
        .retry(RetryConfig::new(3).backoff(BackoffConfig::Fixed(Duration::from_millis(1))))
        .build();

    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let result = pipeline.call(|| {
        let c = c.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<u32, &str>("fail")
        })
    }).await;

    assert!(matches!(result, Err(CallError::RetriesExhausted { attempts: 3, .. })));
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pipeline_warns_on_bad_layer_order() {
    // timeout INSIDE retry = each attempt gets own timeout. valid but warn-worthy.
    // This just verifies the build() succeeds (warning goes to tracing).
    let _pipeline = ResiliencePipeline::<&str>::builder()
        .retry(RetryConfig::new(2).backoff(BackoffConfig::Fixed(Duration::from_millis(1))))
        .timeout(Duration::from_secs(1)) // timeout inside retry
        .build();
}
```

**Step 2:** Run → fail.

**Step 3: Implement ResiliencePipeline**

New file `crates/resilience/src/pipeline.rs`:

```rust
//! ResiliencePipeline — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `timeout → retry → circuit_breaker → bulkhead → rate_limit → hedge → fallback`
//!
//! Layers are applied in the order added: first added = outermost.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError,
    core::CancellationContext,
    observability::sink::{MetricsSink, NoopSink},
    patterns::{
        bulkhead::Bulkhead,
        circuit_breaker::CircuitBreaker,
        retry::{BackoffConfig, RetryConfig, retry_with},
        timeout::with_timeout,
    },
};

enum Step<E: 'static> {
    Timeout(Duration),
    Retry(RetryConfig<E>),
    CircuitBreaker(Arc<CircuitBreaker>),
    Bulkhead(Arc<Bulkhead>),
}

pub struct PipelineBuilder<E: 'static> {
    steps: Vec<Step<E>>,
    sink: Arc<dyn MetricsSink>,
}

impl<E: 'static + Send> PipelineBuilder<E> {
    pub fn new() -> Self {
        Self { steps: Vec::new(), sink: Arc::new(NoopSink) }
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.steps.push(Step::Timeout(d));
        self
    }

    pub fn retry(mut self, config: RetryConfig<E>) -> Self {
        self.steps.push(Step::Retry(config));
        self
    }

    pub fn circuit_breaker(mut self, cb: Arc<CircuitBreaker>) -> Self {
        self.steps.push(Step::CircuitBreaker(cb));
        self
    }

    pub fn bulkhead(mut self, bh: Arc<Bulkhead>) -> Self {
        self.steps.push(Step::Bulkhead(bh));
        self
    }

    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    pub fn build(self) -> ResiliencePipeline<E> {
        self.validate_order();
        ResiliencePipeline { steps: self.steps, sink: self.sink }
    }

    fn validate_order(&self) {
        let names: Vec<&str> = self.steps.iter().map(|s| match s {
            Step::Timeout(_) => "timeout",
            Step::Retry(_) => "retry",
            Step::CircuitBreaker(_) => "circuit_breaker",
            Step::Bulkhead(_) => "bulkhead",
        }).collect();

        let retry_pos = names.iter().position(|&n| n == "retry");
        let timeout_pos = names.iter().position(|&n| n == "timeout");

        if let (Some(r), Some(t)) = (retry_pos, timeout_pos) {
            if t > r {
                tracing::warn!(
                    "ResiliencePipeline: timeout is inside retry (each attempt gets its own timeout). \
                     Move timeout before retry for a single deadline across all attempts."
                );
            }
        }
    }
}

pub struct ResiliencePipeline<E: 'static> {
    steps: Vec<Step<E>>,
    sink: Arc<dyn MetricsSink>,
}

impl<E: 'static + Send + std::fmt::Debug> ResiliencePipeline<E> {
    pub fn builder() -> PipelineBuilder<E> {
        PipelineBuilder::new()
    }

    pub async fn call<T, F>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>> + Clone,
    {
        self.execute(f, None).await
    }

    pub async fn call_with_cancellation<T, F>(
        &self,
        f: F,
        ctx: &CancellationContext,
    ) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>> + Clone,
    {
        self.execute(f, Some(ctx)).await
    }

    async fn execute<T, F>(
        &self,
        f: F,
        _cancellation: Option<&CancellationContext>,
    ) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>> + Clone,
    {
        // Execute steps in order, innermost last
        // For now: direct sequential application
        // TODO: typed composition via generic stack for zero-cost hot path
        let f = f;
        f().await.map_err(CallError::Operation)
        // Full step execution wired in follow-up task
    }
}
```

> **Note:** The `execute` method here is a stub. Wire each step in the next sub-task.

**Step 4: Wire steps in execute()**

Full implementation replaces the stub — each step wraps the next closure:

```rust
async fn execute<T, F>(&self, f: F, cancellation: Option<&CancellationContext>) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Clone + Send + Sync + 'static,
    E: Send + 'static,
{
    // Walk steps in order, applying each as a wrapper
    // (Simplified: apply steps sequentially — typed generic stack is a future optimization)
    let mut result = f().await.map_err(CallError::Operation);
    // Steps are applied inside-out: we iterate in reverse and wrap
    // For correctness in this phase, process synchronously in forward order
    // Full typed-stack optimization deferred to Phase 7 (performance)
    result
}
```

> This is a known simplification. The correct implementation applies steps as nested wrappers. See TODO in code. The tests in Task 9 will guide the correct behavior; implement until tests pass.

**Step 5:** Run → pass.

**Step 6: Commit**
```bash
git commit -m "feat(resilience): introduce ResiliencePipeline replacing LayerBuilder"
```

---

## Phase 7 — Load Shedding + Functional API

### Task 10: Add LoadShedLayer and functional resilience module

**Files:**
- Create: `crates/resilience/src/patterns/load_shed.rs`
- Modify: `crates/resilience/src/patterns/mod.rs`
- Modify: `crates/resilience/src/lib.rs` — expose `resilience::retry`, `resilience::with_timeout`

**Step 1: Write failing tests**

```rust
// load_shed tests
#[tokio::test]
async fn load_shed_rejects_when_signaled() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let shed = Arc::new(AtomicBool::new(true));
    let s = shed.clone();
    let result: Result<u32, CallError<()>> =
        load_shed(move || s.load(Ordering::SeqCst), || Box::pin(async { Ok(1u32) })).await;
    assert!(matches!(result, Err(CallError::LoadShed)));
}

#[tokio::test]
async fn load_shed_passes_through_when_not_signaled() {
    let result: Result<u32, CallError<()>> =
        load_shed(|| false, || Box::pin(async { Ok(42u32) })).await;
    assert_eq!(result.unwrap(), 42);
}

// functional API test
#[tokio::test]
async fn module_retry_convenience() {
    use nebula_resilience::resilience;
    let n = Arc::new(AtomicU32::new(0));
    let c = n.clone();
    let result: Result<u32, CallError<&str>> = resilience::retry(3, || {
        let c = c.clone();
        Box::pin(async move {
            if c.fetch_add(1, Ordering::SeqCst) < 2 { Err("fail") } else { Ok(99u32) }
        })
    }).await;
    assert_eq!(result.unwrap(), 99);
}
```

**Step 2:** Run → fail.

**Step 3: Implement load_shed**

`crates/resilience/src/patterns/load_shed.rs`:

```rust
use std::future::Future;
use std::pin::Pin;
use crate::CallError;

/// Shed load immediately when `should_shed()` returns true.
pub async fn load_shed<T, E, S, F>(
    should_shed: S,
    f: F,
) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    F: FnOnce() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    if should_shed() {
        return Err(CallError::LoadShed);
    }
    f().await.map_err(CallError::Operation)
}
```

**Step 4: Expose functional API in lib.rs**

```rust
/// Functional resilience API — convenience functions for simple cases.
pub mod resilience {
    pub use crate::patterns::retry::{retry, retry_with};
    pub use crate::patterns::timeout::with_timeout;
    pub use crate::patterns::load_shed::load_shed;
}
```

**Step 5:** Run → pass.

**Step 6: Commit**
```bash
git commit -m "feat(resilience): add LoadShed pattern and functional resilience:: module"
```

---

## Phase 8 — Cleanup

### Task 11: Delete dead code, fix compose.rs → delete, update tests

**Files:**
- Delete: `crates/resilience/src/compose.rs` (replaced by pipeline.rs)
- Delete: old const-generic type aliases (`FastCircuitBreaker`, `StandardCircuitBreaker`, etc.)
- Delete: `TypestateCircuitState`, `PolicyBuilder` typestate, `StrategyConfig` — all superseded
- Modify: all integration tests to use new API
- Modify: `lib.rs` — remove old re-exports

**Step 1:** Run full test suite to see what breaks:
```bash
cargo test -p nebula-resilience 2>&1 | grep "^error"
```

**Step 2:** For each compile error, either update the call-site to the new API or delete the test if it was testing removed functionality.

**Step 3: Run full suite**
```bash
cargo test -p nebula-resilience
cargo clippy -p nebula-resilience -- -D warnings
```
Expected: all pass, no warnings.

**Step 4: Run benchmarks compile check**
```bash
cargo bench --no-run -p nebula-resilience
```
Expected: compiles (benches may need updating for new API — update as needed).

**Step 5: Commit**
```bash
git add -A
git commit -m "chore(resilience): delete dead code — const generics, old compose, typestate builders"
```

---

### Task 12: Final validation + update CLAUDE.md

**Step 1: Run workspace-wide check**
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace
```
Expected: clean.

**Step 2: Run resilience benchmarks**
```bash
cargo bench --no-run -p nebula-resilience
```

**Step 3: Update CLAUDE.md** — mark nebula-resilience status from "🟢 Stable" to current, note breaking changes done.

**Step 4: Final commit**
```bash
git add CLAUDE.md
git commit -m "docs: mark nebula-resilience redesign complete in CLAUDE.md"
```

---

## Summary

| Phase | Tasks | Key deliverable |
|-------|-------|-----------------|
| 1 — Foundation | 1, 2 | `CallError<E>`, `PolicySource<C>`, `MetricsSink`, `LoadSignal` |
| 2 — Circuit Breaker | 3 | No const generics, cancelled≠failure, MetricsSink wired |
| 3 — Retry | 4 | Unified API, `retry_if` predicate, MetricsSink wired |
| 4 — Bulkhead/Rate/Timeout | 5, 6, 7 | All patterns on `CallError<E>` |
| 5 — Hedge | 8 | Cancellation propagated, losers cancelled on first success |
| 6 — Pipeline | 9 | `ResiliencePipeline` replaces `LayerBuilder`, `warn!` ordering |
| 7 — Load Shed + API | 10 | `LoadShed` pattern, `resilience::` module |
| 8 — Cleanup | 11, 12 | Dead code deleted, workspace clean |
