//! Circuit breaker pattern — plain-struct config, injectable sink and clock.
//!
//! # Examples
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let cb = CircuitBreaker::new(CircuitBreakerConfig {
//!     failure_threshold: 3,
//!     reset_timeout: Duration::from_secs(30),
//!     min_operations: 1,
//!     ..Default::default()
//! })
//! .expect("valid config");
//!
//! let value = cb
//!     .call(|| Box::pin(async { Ok::<_, &str>("ok") }))
//!     .await
//!     .unwrap();
//! assert_eq!(value, "ok");
//! # }
//! ```

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;

use crate::{
    CallContext, CallError, ConfigError,
    clock::{InstantSource, SystemInstant},
    events::{EventSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the circuit breaker pattern.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit. Min: 1.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to `HalfOpen`.
    pub reset_timeout: Duration,
    /// Max concurrent probe operations allowed in `HalfOpen` state. Default: 1.
    pub max_half_open_operations: u32,
    /// Successful half-open probes required before closing.
    ///
    /// `None` means use `max_half_open_operations`, so configurations that admit multiple
    /// concurrent probes also require multiple successful probes before recovery.
    pub half_open_success_threshold: Option<u32>,
    /// Minimum number of operations required before failures can trip the breaker. Default: 5.
    pub min_operations: u32,
    /// Whether timeouts count as failures **and toward `total` operations**.
    /// When `false`, timeouts are completely ignored by the circuit breaker —
    /// they do not count as failures, successes, or toward `min_operations`.
    /// Default: `true`.
    pub count_timeouts_as_failures: bool,
    /// Multiplier applied to `base_reset_timeout` on consecutive opens.
    /// Default: 1.0 (no increase).
    #[cfg_attr(feature = "serde", serde(alias = "break_duration_multiplier"))]
    pub reset_timeout_multiplier: f64,
    /// Maximum reset timeout cap when the multiplier is active. Default: 5 minutes.
    #[cfg_attr(feature = "serde", serde(alias = "max_break_duration"))]
    pub max_reset_timeout: Duration,
    /// Duration threshold above which a successful call is considered "slow". `None` = disabled.
    #[cfg_attr(feature = "serde", serde(default))]
    pub slow_call_threshold: Option<Duration>,
    /// Slow call rate threshold (0.0--1.0). If slow calls / total >= this, CB trips. Default: 1.0.
    pub slow_call_rate_threshold: f64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            max_half_open_operations: 1,
            half_open_success_threshold: None,
            min_operations: 5,
            count_timeouts_as_failures: true,
            reset_timeout_multiplier: 1.0,
            max_reset_timeout: Duration::from_mins(5),
            slow_call_threshold: None,
            slow_call_rate_threshold: 1.0,
        }
    }
}

impl CircuitBreakerConfig {
    /// Validate configuration. Called by `CircuitBreaker::new()`.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `failure_threshold` is 0, `reset_timeout` is zero,
    /// or `max_half_open_operations` is 0.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.failure_threshold == 0 {
            return Err(ConfigError::new("failure_threshold", "must be >= 1"));
        }
        if self.reset_timeout.is_zero() {
            return Err(ConfigError::new("reset_timeout", "must be > 0"));
        }
        if self.max_half_open_operations == 0 {
            return Err(ConfigError::new("max_half_open_operations", "must be >= 1"));
        }
        if self.half_open_success_threshold == Some(0) {
            return Err(ConfigError::new(
                "half_open_success_threshold",
                "must be >= 1 when set",
            ));
        }
        if self.min_operations == 0 {
            return Err(ConfigError::new("min_operations", "must be >= 1"));
        }
        if self.reset_timeout_multiplier < 1.0 {
            return Err(ConfigError::new(
                "reset_timeout_multiplier",
                "must be >= 1.0",
            ));
        }
        if !(0.0..=1.0).contains(&self.slow_call_rate_threshold) {
            return Err(ConfigError::new(
                "slow_call_rate_threshold",
                "must be between 0.0 and 1.0",
            ));
        }
        Ok(())
    }
}

// ── Outcome (internal) ────────────────────────────────────────────────────────

