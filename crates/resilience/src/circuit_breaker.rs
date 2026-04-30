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

#[cfg(not(loom))]
use std::sync::atomic::{AtomicU32, Ordering};
use std::{sync::Arc, time::Duration};

// Under loom, swap std atomics for loom-instrumented equivalents.
#[cfg(loom)]
use loom::sync::atomic::{AtomicU32, Ordering};
use parking_lot::Mutex;

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
    pub max_half_open_operations: u32,
    /// Minimum number of operations required before failures can trip the breaker. Default: 5.
    pub min_operations: u32,
    /// Whether timeouts count as failures **and toward `total` operations**.
    /// When `false`, timeouts are completely ignored by the circuit breaker —
    /// they do not count as failures, successes, or toward `min_operations`.
    /// Default: `true`.
    pub count_timeouts_as_failures: bool,
    /// Multiplier for reset timeout on consecutive opens. Default: 1.0 (no increase).
    pub break_duration_multiplier: f64,
    /// Maximum reset timeout cap when using dynamic break duration. Default: 5 minutes.
    pub max_break_duration: Duration,
    /// Duration threshold above which a successful call is considered "slow". `None` = disabled.
    pub slow_call_threshold: Option<Duration>,
    /// Slow call rate threshold (0.0--1.0). If slow calls / total >= this, CB trips. Default: 1.0.
    pub slow_call_rate_threshold: f64,
    /// Size of the count-based sliding window. 0 = use simple counters (default).
    pub sliding_window_size: u32,
    /// Failure rate threshold (0.0--1.0) used with sliding window. `None` = use
    /// `failure_threshold` count.
    pub failure_rate_threshold: Option<f64>,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            max_half_open_operations: 1,
            min_operations: 5,
            count_timeouts_as_failures: true,
            break_duration_multiplier: 1.0,
            max_break_duration: Duration::from_mins(5),
            slow_call_threshold: None,
            slow_call_rate_threshold: 1.0,
            sliding_window_size: 0,
            failure_rate_threshold: None,
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
        if self.min_operations == 0 {
            return Err(ConfigError::new("min_operations", "must be >= 1"));
        }
        if self.break_duration_multiplier < 1.0 {
            return Err(ConfigError::new(
                "break_duration_multiplier",
                "must be >= 1.0",
            ));
        }
        if !(0.0..=1.0).contains(&self.slow_call_rate_threshold) {
            return Err(ConfigError::new(
                "slow_call_rate_threshold",
                "must be between 0.0 and 1.0",
            ));
        }
        if self
            .failure_rate_threshold
            .is_some_and(|r| !(0.0..=1.0).contains(&r))
        {
            return Err(ConfigError::new(
                "failure_rate_threshold",
                "must be between 0.0 and 1.0",
            ));
        }
        Ok(())
    }
}

// ── Outcome (internal) ────────────────────────────────────────────────────────

/// The outcome of an operation, used to update circuit breaker state.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitBreakerStats {
    /// Current circuit state.
    pub state: CircuitState,
    /// Current failure count.
    pub failures: u32,
    /// Total operations in current window.
    pub total: u32,
    /// Number of slow calls in current window.
    pub slow_calls: u32,
}

type StateChangeCallback = Box<dyn Fn(CircuitState, CircuitState) + Send + Sync>;

/// Circuit breaker — protects downstream calls by rejecting requests when failure rate is high.
///
/// Shared state via `Arc<CircuitBreaker>`. Inject [`MockClock`](crate::clock::MockClock) and
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
    clock: Arc<dyn Clock>,
    sink: Arc<dyn MetricsSink>,
    state: Mutex<InnerState>,
    on_state_change: Option<StateChangeCallback>,
}

/// Sum a slice of 0/1 bytes into a u32.
///
/// On x86-64, uses SSE2 `psadbw` (sum-of-absolute-differences against zero)
/// to process 16 bytes per cycle — 16x faster than scalar for large windows.
/// Falls back to a 4-accumulator scalar loop on non-x86 targets.
///
/// Outlined (`inline(never)`) to prevent LLVM from duplicating the loop
/// body at every inlined `failure_count`/`slow_count` call site.
#[inline(never)]
fn byte_sum(slice: &[u8]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        byte_sum_sse2(slice)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        byte_sum_scalar(slice)
    }
}

