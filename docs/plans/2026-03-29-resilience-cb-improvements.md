# Circuit Breaker & Resilience Improvements

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Upgrade circuit breaker with slow call detection, manual control, callbacks, dynamic break duration, and sliding window — plus add retry_after to RateLimited.

**Architecture:** Tasks 1–4 are independent (can parallel). Task 5 (slow call) adds new Outcome variants. Task 6 (sliding window) restructures InnerState — do last. All in `crates/resilience/src/`.

**Tech Stack:** Rust 1.93, tokio, parking_lot, existing `CircuitBreaker` / `CallError<E>` / `MetricsSink` types.

---

## Task 1: RateLimited with retry_after hint

**Files:**
- Modify: `crates/resilience/src/types.rs`
- Modify: `crates/resilience/src/rate_limiter.rs`
- Modify: `crates/resilience/src/pipeline.rs`
- Modify: `crates/resilience/src/fallback.rs`

**Goal:** Change `CallError::RateLimited` from unit variant to `RateLimited { retry_after: Option<Duration> }`.

### Step 1: Update CallError enum in types.rs

Change:
```rust
/// Rate limit exceeded.
RateLimited,
```
To:
```rust
/// Rate limit exceeded.
RateLimited {
    /// Optional hint for when to retry. `None` means unknown.
    retry_after: Option<Duration>,
},
```

Update `Display` impl:
```rust
Self::RateLimited { retry_after: Some(d) } => write!(f, "rate limit exceeded (retry after {d:?})"),
Self::RateLimited { retry_after: None } => write!(f, "rate limit exceeded"),
```

Update `is_retriable` — matches `Self::RateLimited { .. }`.

Update `map_operation` — `Self::RateLimited { retry_after } => CallError::RateLimited { retry_after }`.

Update `kind()` — `Self::RateLimited { .. } => CallErrorKind::RateLimited`.

Add helper:
```rust
/// Returns the retry-after hint for `RateLimited` errors, if available.
#[must_use]
pub const fn retry_after(&self) -> Option<Duration> {
    match self {
        Self::RateLimited { retry_after } => *retry_after,
        _ => None,
    }
}
```

### Step 2: Fix all `CallError::RateLimited` callsites

**In `rate_limiter.rs`** — replace all `Err(CallError::RateLimited)` with `Err(CallError::RateLimited { retry_after: None })`. About 13 occurrences.

**In `pipeline.rs`** — replace `CallError::RateLimited` (match arms) with `CallError::RateLimited { .. }`, and creation sites with `CallError::RateLimited { retry_after: None }`.

**In `fallback.rs`** — replace match arm with `CallError::RateLimited { .. }`.

### Step 3: Update tests

Update test in types.rs `rate_limited_is_retriable`:
```rust
let e: CallError<MyErr> = CallError::RateLimited { retry_after: None };
```

Add new test:
```rust
#[test]
fn rate_limited_retry_after_accessor() {
    let e: CallError<MyErr> = CallError::RateLimited {
        retry_after: Some(Duration::from_secs(5)),
    };
    assert_eq!(e.retry_after(), Some(Duration::from_secs(5)));
    assert!(e.is_retriable());

    let e2: CallError<MyErr> = CallError::RateLimited { retry_after: None };
    assert_eq!(e2.retry_after(), None);
}
```

### Step 4: Verify

```bash
rtk cargo fmt -p nebula-resilience
rtk cargo clippy -p nebula-resilience -- -D warnings
rtk cargo nextest run -p nebula-resilience
```

---