/// The outcome of an operation, used to update circuit breaker state.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Outcome {
    /// Operation succeeded.
    Success,
    /// Operation failed.
    Failure,
    /// Operation timed out.
    Timeout,
    /// Operation was cancelled — never counted as a failure.
    Cancelled,
    /// Operation succeeded but exceeded the slow call threshold.
    SlowSuccess,
    /// Operation failed and exceeded the slow call threshold.
    SlowFailure,
}

impl From<crate::classifier::ErrorClass> for Outcome {
    /// Map an [`ErrorClass`](crate::classifier::ErrorClass) to a circuit breaker [`Outcome`].
    ///
    /// - `Cancelled`, `Overload`, `Permanent` → `Cancelled` (don't trip)
    /// - `Timeout` → `Timeout` (respects `count_timeouts_as_failures`)
    /// - `Transient`, `Unavailable`, `Unknown` → `Failure` (trips breaker)
    fn from(class: crate::classifier::ErrorClass) -> Self {
        use crate::classifier::ErrorClass;
        match class {
            ErrorClass::Cancelled | ErrorClass::Overload | ErrorClass::Permanent => Self::Cancelled,
            ErrorClass::Timeout => Self::Timeout,
            ErrorClass::Transient | ErrorClass::Unavailable | ErrorClass::Unknown => Self::Failure,
        }
    }
}

// ── State machine (internal) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Closed,
    Open { opened_at: std::time::Instant },
    HalfOpen,
}

const STATE_CLOSED: u32 = 0;
const STATE_OPEN: u32 = 1;
const STATE_HALF_OPEN: u32 = 2;

// ── CircuitBreaker ────────────────────────────────────────────────────────────

/// Snapshot of circuit breaker state for health reporting.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitBreakerStats {
    /// Current circuit state.
    pub state: CircuitState,
    /// Current failure count.
    pub failures: u32,
    /// Total operations observed since the counters were last reset.
    pub total: u32,
    /// Number of slow calls observed since the counters were last reset.
    pub slow_calls: u32,
}

/// A state in the circuit breaker state machine.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Breaker tripped — requests rejected immediately.
    Open,
    /// Probing — limited requests allowed to test recovery.
    HalfOpen,
}

type StateChangeCallback = Box<dyn Fn(CircuitState, CircuitState) + Send + Sync>;

