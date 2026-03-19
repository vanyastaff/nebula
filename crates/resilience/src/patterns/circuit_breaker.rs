//! Circuit breaker pattern — plain-struct config, injectable sink and clock.

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError, ConfigError,
    clock::{Clock, SystemClock},
    observability::sink::{CircuitState, MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the circuit breaker pattern.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit. Min: 1.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to HalfOpen.
    pub reset_timeout: Duration,
    /// Max concurrent probe operations allowed in HalfOpen state. Default: 1.
    pub half_open_max_ops: u32,
    /// Minimum number of operations required before the failure rate can trip the breaker. Default: 5.
    pub min_operations: u32,
    /// Failure rate (0.0..=1.0) that triggers opening. Default: 0.5.
    pub failure_rate_threshold: f64,
    /// Sliding window for failure counting. Default: 60s.
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
    /// Validate configuration. Called by `CircuitBreaker::new()`.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.failure_threshold == 0 {
            return Err(ConfigError::new("failure_threshold", "must be >= 1"));
        }
        if self.reset_timeout.is_zero() {
            return Err(ConfigError::new("reset_timeout", "must be > 0"));
        }
        if !(0.0..=1.0).contains(&self.failure_rate_threshold) {
            return Err(ConfigError::new(
                "failure_rate_threshold",
                "must be 0.0..=1.0",
            ));
        }
        Ok(())
    }
}

// ── Outcome (internal) ────────────────────────────────────────────────────────

/// The outcome of an operation, used to update circuit breaker state.
#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    /// Operation succeeded.
    Success,
    /// Operation failed.
    Failure,
    /// Operation timed out.
    Timeout,
    /// Operation was cancelled — never counted as a failure.
    Cancelled,
}

// ── State machine (internal) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Closed,
    Open { opened_at: std::time::Instant },
    HalfOpen,
}

// ── CircuitBreaker ────────────────────────────────────────────────────────────

/// Snapshot of circuit breaker state for health reporting.
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    /// Current circuit state.
    pub state: CircuitState,
    /// Current failure count.
    pub failures: u32,
    /// Total operations in current window.
    pub total: u32,
}

/// Circuit breaker — protects downstream calls by rejecting requests when failure rate is high.
///
/// Shared state via `Arc<CircuitBreaker>`. Inject [`MockClock`] and [`RecordingSink`] for tests.
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
    /// Create a new circuit breaker with the given configuration.
    ///
    /// Returns `Err(ConfigError)` if configuration is invalid.
    pub fn new(config: CircuitBreakerConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            state: Mutex::new(InnerState {
                state: State::Closed,
                failures: 0,
                total: 0,
            }),
            clock: Arc::new(SystemClock),
            sink: Arc::new(NoopSink),
        })
    }

    /// Replace the metrics sink (builder-style).
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Replace the clock (builder-style, for testing).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Execute a closure under the circuit breaker.
    ///
    /// Returns `Err(CallError::CircuitOpen)` immediately if the breaker is open.
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

    fn can_execute<E>(&self) -> Result<(), CallError<E>> {
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

    /// Record an operation outcome directly (useful when driving the CB from external code).
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
                    inner.total = inner.total.saturating_add(1);
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
                    inner.state = State::Open {
                        opened_at: self.clock.now(),
                    };
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::Open,
                    });
                }
            }
        }
    }

    /// Returns the current circuit state.
    pub fn circuit_state(&self) -> CircuitState {
        to_circuit_state(self.state.lock().state)
    }

    /// Returns a stats snapshot.
    pub fn stats(&self) -> CircuitBreakerStats {
        let inner = self.state.lock();
        CircuitBreakerStats {
            state: to_circuit_state(inner.state),
            failures: inner.failures,
            total: inner.total,
        }
    }

    /// Alias for `new()` — backward compat with manager.rs.
    pub fn with_config(config: CircuitBreakerConfig) -> Result<Self, ConfigError> {
        Self::new(config)
    }
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("CircuitBreaker")
            .field("state", &stats.state)
            .field("failures", &stats.failures)
            .field("total", &stats.total)
            .finish()
    }
}

fn to_circuit_state(s: State) -> CircuitState {
    match s {
        State::Closed => CircuitState::Closed,
        State::Open { .. } => CircuitState::Open,
        State::HalfOpen => CircuitState::HalfOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::sink::CircuitState as CS;
    use crate::{CallError, RecordingSink};
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
            let _ = cb.call::<(), _>(|| Box::pin(async { Err("fail") })).await;
        }
        let err: CallError<&str> = cb
            .call::<(), _>(|| Box::pin(async { Ok(()) }))
            .await
            .unwrap_err();
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
        let result = cb.call::<u32, &str>(|| Box::pin(async { Ok(42) })).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn emits_state_change_event_on_open() {
        let sink = RecordingSink::new();
        let cb = CircuitBreaker::new(default_config())
            .unwrap()
            .with_sink(sink.clone());
        for _ in 0..3 {
            let _ = cb
                .call::<(), &str>(|| Box::pin(async { Err("fail") }))
                .await;
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