## Task 2: Manual Circuit Control

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs`

**Goal:** Add `force_open()` and `force_close()` methods for operational control.

### Step 1: Add test

```rust
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
    // Trip it
    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }
    assert_eq!(cb.circuit_state(), CS::Open);

    cb.force_close();
    assert_eq!(cb.circuit_state(), CS::Closed);

    // Should accept calls
    let result = cb.call::<u32, &str, _>(|| Box::pin(async { Ok(42) })).await;
    assert_eq!(result.unwrap(), 42);
}
```

### Step 2: Implement

Add to `impl CircuitBreaker`:

```rust
/// Manually open the circuit, rejecting all calls until [`force_close`] or reset timeout.
pub fn force_open(&self) {
    let mut inner = self.state.lock();
    let prev = to_circuit_state(inner.state);
    inner.state = State::Open { opened_at: self.clock.now() };
    inner.half_open_probes = 0;
    if prev != CircuitState::Open {
        self.sink.record(ResilienceEvent::CircuitStateChanged {
            from: prev,
            to: CircuitState::Open,
        });
    }
}

/// Manually close the circuit, resetting all counters.
pub fn force_close(&self) {
    let mut inner = self.state.lock();
    let prev = to_circuit_state(inner.state);
    inner.state = State::Closed;
    inner.failures = 0;
    inner.total = 0;
    inner.half_open_probes = 0;
    if prev != CircuitState::Closed {
        self.sink.record(ResilienceEvent::CircuitStateChanged {
            from: prev,
            to: CircuitState::Closed,
        });
    }
}
```

### Step 3: Verify

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(force_)'
rtk cargo clippy -p nebula-resilience -- -D warnings
```

---

## Task 3: State Transition Callbacks

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs`

**Goal:** Add `on_state_change` callback, called on every state transition (alongside MetricsSink).

### Step 1: Add test

```rust
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
        let _ = cb.call::<(), &str, _>(|| Box::pin(async { Err("fail") })).await;
    }

    let t = transitions.lock().unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0], (CS::Closed, CS::Open));
}
```

### Step 2: Implement

Add type alias and field:
```rust
type StateChangeCallback = Box<dyn Fn(CircuitState, CircuitState) + Send + Sync>;
```

Add to `CircuitBreaker`:
```rust
on_state_change: Option<StateChangeCallback>,
```

Initialize to `None` in `new()`.

Builder method:
```rust
/// Register a callback for state transitions.
#[must_use]
pub fn on_state_change<F>(mut self, f: F) -> Self
where
    F: Fn(CircuitState, CircuitState) + Send + Sync + 'static,
{
    self.on_state_change = Some(Box::new(f));
    self
}
```

In `record_outcome` and `can_execute`, after each `self.sink.record(ResilienceEvent::CircuitStateChanged { from, to })`, add:
```rust
if let Some(ref cb) = self.on_state_change {
    cb(from, to);
}
```

Note: `self.on_state_change` is accessed while `inner` lock is held. The callback should NOT be called under the lock — call it after dropping the lock. Restructure to collect `(from, to)` under lock, then call callback after `drop(inner)`.

### Step 3: Verify

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(on_state_change)'
rtk cargo clippy -p nebula-resilience -- -D warnings
```

---