/// Circuit breaker — protects downstream calls by rejecting requests when failure rate is high.
///
/// Shared state via `Arc<CircuitBreaker>`. Inject [`MockInstant`](crate::clock::MockInstant) and
/// [`RecordingSink`](crate::RecordingSink) for tests.
///
/// # Cancel safety
///
/// [`call()`](CircuitBreaker::call) is cancel-safe with respect to the half-open probe count.
/// If the future returned by `call()` is dropped before completion (e.g. via `tokio::select!`),
/// the probe slot is automatically released via `record_outcome(Cancelled)`.
/// Layout: `atomic_state` first so `circuit_state()` shares cache line 0
/// with the start of `config`, instead of sitting alone on a 5th line.
/// `repr(C)` locks field order — without it, rustc pushes `AtomicU32`
/// (align 4) to the end after all 8-byte-aligned fields.
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_resilience::{
///     CallError,
///     circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let cb = Arc::new(
///     CircuitBreaker::new(CircuitBreakerConfig {
///         failure_threshold: 2,
///         reset_timeout: Duration::from_millis(100),
///         min_operations: 1,
///         ..Default::default()
///     })
///     .expect("valid config"),
/// );
///
/// // Drive it through a couple of failures to trip the circuit.
/// for _ in 0..2 {
///     let _: Result<(), CallError<&str>> = cb
///         .call(|| Box::pin(async { Err::<(), _>("upstream down") }))
///         .await;
/// }
///
/// // Subsequent calls are short-circuited until reset_timeout elapses.
/// let err: CallError<&str> = cb
///     .call::<(), _, _>(|| Box::pin(async { Ok(()) }))
///     .await
///     .unwrap_err();
/// assert!(matches!(err, CallError::CircuitOpen));
/// # }
/// ```
#[repr(C)]
pub struct CircuitBreaker {
    /// Lock-free state mirror for observability. Offset 0 = cache line 0.
    atomic_state: AtomicU32,
    config: CircuitBreakerConfig,
    instant_source: Arc<dyn InstantSource>,
    sink: Arc<dyn EventSink>,
    state: Mutex<InnerState>,
    on_state_change: Option<StateChangeCallback>,
}

struct InnerState {
    state: State,
    failures: u32,
    total: u32,
    /// Number of active probe operations in `HalfOpen` state.
    half_open_probes: u32,
    /// Number of successful probes observed in the current `HalfOpen` recovery round.
    half_open_successes: u32,
    /// Number of consecutive times the circuit has opened (for dynamic break duration).
    consecutive_opens: u32,
    /// Number of slow calls observed since the counters were last reset.
    slow_calls: u32,
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
            atomic_state: AtomicU32::new(STATE_CLOSED),
            state: Mutex::new(InnerState {
                state: State::Closed,
                failures: 0,
                total: 0,
                half_open_probes: 0,
                half_open_successes: 0,
                consecutive_opens: 0,
                slow_calls: 0,
            }),
            instant_source: Arc::new(SystemInstant),
            sink: Arc::new(NoopSink),
            on_state_change: None,
        })
    }

    /// Replace the metrics sink (builder-style).
    #[must_use]
    pub fn with_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Replace the monotonic instant source (builder-style, for testing).
    ///
    /// Named `instant_source` rather than `clock` because this crate's source
    /// carries only `Instant`; `nebula_core::accessor::Clock` (wall time plus
    /// monotonic) is a different contract on a different type.
    #[must_use]
    pub fn with_instant_source(mut self, source: Arc<dyn InstantSource>) -> Self {
        self.instant_source = source;
        self
    }

    /// Register a callback for circuit state transitions.
    #[must_use]
    pub fn on_state_change<F>(mut self, f: F) -> Self
    where
        F: Fn(CircuitState, CircuitState) + Send + Sync + 'static,
    {
        self.on_state_change = Some(Box::new(f));
        self
    }

    /// Classify an operation result with timing information.
    ///
    /// If `slow_call_threshold` is configured and `duration` exceeds it,
    /// returns `SlowSuccess`/`SlowFailure` instead of `Success`/`Failure`.
    #[must_use]
    pub fn classify_outcome(&self, success: bool, duration: Duration) -> Outcome {
        let is_slow = self
            .config
            .slow_call_threshold
            .is_some_and(|threshold| duration >= threshold);
        match (success, is_slow) {
            (true, false) => Outcome::Success,
            (true, true) => Outcome::SlowSuccess,
            (false, false) => Outcome::Failure,
            (false, true) => Outcome::SlowFailure,
        }
    }

    /// Map a classified error to a circuit-breaker outcome while preserving
    /// timeout-specific config and slow-call accounting.
    #[must_use]
    pub(crate) fn classify_error_outcome(
        &self,
        class: crate::classifier::ErrorClass,
        duration: Duration,
    ) -> Outcome {
        match class {
            crate::classifier::ErrorClass::Timeout => Outcome::Timeout,
            class if class.counts_as_failure() => self.classify_outcome(false, duration),
            class => class.into(),
        }
    }

    /// Returns true when operation duration must be measured for slow-call accounting.
    #[must_use]
    pub(crate) const fn tracks_slow_calls(&self) -> bool {
        self.config.slow_call_threshold.is_some()
    }

    /// Return the current instant from the breaker's instant source.
    #[must_use]
    pub(crate) fn monotonic_now(&self) -> std::time::Instant {
        self.instant_source.now()
    }

    /// Manually force the circuit open, rejecting all calls until reset timeout or
    /// [`force_close`](Self::force_close).
    pub fn force_open(&self) {
        let mut inner = self.state.lock();
        let prev = to_circuit_state(inner.state);
        inner.state = State::Open {
            opened_at: self.instant_source.now(),
        };
        inner.half_open_probes = 0;
        inner.half_open_successes = 0;
        self.atomic_state.store(STATE_OPEN, Ordering::Relaxed);
        drop(inner);
        if prev != CircuitState::Open {
            self.sink.record(ResilienceEvent::CircuitStateChanged {
                from: prev,
                to: CircuitState::Open,
            });
            if let Some(ref cb) = self.on_state_change {
                cb(prev, CircuitState::Open);
            }
        }
    }

    /// Manually close the circuit, resetting all counters.
    pub fn force_close(&self) {
        let mut inner = self.state.lock();
        let prev = to_circuit_state(inner.state);
        Self::reset_counters(&mut inner);
        self.atomic_state.store(STATE_CLOSED, Ordering::Relaxed);
        drop(inner);
        if prev != CircuitState::Closed {
            self.sink.record(ResilienceEvent::CircuitStateChanged {
                from: prev,
                to: CircuitState::Closed,
            });
            if let Some(ref cb) = self.on_state_change {
                cb(prev, CircuitState::Closed);
            }
        }
    }

    // Reason: u32 cast to i32 for powi is safe within realistic consecutive_opens range.
    #[expect(
        clippy::cast_possible_wrap,
        reason = "u32 cast to i32 for powi is safe within realistic consecutive_opens range"
    )]
    fn effective_reset_timeout(&self, consecutive_opens: u32) -> Duration {
        if consecutive_opens <= 1 || self.config.reset_timeout_multiplier <= 1.0 {
            return self.config.reset_timeout;
        }
        let exponent = consecutive_opens - 1;
        let max_secs = self.config.max_reset_timeout.as_secs_f64();
        let multiplied = (self.config.reset_timeout.as_secs_f64()
            * self.config.reset_timeout_multiplier.powi(exponent as i32))
        .min(max_secs);
        Duration::from_secs_f64(multiplied)
    }

    fn required_half_open_successes(&self) -> u32 {
        self.config
            .half_open_success_threshold
            .unwrap_or(self.config.max_half_open_operations)
    }

    /// Execute a closure under the circuit breaker.
    ///
    /// All errors count as failures (equivalent to
    /// [`AlwaysTransient`](crate::classifier::AlwaysTransient) classifier).
    /// Use [`call_with_classifier`](Self::call_with_classifier) for
    /// error-type-aware outcome mapping.
    ///
    /// If the returned future is dropped before completion, the probe slot
    /// (if in `HalfOpen` state) is automatically released.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` if the breaker is open,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let cb = CircuitBreaker::new(CircuitBreakerConfig::default()).expect("valid config");
    ///
    /// let value = cb
    ///     .call(|| Box::pin(async { Ok::<_, &str>(42u32) }))
    ///     .await
    ///     .unwrap();
    /// assert_eq!(value, 42);
    /// # }
    /// ```
    pub async fn call<T, E, Fut>(&self, f: impl FnOnce() -> Fut) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.try_acquire()?;
        let mut guard = ProbeGuard::new(self);
        let start = self.instant_source.now();
        let result = f().await;
        let duration = self.instant_source.now().duration_since(start);
        let outcome = self.classify_outcome(result.is_ok(), duration);
        guard.defuse();
        self.record_outcome(outcome);
        result.map_err(CallError::Operation)
    }

    /// Execute a closure under the circuit breaker with a shared policy context.
    ///
    /// Context cancellation/deadline bounds the operation while preserving the
    /// circuit breaker state-machine guarantees. Context cancellation is recorded
    /// as a cancelled outcome and does not trip the breaker; context deadline
    /// expiry is recorded as a timeout outcome.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` if the breaker is open,
    /// `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline expires,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call_with_context<T, E, Fut>(
        &self,
        context: &CallContext,
        f: impl FnOnce() -> Fut + Send,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.call_with_context_inner(context, None, f).await
    }

    /// Execute a closure under the circuit breaker with error classification.
    ///
    /// Uses the provided `ErrorClassifier` to determine how each error
    /// affects the circuit state:
    ///
    /// | `ErrorClass` | CB outcome |
    /// |------------------------------|--------------------------------------|
    /// | `Cancelled`, `Overload` | `Cancelled` — doesn't trip breaker |
    /// | `Permanent` | `Cancelled` — downstream is healthy |
    /// | `Timeout` | `Timeout` — respects `count_timeouts` |
    /// | `Transient`, `Unavailable`, `Unknown` | `Failure` / `SlowFailure` |
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` if the breaker is open,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call_with_classifier<T, E, Fut>(
        &self,
        classifier: &dyn crate::classifier::ErrorClassifier<E>,
        f: impl FnOnce() -> Fut,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.try_acquire()?;
        let mut guard = ProbeGuard::new(self);
        let start = self.instant_source.now();
        let result = f().await;
        let duration = self.instant_source.now().duration_since(start);

        let outcome = match &result {
            Ok(_) => self.classify_outcome(true, duration),
            Err(e) => self.classify_error_outcome(classifier.classify(e), duration),
        };

        guard.defuse();
        self.record_outcome(outcome);
        result.map_err(CallError::Operation)
    }

    /// Execute a closure under the circuit breaker with both error
    /// classification and a shared policy context.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` if the breaker is open,
    /// `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline expires,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call_with_classifier_and_context<T, E, Fut>(
        &self,
        classifier: &dyn crate::classifier::ErrorClassifier<E>,
        context: &CallContext,
        f: impl FnOnce() -> Fut + Send,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.call_with_context_inner(context, Some(classifier), f)
            .await
    }

    async fn call_with_context_inner<T, E, Fut>(
        &self,
        context: &CallContext,
        classifier: Option<&dyn crate::classifier::ErrorClassifier<E>>,
        f: impl FnOnce() -> Fut + Send,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.try_acquire()?;
        let mut guard = ProbeGuard::new(self);
        let start = self.instant_source.now();
        let result = context
            .run_result(async { f().await.map_err(CallError::Operation) })
            .await;
        let duration = self.instant_source.now().duration_since(start);

        let outcome = match &result {
            Ok(_) => self.classify_outcome(true, duration),
            Err(CallError::Operation(error)) => classifier.map_or_else(
                || self.classify_outcome(false, duration),
                |classifier| self.classify_error_outcome(classifier.classify(error), duration),
            ),
            Err(CallError::Timeout(_)) => Outcome::Timeout,
            Err(CallError::Cancelled { .. }) => Outcome::Cancelled,
            Err(_) => self.classify_outcome(false, duration),
        };

        guard.defuse();
        self.record_outcome(outcome);
        result
    }

    /// Check if the circuit allows execution, **taking a half-open probe
    /// slot when it does**.
    ///
    /// This is not a read-only predicate: in `HalfOpen` it increments the
    /// active-probe count, and in `Open` with an elapsed reset timeout it
    /// transitions the breaker to `HalfOpen` and resets the counters. Every
    /// successful call must therefore be paired with
    /// [`record_outcome`](Self::record_outcome) (the [`call`](Self::call)
    /// methods do this via an internal drop guard).
    ///
    /// To merely observe the state, use [`circuit_state`](Self::circuit_state)
    /// or [`stats`](Self::stats) — calling this as a predicate leaks probe
    /// slots and can open the circuit it was asked to observe.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::CircuitOpen)` when the circuit is open
    /// or the half-open probe limit has been reached.
    pub fn try_acquire<E>(&self) -> Result<(), CallError<E>> {
        let mut transition: Option<(CircuitState, CircuitState)> = None;
        let mut inner = self.state.lock();
        let result = match inner.state {
            State::Closed => Ok(()),
            State::HalfOpen => {
                if inner.half_open_probes >= self.config.max_half_open_operations {
                    Err(CallError::CircuitOpen)
                } else {
                    inner.half_open_probes = inner.half_open_probes.saturating_add(1);
                    Ok(())
                }
            },
            State::Open { opened_at } => {
                let elapsed = self.instant_source.now().duration_since(opened_at);
                let timeout = self.effective_reset_timeout(inner.consecutive_opens);
                if elapsed >= timeout {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::HalfOpen;
                    inner.failures = 0;
                    inner.total = 0;
                    inner.slow_calls = 0;
                    inner.half_open_successes = 0;
                    inner.half_open_probes = 1; // this call is the first probe
                    self.atomic_state.store(STATE_HALF_OPEN, Ordering::Relaxed);
                    transition = Some((prev, CircuitState::HalfOpen));
                    Ok(())
                } else {
                    Err(CallError::CircuitOpen)
                }
            },
        };
        drop(inner);
        if let Some((from, to)) = transition {
            self.sink
                .record(ResilienceEvent::CircuitStateChanged { from, to });
            if let Some(ref cb) = self.on_state_change {
                cb(from, to);
            }
        }
        result
    }

    /// Whether the failure count has reached the configured threshold.
    const fn should_trip_on_failure(&self, inner: &InnerState) -> bool {
        inner.failures >= self.config.failure_threshold && inner.total >= self.config.min_operations
    }

    /// Whether the slow call rate has exceeded the configured threshold.
    fn slow_rate_trips(&self, inner: &InnerState) -> bool {
        if self.config.slow_call_threshold.is_none() {
            return false;
        }
        inner.total >= self.config.min_operations
            && rate_exceeds(
                inner.slow_calls,
                inner.total,
                self.config.slow_call_rate_threshold,
            )
    }

    /// Transition to `Open` from the current state, returning the transition pair.
    fn trip_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        let prev = to_circuit_state(inner.state);
        inner.state = State::Open {
            opened_at: self.instant_source.now(),
        };
        inner.half_open_probes = 0;
        inner.half_open_successes = 0;
        inner.consecutive_opens += 1;
        self.atomic_state.store(STATE_OPEN, Ordering::Relaxed);
        (prev, CircuitState::Open)
    }

    /// Trip to `Open` from `HalfOpen`, clearing the probe count first.
    /// Extracted to deduplicate the identical reset+trip pattern in `record_outcome`.
    fn trip_open_from_half_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        inner.half_open_probes = 0;
        inner.half_open_successes = 0;
        self.trip_open(inner)
    }

    /// Reset all counters and set state to `Closed`.
    const fn reset_counters(inner: &mut InnerState) {
        inner.state = State::Closed;
        inner.failures = 0;
        inner.total = 0;
        inner.slow_calls = 0;
        inner.half_open_probes = 0;
        inner.half_open_successes = 0;
        inner.consecutive_opens = 0;
    }

    /// Reset all counters and transition to `Closed` from the current state.
    fn close_from_half_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        let prev = to_circuit_state(inner.state);
        Self::reset_counters(inner);
        self.atomic_state.store(STATE_CLOSED, Ordering::Relaxed);
        (prev, CircuitState::Closed)
    }

    /// Record a successful half-open probe.
    fn record_half_open_success(
        &self,
        inner: &mut InnerState,
    ) -> Option<(CircuitState, CircuitState)> {
        inner.half_open_probes = inner.half_open_probes.saturating_sub(1);
        inner.half_open_successes = inner.half_open_successes.saturating_add(1);
        if inner.half_open_successes >= self.required_half_open_successes() {
            Some(self.close_from_half_open(inner))
        } else {
            None
        }
    }

    /// Record an operation outcome directly (useful when driving the CB from external code).
    ///
    /// In the Closed state, each success decrements the failure counter by one ("leaky bucket"
    /// forgiveness). This means that interleaved successes slowly erase past failures,
    /// preventing the breaker from tripping on intermittent errors.
    pub fn record_outcome(&self, outcome: Outcome) {
        let mut transition: Option<(CircuitState, CircuitState)> = None;
        let mut inner = self.state.lock();
        match outcome {
            Outcome::Cancelled => {
                // Never count cancellations as failures, but release the probe slot
                // so that half-open probes aren't permanently leaked on drop/cancel.
                inner.half_open_probes = inner.half_open_probes.saturating_sub(1);
            },
            Outcome::Success => {
                if inner.state == State::HalfOpen {
                    transition = self.record_half_open_success(&mut inner);
                } else {
                    inner.failures = inner.failures.saturating_sub(1);
                    inner.total = inner.total.saturating_add(1);
                }
            },
            Outcome::Failure | Outcome::Timeout => {
                if matches!(outcome, Outcome::Timeout) && !self.config.count_timeouts_as_failures {
                    // Don't count as failure, but still release the probe slot
                    // so half-open probes aren't permanently leaked.
                    inner.half_open_probes = inner.half_open_probes.saturating_sub(1);
                } else {
                    inner.failures = inner.failures.saturating_add(1);
                    inner.total = inner.total.saturating_add(1);

                    if inner.state == State::HalfOpen {
                        transition = Some(self.trip_open_from_half_open(&mut inner));
                    } else if self.should_trip_on_failure(&inner) {
                        transition = Some(self.trip_open(&mut inner));
                    }
                }
            },
            Outcome::SlowSuccess => {
                inner.slow_calls = inner.slow_calls.saturating_add(1);
                inner.total = inner.total.saturating_add(1);
                if inner.state == State::HalfOpen {
                    transition = self.record_half_open_success(&mut inner);
                } else {
                    inner.failures = inner.failures.saturating_sub(1);
                    if self.slow_rate_trips(&inner) {
                        transition = Some(self.trip_open(&mut inner));
                    }
                }
            },
            Outcome::SlowFailure => {
                inner.slow_calls = inner.slow_calls.saturating_add(1);
                inner.failures = inner.failures.saturating_add(1);
                inner.total = inner.total.saturating_add(1);
                if inner.state == State::HalfOpen {
                    transition = Some(self.trip_open_from_half_open(&mut inner));
                } else if self.should_trip_on_failure(&inner) || self.slow_rate_trips(&inner) {
                    transition = Some(self.trip_open(&mut inner));
                }
            },
        }
        drop(inner);
        if let Some((from, to)) = transition {
            self.sink
                .record(ResilienceEvent::CircuitStateChanged { from, to });
            if let Some(ref cb) = self.on_state_change {
                cb(from, to);
            }
        }
    }

    /// Returns the current circuit state (lock-free atomic read).
    pub fn circuit_state(&self) -> CircuitState {
        match self.atomic_state.load(Ordering::Relaxed) {
            STATE_OPEN => CircuitState::Open,
            STATE_HALF_OPEN => CircuitState::HalfOpen,
            _ => CircuitState::Closed,
        }
    }

    /// Returns a stats snapshot.
    pub fn stats(&self) -> CircuitBreakerStats {
        let inner = self.state.lock();
        let state = to_circuit_state(inner.state);
        let (failures, total, slow_calls) = (inner.failures, inner.total, inner.slow_calls);
        drop(inner);
        CircuitBreakerStats {
            state,
            failures,
            total,
            slow_calls,
        }
    }
}