/// SSE2 SIMD path: `psadbw` sums 16 bytes per iteration into two u64 lanes.
/// SSE2 is guaranteed on all x86-64 CPUs — no runtime feature check needed.
#[cfg(target_arch = "x86_64")]
#[allow(
    unsafe_code,
    clippy::cast_ptr_alignment,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn byte_sum_sse2(slice: &[u8]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_add_epi64, _mm_cvtsi128_si64, _mm_loadu_si128, _mm_sad_epu8,
        _mm_setzero_si128, _mm_unpackhi_epi64,
    };

    // SAFETY: SSE2 is guaranteed on x86-64. All pointer arithmetic is bounds-checked
    // via the chunk/remainder split. `_mm_loadu_si128` handles unaligned loads.
    unsafe {
        let zero = _mm_setzero_si128();
        let mut acc = _mm_setzero_si128();

        let chunks = slice.chunks_exact(16);
        let remainder = chunks.remainder();

        for chunk in chunks {
            let v = _mm_loadu_si128(chunk.as_ptr().cast::<__m128i>());
            acc = _mm_add_epi64(acc, _mm_sad_epu8(v, zero));
        }

        // Horizontal sum of 2 u64 lanes
        let hi = _mm_unpackhi_epi64(acc, acc);
        let total = _mm_add_epi64(acc, hi);
        let mut sum = _mm_cvtsi128_si64(total) as u32;

        for &b in remainder {
            sum += u32::from(b);
        }
        sum
    }
}

/// Scalar fallback: 4 independent accumulators to break loop-carried dependency.
#[cfg(not(target_arch = "x86_64"))]
fn byte_sum_scalar(slice: &[u8]) -> u32 {
    let (mut a, mut b, mut c, mut d) = (0u32, 0u32, 0u32, 0u32);
    let mut chunks = slice.chunks_exact(4);
    for chunk in &mut chunks {
        a += u32::from(chunk[0]);
        b += u32::from(chunk[1]);
        c += u32::from(chunk[2]);
        d += u32::from(chunk[3]);
    }
    let sum = a + b + c + d;
    sum + chunks
        .remainder()
        .iter()
        .copied()
        .map(u32::from)
        .sum::<u32>()
}

/// Fixed-size ring buffer of call outcomes for rate-based circuit breaking.
///
/// Stores failure and slow-call flags in separate byte arrays. The
/// `byte_sum` helper uses chunked iteration that LLVM auto-vectorizes
/// with `psadbw`/`vpsadbw` SIMD instructions at window sizes >= 32.
///
/// Capacity is rounded up to the next power of two so that the ring
/// pointer wraps via bitmask (`& mask`) instead of integer division.
///
/// Made `pub` so it can be benchmarked directly from `benches/sliding_window_cb.rs`.
#[doc(hidden)]
#[derive(Debug)]
pub struct OutcomeWindow {
    /// 1 = failure, 0 = success — one byte per slot, contiguous for SIMD.
    failure_ring: Box<[u8]>,
    /// 1 = slow call, 0 = normal — one byte per slot, contiguous for SIMD.
    slow_ring: Box<[u8]>,
    /// Bitmask for wrapping: always `capacity - 1` where capacity is a power of two.
    mask: usize,
    head: usize,
    len: usize,
}

impl OutcomeWindow {
    #[must_use]
    pub fn new(requested: usize) -> Self {
        let cap = requested.next_power_of_two().max(1);
        Self {
            failure_ring: vec![0u8; cap].into_boxed_slice(),
            slow_ring: vec![0u8; cap].into_boxed_slice(),
            mask: cap - 1,
            head: 0,
            len: 0,
        }
    }