## Task 4: Dynamic Break Duration

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs`

**Goal:** Reset timeout increases exponentially with consecutive opens. Configurable multiplier and max.

### Step 1: Add config fields

In `CircuitBreakerConfig`:
```rust
/// Multiplier for reset timeout on consecutive opens. Default: 1.0 (no increase).
pub break_duration_multiplier: f64,
/// Maximum reset timeout cap. Default: 5 minutes.
pub max_break_duration: Duration,
```

Default: `break_duration_multiplier: 1.0`, `max_break_duration: Duration::from_secs(300)`.

Validate: multiplier must be >= 1.0.

### Step 2: Add tracking to InnerState

```rust
/// Number of consecutive times the CB has opened without a successful close.
consecutive_opens: u32,
```

Initialize to 0.

### Step 3: Compute dynamic duration

In `can_execute` where we check `elapsed >= self.config.reset_timeout`:

```rust
let effective_timeout = self.effective_reset_timeout(inner.consecutive_opens);
if elapsed >= effective_timeout { ... }
```

Helper method:
```rust
fn effective_reset_timeout(&self, consecutive_opens: u32) -> Duration {
    if consecutive_opens == 0 || self.config.break_duration_multiplier <= 1.0 {
        return self.config.reset_timeout;
    }
    let multiplied = self.config.reset_timeout.as_secs_f64()
        * self.config.break_duration_multiplier.powi(consecutive_opens as i32);
    Duration::from_secs_f64(multiplied).min(self.config.max_break_duration)
}
```

### Step 4: Increment/reset consecutive_opens

- In `record_outcome` when transitioning to Open: `inner.consecutive_opens += 1;`
- In `record_outcome` when transitioning to Closed from HalfOpen (success): `inner.consecutive_opens = 0;`
- In `force_close`: `inner.consecutive_opens = 0;`

### Step 5: Add tests

```rust
#[tokio::test]
async fn dynamic_break_duration_increases_on_repeated_opens() {
    let clock = Arc::new(crate::clock::MockClock::new());
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 2,
        reset_timeout: Duration::from_millis(100),
        half_open_max_ops: 1,
        min_operations: 1,
        count_timeouts_as_failures: true,
        break_duration_multiplier: 2.0,
        max_break_duration: Duration::from_secs(10),
    })
    .unwrap()
    .with_clock(Arc::clone(&clock) as Arc<dyn crate::clock::Clock>);

    // First trip
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);

    // Wait 100ms (first reset_timeout) → should transition to HalfOpen
    clock.advance(Duration::from_millis(110));
    assert!(cb.can_execute::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    // Fail again → Open (consecutive_opens = 2)
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);

    // Wait 100ms again — should NOT be enough (need 200ms due to 2x multiplier)
    clock.advance(Duration::from_millis(110));
    assert!(matches!(cb.can_execute::<&str>(), Err(CallError::CircuitOpen)));

    // Wait another 100ms (total 220ms > 200ms) → should transition
    clock.advance(Duration::from_millis(100));
    assert!(cb.can_execute::<&str>().is_ok());
}
```

### Step 6: Verify

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(dynamic_break)'
rtk cargo clippy -p nebula-resilience -- -D warnings
```

---

## Task 5: Slow Call Detection

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs`

**Goal:** Track calls exceeding a duration threshold. If slow call rate exceeds threshold, trip the CB.

### Step 1: Add config fields

```rust
/// Duration threshold above which a successful call is considered "slow". `None` disables.
pub slow_call_threshold: Option<Duration>,
/// Slow call rate threshold (0.0–1.0). If the ratio of slow calls exceeds this, CB trips. Default: 1.0 (disabled).
pub slow_call_rate_threshold: f64,
```

Default: `slow_call_threshold: None`, `slow_call_rate_threshold: 1.0`.

Validate: `slow_call_rate_threshold` must be in 0.0..=1.0.

### Step 2: Add Outcome variants

```rust
pub enum Outcome {
    Success,
    Failure,
    Timeout,
    Cancelled,
    /// Operation succeeded but exceeded the slow call threshold.
    SlowSuccess,
    /// Operation failed and exceeded the slow call threshold.
    SlowFailure,
}
```

### Step 3: Add classify_outcome helper

```rust
/// Classify an operation result with timing information.
///
/// If a `slow_call_threshold` is configured and `duration` exceeds it,
/// the outcome is classified as `SlowSuccess`/`SlowFailure` instead of `Success`/`Failure`.
pub fn classify_outcome(&self, success: bool, duration: Duration) -> Outcome {
    let is_slow = self.config.slow_call_threshold
        .is_some_and(|threshold| duration >= threshold);
    match (success, is_slow) {
        (true, false) => Outcome::Success,
        (true, true) => Outcome::SlowSuccess,
        (false, false) => Outcome::Failure,
        (false, true) => Outcome::SlowFailure,
    }
}
```

### Step 4: Track slow calls in InnerState

Add:
```rust
slow_calls: u32,
```

Initialize to 0.

In `record_outcome`:
- `SlowSuccess`: count as success (leaky bucket) + increment `slow_calls`, increment `total`
- `SlowFailure`: count as failure + increment `slow_calls`, increment `total`

After recording, check BOTH thresholds:
```rust
let should_trip = (inner.failures >= self.config.failure_threshold
    && inner.total >= self.config.min_operations)
    || (inner.total >= self.config.min_operations
        && self.config.slow_call_threshold.is_some()
        && inner.slow_calls as f64 / inner.total as f64 >= self.config.slow_call_rate_threshold);
