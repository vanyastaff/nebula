//! Circuit breaker pattern — plain-struct config, injectable sink and clock.

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError, ConfigError,
    clock::{Clock, SystemClock},
    sink::{CircuitState, MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the circuit breaker pattern.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit. Min: 1.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to `HalfOpen`.
    pub reset_timeout: Duration,
    /// Max concurrent probe operations allowed in `HalfOpen` state. Default: 1.
    pub half_open_max_ops: u32,
    /// Minimum number of operations required before failures can trip the breaker. Default: 5.
    pub min_operations: u32,
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
            count_timeouts_as_failures: true,
        }
    }
}

impl CircuitBreakerConfig {
    /// Validate configuration. Called by `CircuitBreaker::new()`.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `failure_threshold` is 0, `reset_timeout` is zero,
    /// or `half_open_max_ops` is 0.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.failure_threshold == 0 {
            return Err(ConfigError::new("failure_threshold", "must be >= 1"));
        }
        if self.reset_timeout.is_zero() {
            return Err(ConfigError::new("reset_timeout", "must be > 0"));
        }
        if self.half_open_max_ops == 0 {
            return Err(ConfigError::new("half_open_max_ops", "must be >= 1"));
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
///
/// # Cancel safety
///
/// [`call()`](CircuitBreaker::call) is cancel-safe with respect to the half-open probe count.
/// If the future returned by `call()` is dropped before completion (e.g. via `tokio::select!`),
/// the probe slot is automatically released via `record_outcome(Cancelled)`.
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
    /// Number of active probe operations in `HalfOpen` state.
    half_open_probes: u32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    ///
    /// # Errors
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
                half_open_probes: 0,
            }),
            clock: Arc::new(SystemClock),
            sink: Arc::new(NoopSink),
        })
    }

    /// Replace the metrics sink (builder-style).
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Replace the clock (builder-style, for testing).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Execute a closure under the circuit breaker.
    ///
    /// If the returned future is dropped before completion, the probe slot
    /// (if in `HalfOpen` state) is automatically released.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` if the breaker is open,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call<T, E>(
        &self,
        f: impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
    ) -> Result<T, CallError<E>> {
        self.can_execute()?;
        let guard = ProbeGuard(self);
        let result = f().await;
        let outcome = if result.is_ok() {
            Outcome::Success
        } else {
            Outcome::Failure
        };
        // Defuse the guard — we'll record the real outcome instead.
        std::mem::forget(guard);
        self.record_outcome(outcome);
        result.map_err(CallError::Operation)
    }

    /// Check if the circuit allows execution.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` when the circuit is open
    /// or the half-open probe limit has been reached.
    pub fn can_execute<E>(&self) -> Result<(), CallError<E>> {
        let mut inner = self.state.lock();
        match inner.state {
            State::Closed => Ok(()),
            State::HalfOpen => {
                if inner.half_open_probes >= self.config.half_open_max_ops {
                    Err(CallError::CircuitOpen)
                } else {
                    inner.half_open_probes = inner.half_open_probes.saturating_add(1);
                    Ok(())
                }
            }
            State::Open { opened_at } => {
                let elapsed = self.clock.now().duration_since(opened_at);
                if elapsed >= self.config.reset_timeout {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::HalfOpen;
                    inner.failures = 0;
                    inner.total = 0;
                    inner.half_open_probes = 1; // this call is the first probe
                    drop(inner);
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::HalfOpen,
                    });
                    Ok(())
                } else {
                    Err(CallError::CircuitOpen)
                }
            }
        }
    }

    /// Record an operation outcome directly (useful when driving the CB from external code).
    ///
    /// In the Closed state, each success decrements the failure counter by one ("leaky bucket"
    /// forgiveness). This means that interleaved successes slowly erase past failures,
    /// preventing the breaker from tripping on intermittent errors.
    // Reason: the lock guard is held intentionally across the entire match to ensure
    // atomic state transitions — dropping early would create a TOCTOU window.
    #[allow(clippy::significant_drop_tightening)]
    pub fn record_outcome(&self, outcome: Outcome) {
        let mut inner = self.state.lock();
        match outcome {
            Outcome::Cancelled => {
                // Never count cancellations as failures, but release the probe slot
                // so that half-open probes aren't permanently leaked on drop/cancel.
                inner.half_open_probes = inner.half_open_probes.saturating_sub(1);
            }
            Outcome::Success => {
                if inner.state == State::HalfOpen {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::Closed;
                    inner.failures = 0;
                    inner.total = 0;
                    inner.half_open_probes = 0;
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
                inner.failures = inner.failures.saturating_add(1);
                inner.total = inner.total.saturating_add(1);

                if inner.state == State::HalfOpen {
                    // Any failure in HalfOpen sends us back to Open.
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::Open {
                        opened_at: self.clock.now(),
                    };
                    inner.half_open_probes = 0;
                    self.sink.record(ResilienceEvent::CircuitStateChanged {
                        from: prev,
                        to: CircuitState::Open,
                    });
                } else if inner.failures >= self.config.failure_threshold
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
}

/// RAII guard that records `Cancelled` on drop if the operation is abandoned.
///
/// Used by `call()` and the pipeline's CB step to ensure half-open probe slots
/// are released when the future is dropped (e.g. by `tokio::select!` or a timeout).
/// Call `std::mem::forget(guard)` to defuse it before recording the real outcome.
pub(crate) struct ProbeGuard<'a>(pub(crate) &'a CircuitBreaker);

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        self.0.record_outcome(Outcome::Cancelled);
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

const fn to_circuit_state(s: State) -> CircuitState {
    match s {
        State::Closed => CircuitState::Closed,
        State::Open { .. } => CircuitState::Open,
        State::HalfOpen => CircuitState::HalfOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::CircuitState as CS;
    use crate::{CallError, RecordingSink};
    use std::time::Duration;

    fn default_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_millis(100),
            half_open_max_ops: 1,
            min_operations: 1,
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
        for _ in 0..10 {
            cb.record_outcome(Outcome::Cancelled);
        }
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

    #[tokio::test]
    async fn half_open_enforces_max_probes() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            half_open_max_ops: 1,
            ..default_config()
        })
        .unwrap();

        // Trip the breaker
        for _ in 0..3 {
            cb.record_outcome(Outcome::Failure);
        }
        assert_eq!(cb.circuit_state(), CS::Open);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(110)).await;

        // First probe should succeed (transitions to HalfOpen)
        assert!(cb.can_execute::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Second probe should be rejected (max_probes=1 reached)
        assert!(matches!(
            cb.can_execute::<&str>(),
            Err(CallError::CircuitOpen)
        ));

        // After the probe succeeds, breaker closes and allows new calls
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);
        assert!(cb.can_execute::<&str>().is_ok());
    }

    #[tokio::test]
    async fn half_open_failure_reopens_breaker() {
        let sink = RecordingSink::new();
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            half_open_max_ops: 1,
            ..default_config()
        })
        .unwrap()
        .with_sink(sink.clone());

        // Trip the breaker
        for _ in 0..3 {
            cb.record_outcome(Outcome::Failure);
        }

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(110)).await;

        // Enter HalfOpen
        assert!(cb.can_execute::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Probe fails → back to Open
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);
    }

    #[tokio::test]
    async fn dropped_call_releases_probe_slot() {
        let cb = Arc::new(
            CircuitBreaker::new(CircuitBreakerConfig {
                half_open_max_ops: 1,
                ..default_config()
            })
            .unwrap(),
        );

        // Trip the breaker
        for _ in 0..3 {
            cb.record_outcome(Outcome::Failure);
        }

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(110)).await;

        // Start a call that will be dropped mid-operation
        let cb2 = Arc::clone(&cb);
        tokio::select! {
            _ = cb2.call::<(), &str>(|| Box::pin(async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(())
            })) => unreachable!(),
            _ = tokio::time::sleep(Duration::from_millis(5)) => {
                // Future dropped — probe guard should release the slot
            }
        }

        // The probe slot should be freed. Wait for reset again and try a new probe.
        // Since the cancelled probe decremented half_open_probes, the next
        // Open→HalfOpen transition should work.
        tokio::time::sleep(Duration::from_millis(110)).await;

        // This must succeed — the probe slot was properly released
        assert!(cb.can_execute::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Complete the probe successfully
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);
    }
}