    #[expect(
        unsafe_code,
        reason = "head is maintained via bitmask so get_unchecked_mut is in-bounds; see SAFETY comment"
    )]
    pub fn record(&mut self, is_failure: bool, is_slow: bool) {
        let h = self.head;
        debug_assert!(h <= self.mask, "head exceeds mask");
        // SAFETY: `head` is always `< capacity` because it is maintained via
        // `(head + 1) & mask` where `mask = capacity - 1` and `capacity` equals
        // both `failure_ring.len()` and `slow_ring.len()`.
        unsafe {
            *self.failure_ring.get_unchecked_mut(h) = u8::from(is_failure);
            *self.slow_ring.get_unchecked_mut(h) = u8::from(is_slow);
        }
        self.head = (h + 1) & self.mask;
        let cap = self.mask + 1;
        if self.len < cap {
            self.len += 1;
        }
    }

    // Reason: usize to u32 cast is safe for practical window sizes (< 2^32).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "usize to u32 cast is safe for practical window sizes (< 2^32)"
    )]
    pub const fn total(&self) -> u32 {
        self.len as u32
    }

    #[must_use]
    pub fn failure_count(&self) -> u32 {
        byte_sum(self.active_slice(&self.failure_ring))
    }

    #[must_use]
    pub fn slow_count(&self) -> u32 {
        byte_sum(self.active_slice(&self.slow_ring))
    }

    #[expect(
        unsafe_code,
        reason = "len <= ring.len() is maintained as invariant; see SAFETY comment"
    )]
    fn active_slice<'a>(&self, ring: &'a [u8]) -> &'a [u8] {
        let cap = self.mask + 1;
        if self.len < cap {
            debug_assert!(self.len <= ring.len(), "len exceeds ring capacity");
            // SAFETY: `len` is always `<= capacity` which equals `ring.len()`.
            // The `len < cap` guard above ensures `len < ring.len()`.
            unsafe { ring.get_unchecked(..self.len) }
        } else {
            ring
        }
    }

    #[cold]
    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.failure_ring.fill(0);
        self.slow_ring.fill(0);
    }
}