```

### Step 5: Update call() to measure duration

```rust
pub async fn call<T, E, Fut>(&self, f: impl FnOnce() -> Fut) -> Result<T, CallError<E>> {
    self.can_execute()?;
    let guard = ProbeGuard(self);
    let start = std::time::Instant::now();
    let result = f().await;
    let duration = start.elapsed();
    let outcome = self.classify_outcome(result.is_ok(), duration);
    std::mem::forget(guard);
    self.record_outcome(outcome);
    result.map_err(CallError::Operation)
}
```

### Step 6: Reset slow_calls on state transitions

In transitions to Closed or HalfOpen: `inner.slow_calls = 0;`

### Step 7: Add tests

```rust
#[tokio::test]
async fn slow_calls_trip_breaker() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 100, // high — won't trip from failures
        reset_timeout: Duration::from_millis(100),
        half_open_max_ops: 1,
        min_operations: 3,
        count_timeouts_as_failures: true,
        slow_call_threshold: Some(Duration::from_millis(10)),
        slow_call_rate_threshold: 0.5,
        ..Default::default()
    })
    .unwrap();

    // 3 slow successes → 100% slow rate > 50% threshold → trip
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
}
```

### Step 8: Add slow call count to CircuitBreakerStats

```rust
pub struct CircuitBreakerStats {
    pub state: CircuitState,
    pub failures: u32,
    pub total: u32,
    pub slow_calls: u32,
}
```

### Step 9: Verify

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(slow_call)'
rtk cargo clippy -p nebula-resilience -- -D warnings
```

---

## Task 6: Count-Based Sliding Window

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs`

**Goal:** Replace simple u32 counters with a ring buffer of recent outcomes for more accurate failure/slow-call rate calculation.

### Step 1: Design the window

```rust
/// Fixed-size ring buffer of call outcomes for rate calculation.
struct SlidingWindow {
    buffer: Vec<OutcomeEntry>,
    head: usize,
    count: usize,
}