/// RAII guard that records `Cancelled` on drop if the operation is abandoned.
///
/// Used by `call()` and the pipeline's CB step to ensure half-open probe slots
/// are released when the future is dropped (e.g. by `tokio::select!` or a timeout).
/// Call [`defuse()`](ProbeGuard::defuse) before recording the real outcome.
pub(crate) struct ProbeGuard<'a> {
    cb: &'a CircuitBreaker,
    defused: bool,
}

impl<'a> ProbeGuard<'a> {
    pub(crate) const fn new(cb: &'a CircuitBreaker) -> Self {
        Self { cb, defused: false }
    }

    /// Defuse the guard — prevents `Cancelled` from being recorded on drop.
    /// Must be called before `record_outcome` with the real outcome.
    pub(crate) const fn defuse(&mut self) {
        self.defused = true;
    }
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if !self.defused {
            self.cb.record_outcome(Outcome::Cancelled);
        }
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

/// Integer-only rate comparison: `count / total >= threshold` without f64 conversion.
///
/// Uses fixed-point scaling (`count * SCALE >= threshold_scaled * total`) to avoid
/// `cvtsi2sd` false-dependency stalls on Intel CPUs. Precision: 6 decimal places.
// Reason: casts are safe — u32 * 1_000_000 fits in u64, threshold in [0.0, 1.0].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn rate_exceeds(count: u32, total: u32, threshold: f64) -> bool {
    const SCALE: u64 = 1_000_000;
    let threshold_scaled = (threshold * SCALE as f64) as u64;
    u64::from(count) * SCALE >= threshold_scaled * u64::from(total)
}

const fn to_circuit_state(s: State) -> CircuitState {
    match s {
        State::Closed => CircuitState::Closed,
        State::Open { .. } => CircuitState::Open,
        State::HalfOpen => CircuitState::HalfOpen,
    }
}

#[cfg(test)]
#[path = "circuit_breaker_tests.rs"]
mod tests;