struct InnerState {
    state: State,
    failures: u32,
    total: u32,
    /// Number of active probe operations in `HalfOpen` state.
    half_open_probes: u32,
    /// Number of consecutive times the circuit has opened (for dynamic break duration).
    consecutive_opens: u32,
    /// Number of slow calls in the current window.
    slow_calls: u32,
    /// Sliding window (used when `config.sliding_window_size > 0`).
    window: Option<OutcomeWindow>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if configuration is invalid.
    pub fn new(config: CircuitBreakerConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let window_size = config.sliding_window_size;
        Ok(Self {
            config,
            atomic_state: AtomicU32::new(STATE_CLOSED),
            state: Mutex::new(InnerState {
                state: State::Closed,
                failures: 0,
                total: 0,
                half_open_probes: 0,
                consecutive_opens: 0,
                slow_calls: 0,
                window: if window_size > 0 {
                    Some(OutcomeWindow::new(window_size as usize))
                } else {
                    None
                },
            }),
            clock: Arc::new(SystemClock),
            sink: Arc::new(NoopSink),
            on_state_change: None,
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

    /// Manually force the circuit open, rejecting all calls until reset timeout or
    /// [`force_close`](Self::force_close).
    pub fn force_open(&self) {
        let mut inner = self.state.lock();
        let prev = to_circuit_state(inner.state);
        inner.state = State::Open {
            opened_at: self.clock.now(),
        };
        inner.half_open_probes = 0;
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
        if consecutive_opens <= 1 || self.config.break_duration_multiplier <= 1.0 {
            return self.config.reset_timeout;
        }
        let exponent = consecutive_opens - 1;
        let max_secs = self.config.max_break_duration.as_secs_f64();
        let multiplied = (self.config.reset_timeout.as_secs_f64()
            * self.config.break_duration_multiplier.powi(exponent as i32))
        .min(max_secs);
        Duration::from_secs_f64(multiplied)
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
        let start = self.clock.now();
        let result = f().await;
        let duration = self.clock.now().duration_since(start);
        let outcome = self.classify_outcome(result.is_ok(), duration);
        guard.defuse();
        self.record_outcome(outcome);
        result.map_err(CallError::Operation)
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
        let start = self.clock.now();
        let result = f().await;
        let duration = self.clock.now().duration_since(start);

        let outcome = match &result {
            Ok(_) => self.classify_outcome(true, duration),
            Err(e) => {
                let class = classifier.classify(e);
                if class.counts_as_failure() {
                    self.classify_outcome(false, duration)
                } else {
                    class.into()
                }
            },
        };

        guard.defuse();
        self.record_outcome(outcome);
        result.map_err(CallError::Operation)
    }

    /// Check if the circuit allows execution.
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
                let elapsed = self.clock.now().duration_since(opened_at);
                let timeout = self.effective_reset_timeout(inner.consecutive_opens);
                if elapsed >= timeout {
                    let prev = to_circuit_state(inner.state);
                    inner.state = State::HalfOpen;
                    inner.failures = 0;
                    inner.total = 0;
                    inner.slow_calls = 0;
                    inner.half_open_probes = 1; // this call is the first probe
                    if let Some(ref mut window) = inner.window {
                        window.reset();
                    }
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

    /// Whether the failure rate/count has exceeded the configured threshold.
    fn should_trip_on_failure(&self, inner: &InnerState) -> bool {
        if let (Some(window), Some(rate_threshold)) =
            (&inner.window, self.config.failure_rate_threshold)
        {
            window.total() >= self.config.min_operations
                && rate_exceeds(window.failure_count(), window.total(), rate_threshold)
        } else {
            inner.failures >= self.config.failure_threshold
                && inner.total >= self.config.min_operations
        }
    }

    /// Whether the slow call rate has exceeded the configured threshold.
    fn slow_rate_trips(&self, inner: &InnerState) -> bool {
        if self.config.slow_call_threshold.is_none() {
            return false;
        }
        let (total, slow) = inner
            .window
            .as_ref()
            .map_or((inner.total, inner.slow_calls), |window| {
                (window.total(), window.slow_count())
            });
        total >= self.config.min_operations
            && rate_exceeds(slow, total, self.config.slow_call_rate_threshold)
    }

    /// Transition to `Open` from the current state, returning the transition pair.
    fn trip_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        let prev = to_circuit_state(inner.state);
        inner.state = State::Open {
            opened_at: self.clock.now(),
        };
        inner.consecutive_opens += 1;
        self.atomic_state.store(STATE_OPEN, Ordering::Relaxed);
        (prev, CircuitState::Open)
    }

    /// Trip to `Open` from `HalfOpen`, clearing the probe count first.
    /// Extracted to deduplicate the identical reset+trip pattern in `record_outcome`.
    fn trip_open_from_half_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        inner.half_open_probes = 0;
        self.trip_open(inner)
    }

    /// Reset all counters and set state to `Closed`.
    fn reset_counters(inner: &mut InnerState) {
        inner.state = State::Closed;
        inner.failures = 0;
        inner.total = 0;
        inner.slow_calls = 0;
        inner.half_open_probes = 0;
        inner.consecutive_opens = 0;
        if let Some(ref mut window) = inner.window {
            window.reset();
        }
    }

    /// Reset all counters and transition to `Closed` from the current state.
    fn close_from_half_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
        let prev = to_circuit_state(inner.state);
        Self::reset_counters(inner);
        self.atomic_state.store(STATE_CLOSED, Ordering::Relaxed);
        (prev, CircuitState::Closed)
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
                    transition = Some(self.close_from_half_open(&mut inner));
                } else {
                    inner.failures = inner.failures.saturating_sub(1);
                    inner.total = inner.total.saturating_add(1);
                    if let Some(ref mut window) = inner.window {
                        window.record(false, false);
                    }
                }
            },
            Outcome::Failure | Outcome::Timeout => {
                if matches!(outcome, Outcome::Timeout) && !self.config.count_timeouts_as_failures {
                    // Don't count as failure, but still release the probe slot
                    // so half-open probes aren't permanently leaked.
                    inner.half_open_probes = inner.half_open_probes.saturating_sub(1);
                    return;
                }
                inner.failures = inner.failures.saturating_add(1);
                inner.total = inner.total.saturating_add(1);
                if let Some(ref mut window) = inner.window {
                    window.record(true, false);
                }

                if inner.state == State::HalfOpen {
                    transition = Some(self.trip_open_from_half_open(&mut inner));
                } else if self.should_trip_on_failure(&inner) {
                    transition = Some(self.trip_open(&mut inner));
                }
            },
            Outcome::SlowSuccess => {
                inner.slow_calls = inner.slow_calls.saturating_add(1);
                inner.total = inner.total.saturating_add(1);
                if let Some(ref mut window) = inner.window {
                    window.record(false, true);
                }
                if inner.state == State::HalfOpen {
                    transition = Some(self.close_from_half_open(&mut inner));
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
                if let Some(ref mut window) = inner.window {
                    window.record(true, true);
                }
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
        let (failures, total, slow_calls) = inner.window.as_ref().map_or_else(
            || (inner.failures, inner.total, inner.slow_calls),
            |window| (window.failure_count(), window.total(), window.slow_count()),
        );
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
#[allow(
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
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{CallError, RecordingSink, sink::CircuitState as CS};

    fn default_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_millis(100),
            max_half_open_operations: 1,
            min_operations: 1,
            count_timeouts_as_failures: true,
            break_duration_multiplier: 1.0,
            max_break_duration: Duration::from_mins(5),
            slow_call_threshold: None,
            slow_call_rate_threshold: 1.0,
            sliding_window_size: 0,
            failure_rate_threshold: None,
        }
    }