#[derive(Clone, Copy, Default)]
struct OutcomeEntry {
    is_failure: bool,
    is_slow: bool,
}
```

Methods:
```rust
impl SlidingWindow {
    fn new(size: usize) -> Self { ... }
    fn record(&mut self, entry: OutcomeEntry) { ... } // push, overwrite oldest
    fn failure_count(&self) -> u32 { ... }
    fn slow_count(&self) -> u32 { ... }
    fn total(&self) -> u32 { self.count as u32 }
}
```

### Step 2: Add config field

```rust
/// Size of the sliding window for rate calculation. Default: 0 (use simple counters).
pub sliding_window_size: u32,
```

Default: 0. Validate: if > 0, must be >= `min_operations`.

### Step 3: Replace InnerState counters

Change `InnerState`:
```rust
struct InnerState {
    state: State,
    /// Simple failure counter (used when sliding_window_size == 0).
    failures: u32,
    /// Simple total counter (used when sliding_window_size == 0).
    total: u32,
    /// Simple slow call counter.
    slow_calls: u32,
    /// Sliding window (used when sliding_window_size > 0).
    window: Option<SlidingWindow>,
    half_open_probes: u32,
    consecutive_opens: u32,
}
```

### Step 4: Update record_outcome

When recording, if `window.is_some()`, push to window and read rates from it. If `window.is_none()`, use simple counters (backward compatible).

```rust
// After determining is_failure and is_slow:
if let Some(ref mut window) = inner.window {
    window.record(OutcomeEntry { is_failure, is_slow });
    let should_trip = window.total() >= self.config.min_operations
        && (window.failure_count() as f64 / window.total() as f64
            >= self.config.failure_threshold as f64 / 100.0  // treat as rate
            || (self.config.slow_call_threshold.is_some()
                && window.slow_count() as f64 / window.total() as f64
                    >= self.config.slow_call_rate_threshold));
} else {
    // existing counter logic
}
```

Wait — this changes the semantics of `failure_threshold`. When using sliding window, `failure_threshold` would need to be a percentage (0-100) rather than an absolute count. This is a bigger design question.

**Simpler approach:** Add a separate `failure_rate_threshold: Option<f64>` field. When `Some` AND `sliding_window_size > 0`, use rate-based tripping. When `None`, use the existing counter-based logic regardless of window.

```rust
/// Failure rate threshold (0.0–1.0). Used with sliding window. `None` = use failure_threshold count.
pub failure_rate_threshold: Option<f64>,
```

### Step 5: Update stats

```rust
impl CircuitBreaker {
    pub fn stats(&self) -> CircuitBreakerStats {
        let inner = self.state.lock();
        if let Some(ref window) = inner.window {
            CircuitBreakerStats {
                state: to_circuit_state(inner.state),
                failures: window.failure_count(),
                total: window.total(),
                slow_calls: window.slow_count(),
            }
        } else {
            CircuitBreakerStats {
                state: to_circuit_state(inner.state),
                failures: inner.failures,
                total: inner.total,
                slow_calls: inner.slow_calls,
            }
        }
    }
}
```

### Step 6: Add tests

```rust
#[tokio::test]
async fn sliding_window_forgets_old_outcomes() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 3,
        sliding_window_size: 5,
        failure_rate_threshold: Some(0.6),
        min_operations: 3,
        reset_timeout: Duration::from_millis(100),
        ..Default::default()
    })
    .unwrap();

    // 3 failures → 3/3 = 100% > 60% → trips
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);

    cb.force_close();

    // Now: 5 calls, 2 failures → 2/5 = 40% < 60% → stays closed
    cb.record_outcome(Outcome::Success);
    cb.record_outcome(Outcome::Success);
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Success);
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Closed);

    // Add more failures: window slides, oldest (success) falls off
    cb.record_outcome(Outcome::Failure);
    // window: [S, F, S, F, F] → 3/5 = 60% >= 60% → trips
    assert_eq!(cb.circuit_state(), CS::Open);
}
```

### Step 7: Verify

```bash
rtk cargo fmt -p nebula-resilience
rtk cargo clippy -p nebula-resilience -- -D warnings
rtk cargo nextest run -p nebula-resilience
rtk cargo test --doc -p nebula-resilience
rtk cargo bench --no-run -p nebula-resilience
```

---

## Task 7: Update docs and context

**Files:**
- Modify: `.claude/crates/resilience.md`

Add to Key Decisions:
- **RateLimited retry_after**: `CallError::RateLimited { retry_after: Some(Duration) }` with `.retry_after()` accessor.
- **Manual circuit control**: `force_open()` / `force_close()` for operational control.
- **CB state callbacks**: `.on_state_change(|from, to| { ... })` for direct notification.
- **Dynamic break duration**: `break_duration_multiplier` + `max_break_duration` — reset timeout grows exponentially.
- **Slow call detection**: `slow_call_threshold` + `slow_call_rate_threshold` — trips CB on degraded latency.
- **Sliding window**: `sliding_window_size` + `failure_rate_threshold` — rate-based tripping over rolling window.

Run full validation:
```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run -p nebula-resilience && rtk cargo test --doc -p nebula-resilience && rtk cargo bench --no-run -p nebula-resilience
```