    #[tokio::test]
    async fn opens_after_failure_threshold() {
        let cb = CircuitBreaker::new(default_config()).unwrap();
        for _ in 0..3 {
            let _ = cb
                .call::<(), _, _>(|| Box::pin(async { Err("fail") }))
                .await;
        }
        let err: CallError<&str> = cb
            .call::<(), _, _>(|| Box::pin(async { Ok(()) }))
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
        let result = cb.call::<u32, &str, _>(|| Box::pin(async { Ok(42) })).await;
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
                .call::<(), &str, _>(|| Box::pin(async { Err("fail") }))
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
            max_half_open_operations: 1,
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
        assert!(cb.try_acquire::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Second probe should be rejected (max_probes=1 reached)
        assert!(matches!(
            cb.try_acquire::<&str>(),
            Err(CallError::CircuitOpen)
        ));

        // After the probe succeeds, breaker closes and allows new calls
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);
        assert!(cb.try_acquire::<&str>().is_ok());
    }

    #[tokio::test]
    async fn half_open_failure_reopens_breaker() {
        let sink = RecordingSink::new();
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            max_half_open_operations: 1,
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
        assert!(cb.try_acquire::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Probe fails → back to Open
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);
    }

    #[tokio::test]
    async fn dropped_call_releases_probe_slot() {
        let cb = Arc::new(
            CircuitBreaker::new(CircuitBreakerConfig {
                max_half_open_operations: 1,
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
            _ = cb2.call(|| Box::pin(async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<(), &str>(())
            })) => unreachable!(),
            () = tokio::time::sleep(Duration::from_millis(5)) => {
                // Future dropped — probe guard should release the slot
            }
        }

        // The probe slot should be freed. Wait for reset again and try a new probe.
        // Since the cancelled probe decremented half_open_probes, the next
        // Open→HalfOpen transition should work.
        tokio::time::sleep(Duration::from_millis(110)).await;

        // This must succeed — the probe slot was properly released
        assert!(cb.try_acquire::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Complete the probe successfully
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);
    }

    #[tokio::test]
    async fn force_open_rejects_calls() {
        let cb = CircuitBreaker::new(default_config()).unwrap();
        cb.force_open();
        assert_eq!(cb.circuit_state(), CS::Open);
        let err: CallError<&str> = cb
            .call::<(), _, _>(|| Box::pin(async { Ok(()) }))
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::CircuitOpen));
    }

    #[tokio::test]
    async fn force_close_resets_circuit() {
        let cb = CircuitBreaker::new(default_config()).unwrap();
        for _ in 0..3 {
            cb.record_outcome(Outcome::Failure);
        }
        assert_eq!(cb.circuit_state(), CS::Open);
        cb.force_close();
        assert_eq!(cb.circuit_state(), CS::Closed);
        let result = cb.call::<u32, &str, _>(|| Box::pin(async { Ok(42) })).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn on_state_change_fires_on_open() {
        let transitions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let t = transitions.clone();

        let cb = CircuitBreaker::new(default_config())
            .unwrap()
            .on_state_change(move |from, to| {
                t.lock().unwrap().push((from, to));
            });

        for _ in 0..3 {
            let _ = cb
                .call::<(), &str, _>(|| Box::pin(async { Err("fail") }))
                .await;
        }

        let t = transitions.lock().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0], (CS::Closed, CS::Open));
        drop(t);
    }

    #[tokio::test]
    async fn dynamic_break_duration_increases_on_repeated_opens() {
        use crate::clock::MockClock;
        let clock = Arc::new(MockClock::new());
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            reset_timeout: Duration::from_millis(100),
            max_half_open_operations: 1,
            min_operations: 1,
            count_timeouts_as_failures: true,
            break_duration_multiplier: 2.0,
            max_break_duration: Duration::from_secs(10),
            slow_call_threshold: None,
            slow_call_rate_threshold: 1.0,
            sliding_window_size: 0,
            failure_rate_threshold: None,
        })
        .unwrap()
        .with_clock(Arc::clone(&clock) as Arc<dyn Clock>);

        // First trip
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);

        // Wait 110ms (> first reset_timeout of 100ms)
        clock.advance(Duration::from_millis(110));
        assert!(cb.try_acquire::<&str>().is_ok());
        assert_eq!(cb.circuit_state(), CS::HalfOpen);

        // Fail again → consecutive_opens = 2, effective timeout = 200ms
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);

        // Wait 110ms — NOT enough (need 200ms)
        clock.advance(Duration::from_millis(110));
        assert!(matches!(
            cb.try_acquire::<&str>(),
            Err(CallError::CircuitOpen)
        ));

        // Wait 100ms more (total 220ms > 200ms)
        clock.advance(Duration::from_millis(100));
        assert!(cb.try_acquire::<&str>().is_ok());
    }

    #[tokio::test]
    async fn slow_calls_trip_breaker() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 100,
            slow_call_threshold: Some(Duration::from_millis(10)),
            slow_call_rate_threshold: 0.5,
            min_operations: 3,
            ..default_config()
        })
        .unwrap();

        // 3 slow successes -> 100% slow rate > 50% threshold
        cb.record_outcome(Outcome::SlowSuccess);
        cb.record_outcome(Outcome::SlowSuccess);
        cb.record_outcome(Outcome::SlowSuccess);
        assert_eq!(cb.circuit_state(), CS::Open);
    }

    #[test]
    fn classify_outcome_detects_slow_calls() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            slow_call_threshold: Some(Duration::from_millis(100)),
            ..CircuitBreakerConfig::default()
        })
        .unwrap();

        assert!(matches!(
            cb.classify_outcome(true, Duration::from_millis(50)),
            Outcome::Success
        ));
        assert!(matches!(
            cb.classify_outcome(true, Duration::from_millis(150)),
            Outcome::SlowSuccess
        ));
        assert!(matches!(
            cb.classify_outcome(false, Duration::from_millis(150)),
            Outcome::SlowFailure
        ));
        assert!(matches!(
            cb.classify_outcome(false, Duration::from_millis(50)),
            Outcome::Failure
        ));
    }

    #[tokio::test]
    async fn slow_calls_below_threshold_dont_trip() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 100,
            slow_call_threshold: Some(Duration::from_millis(10)),
            slow_call_rate_threshold: 0.5,
            min_operations: 4,
            ..default_config()
        })
        .unwrap();

        // 1 slow + 3 normal = 25% < 50%
        cb.record_outcome(Outcome::SlowSuccess);
        cb.record_outcome(Outcome::Success);
        cb.record_outcome(Outcome::Success);
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);
    }

    #[tokio::test]
    async fn sliding_window_forgets_old_outcomes() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            sliding_window_size: 4,
            failure_rate_threshold: Some(0.6),
            min_operations: 3,
            ..default_config()
        })
        .unwrap();

        // 3 failures -> 3/3 = 100% > 60% -> trips
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);

        cb.force_close();

        // 4 calls: 1 failure, 3 successes -> 1/4 = 25% < 60% -> stays closed
        cb.record_outcome(Outcome::Success);
        cb.record_outcome(Outcome::Success);
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Success);
        assert_eq!(cb.circuit_state(), CS::Closed);

        // One more failure pushes oldest (success) out:
        // window = [S, F, S, F] -> 2/4 = 50% < 60% -> stays closed
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Closed);

        // Another failure pushes a success out:
        // window = [F, S, F, F] -> 3/4 = 75% >= 60% -> trips
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);
    }

    #[test]
    fn sliding_window_without_rate_threshold_uses_count() {
        // sliding_window_size > 0 but failure_rate_threshold is None -> count-based
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            sliding_window_size: 10,
            failure_rate_threshold: None,
            min_operations: 1,
            ..default_config()
        })
        .unwrap();

        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Closed);
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CS::Open);
    }

    #[test]
    fn invalid_failure_rate_threshold_rejected() {
        let result = CircuitBreaker::new(CircuitBreakerConfig {
            failure_rate_threshold: Some(1.5),
            ..default_config()
        });
        assert!(result.is_err());
    }

    #[test]
    fn sliding_window_stats_reflect_window() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 100,
            sliding_window_size: 4,
            failure_rate_threshold: Some(0.9),
            min_operations: 1,
            ..default_config()
        })
        .unwrap();

        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Success);
        cb.record_outcome(Outcome::Success);

        let stats = cb.stats();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.failures, 2);

        // Push oldest failure out of window
        cb.record_outcome(Outcome::Success);
        let stats = cb.stats();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.failures, 1);
    }

    // ── C1: min_operations validation ────────────────────────────────────

    #[test]
    fn rejects_min_operations_zero() {
        let config = CircuitBreakerConfig {
            min_operations: 0,
            ..default_config()
        };
        let err = CircuitBreaker::new(config).unwrap_err();
        assert_eq!(err.field, "min_operations");
    }
}

// ---------------------------------------------------------------------------
// Loom concurrency tests — exhaustively verify the atomic_state mirror pattern.
//
// The circuit breaker uses a `Relaxed` atomic store (inside the parking_lot mutex)
// paired with a `Relaxed` atomic load (outside the mutex) for `circuit_state()`.
// Loom checks that no invalid interleaving produces an undefined state value.
//
// Run with:
//   RUSTFLAGS="--cfg loom" cargo test -p nebula-resilience -- loom
//
// Note: parking_lot::Mutex is NOT loom-instrumented. These tests focus exclusively
// on the AtomicU32 mirror — the mutex correctness is guaranteed by parking_lot.
// ---------------------------------------------------------------------------
#[cfg(all(test, loom))]
mod loom_tests {
    use loom::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        thread,
    };

    use super::{STATE_CLOSED, STATE_HALF_OPEN, STATE_OPEN};

    /// Two threads race: writer transitions Closed→Open→HalfOpen→Closed
    /// while reader polls `circuit_state()`. Loom verifies that the reader
    /// only ever sees valid state values (0, 1, or 2) — never torn or
    /// intermediate values.
    #[test]
    fn atomic_state_mirror_only_sees_valid_states() {
        loom::model(|| {
            let state = Arc::new(AtomicU32::new(STATE_CLOSED));

            let writer_state = Arc::clone(&state);
            let writer = thread::spawn(move || {
                writer_state.store(STATE_OPEN, Ordering::Relaxed);
                writer_state.store(STATE_HALF_OPEN, Ordering::Relaxed);
                writer_state.store(STATE_CLOSED, Ordering::Relaxed);
            });

            let val = state.load(Ordering::Relaxed);
            assert!(
                val == STATE_CLOSED || val == STATE_OPEN || val == STATE_HALF_OPEN,
                "invalid state value observed: {val}"
            );

            writer.join().unwrap();
            assert_eq!(state.load(Ordering::Relaxed), STATE_CLOSED);
        });
    }

    /// Multiple concurrent readers + one writer.
    #[test]
    fn concurrent_readers_see_valid_states() {
        loom::model(|| {
            let state = Arc::new(AtomicU32::new(STATE_CLOSED));

            let w = Arc::clone(&state);
            let writer = thread::spawn(move || {
                w.store(STATE_OPEN, Ordering::Relaxed);
            });

            let r1 = Arc::clone(&state);
            let reader1 = thread::spawn(move || {
                let v = r1.load(Ordering::Relaxed);
                assert!(v == STATE_CLOSED || v == STATE_OPEN);
            });

            let v = state.load(Ordering::Relaxed);
            assert!(v == STATE_CLOSED || v == STATE_OPEN);

            writer.join().unwrap();
            reader1.join().unwrap();
        });
    }

    /// Verify no spurious values from valid state machine transitions.
    #[test]
    fn no_spurious_state_values() {
        loom::model(|| {
            let state = Arc::new(AtomicU32::new(STATE_CLOSED));

            let w = Arc::clone(&state);
            let t = thread::spawn(move || {
                w.store(STATE_OPEN, Ordering::Relaxed);
                w.store(STATE_CLOSED, Ordering::Relaxed);
            });

            let v = state.load(Ordering::Relaxed);
            assert!(v <= STATE_HALF_OPEN, "saw value {v} > 2");

            t.join().unwrap();
        });
    }
}
