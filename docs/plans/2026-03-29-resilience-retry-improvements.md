# Resilience Retry & Backoff Improvements

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enhance retry/backoff system with Fibonacci backoff, total delay budget, notify callback, jitter seed, custom backoff sequences, and pipeline fallback — inspired by backon, tower, and governor.

**Architecture:** All changes are in `crates/resilience/src/retry.rs` (tasks 1–5) and `crates/resilience/src/pipeline.rs` + `lib.rs` (task 6). Each feature is additive — no breaking changes to existing API. TDD: write failing test first, then implement.

**Tech Stack:** Rust 1.93, tokio, fastrand (already dep), existing `CallError<E>` / `RetryConfig<E>` / `BackoffConfig` types.

---

## Task 1: Fibonacci Backoff

**Files:**
- Modify: `crates/resilience/src/retry.rs` (BackoffConfig enum + delay_for)

### Step 1: Write failing test

Add to `mod tests` in `retry.rs`:

```rust
#[test]
fn fibonacci_backoff_produces_correct_sequence() {
    let cfg = BackoffConfig::Fibonacci {
        base: Duration::from_millis(100),
        max: Duration::from_secs(5),
    };
    // fib sequence: 1, 1, 2, 3, 5, 8, 13...
    // delays: 100ms*1, 100ms*1, 100ms*2, 100ms*3, 100ms*5, 100ms*8
    assert_eq!(cfg.delay_for(0), Duration::from_millis(100));
    assert_eq!(cfg.delay_for(1), Duration::from_millis(100));
    assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
    assert_eq!(cfg.delay_for(3), Duration::from_millis(300));
    assert_eq!(cfg.delay_for(4), Duration::from_millis(500));
    assert_eq!(cfg.delay_for(5), Duration::from_millis(800));
}

#[test]
fn fibonacci_backoff_respects_max() {
    let cfg = BackoffConfig::Fibonacci {
        base: Duration::from_millis(100),
        max: Duration::from_millis(250),
    };
    assert_eq!(cfg.delay_for(4), Duration::from_millis(250)); // 500 capped to 250
}
```

### Step 2: Run test, verify FAIL

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(fibonacci)'
```

Expected: compile error — `Fibonacci` variant doesn't exist yet.

### Step 3: Implement Fibonacci variant

In `BackoffConfig` enum, add variant:

```rust
/// Fibonacci-increasing delay (1, 1, 2, 3, 5, 8...), capped at `max`.
Fibonacci {
    /// Base delay multiplied by the Fibonacci number.
    base: Duration,
    /// Maximum delay cap.
    max: Duration,
},
```

In `delay_for`, add match arm:

```rust
Self::Fibonacci { base, max } => {
    let fib_n = fibonacci(attempt);
    base.saturating_mul(fib_n).min(*max)
}
```

Add helper above `delay_for`:

```rust
/// Compute the nth Fibonacci number (0-indexed: fib(0)=1, fib(1)=1, fib(2)=2, ...).
const fn fibonacci(n: u32) -> u32 {
    let (mut a, mut b) = (1u32, 1u32);
    let mut i = 0;
    while i < n {
        let next = a.saturating_add(b);
        a = b;
        b = next;
        i += 1;
    }
    a
}
```

### Step 4: Run test, verify PASS

```bash
rtk cargo nextest run -p nebula-resilience -E 'test(fibonacci)'
```

### Step 5: Clippy + commit

```bash
rtk cargo clippy -p nebula-resilience -- -D warnings
```

---

## Task 2: Total Delay Budget

**Files:**
- Modify: `crates/resilience/src/retry.rs` (RetryConfig + retry_with)

### Step 1: Write failing test

```rust
#[tokio::test]
async fn total_budget_stops_retries_early() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let config = RetryConfig::new(100) // many attempts
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(50)))
        .total_budget(Duration::from_millis(120));

    let start = std::time::Instant::now();
    let _: Result<(), CallError<&str>> = retry_with(config, || {
        let c = c.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err("fail")
        })
    })
    .await;
    let elapsed = start.elapsed();

    // With 50ms delay and 120ms budget: 1st attempt (0ms) + sleep 50ms +
    // 2nd attempt + sleep 50ms + 3rd attempt + would sleep 50ms but budget
    // exceeded → stops. Should get ~3 attempts.
    let attempts = counter.load(Ordering::SeqCst);
    assert!(attempts <= 4, "expected <= 4, got {attempts}");
    assert!(elapsed < Duration::from_millis(300), "took too long: {elapsed:?}");
}
```

### Step 2: Run test, verify FAIL

### Step 3: Implement

Add to `RetryConfig`:

```rust
total_budget: Option<Duration>,
```

Initialize to `None` in both `new()` and `new_unchecked()`.

Add builder method:

```rust
/// Set a total delay budget — retries stop if cumulative sleep time would exceed this.
#[must_use]
pub const fn total_budget(mut self, budget: Duration) -> Self {
    self.total_budget = Some(budget);
    self
}
```

In `retry_with`, add tracking before the loop:

```rust
let mut total_delay = Duration::ZERO;
```

Before sleeping, check budget:

```rust
let delay = apply_jitter(config.backoff.delay_for(attempt), &config.jitter);
if !delay.is_zero() {
    if let Some(budget) = config.total_budget {
        if total_delay + delay > budget {
            last_err = Some(e);
            break;
        }
    }
    total_delay += delay;
    tokio::time::sleep(delay).await;
}
```

### Step 4: Run test, verify PASS

### Step 5: Clippy + commit

---

## Task 3: Retry Notify Callback

**Files:**
- Modify: `crates/resilience/src/retry.rs` (RetryConfig + retry_with)

### Step 1: Write failing test

```rust
#[tokio::test]
async fn on_retry_callback_receives_error_and_delay() {
    let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
    let n = notifications.clone();

    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
        .on_retry(move |_err: &str, delay: Duration, attempt: u32| {
            n.lock().unwrap().push((attempt, delay));
        });

    let _: Result<(), CallError<&str>> =
        retry_with(config, || Box::pin(async { Err("fail") })).await;

    let notifs = notifications.lock().unwrap();
    assert_eq!(notifs.len(), 2); // 2 retries (3 attempts - 1 initial)
    assert_eq!(notifs[0].0, 1); // attempt 1
    assert_eq!(notifs[1].0, 2); // attempt 2
}
```

### Step 2: Run test, verify FAIL

### Step 3: Implement

Type alias:

```rust
type RetryNotify<E> = Box<dyn Fn(&E, Duration, u32) + Send + Sync>;
```

Add to `RetryConfig`:

```rust
on_retry: Option<RetryNotify<E>>,
```

Initialize to `None` in `new()` and `new_unchecked()`.

Builder method:

```rust
/// Register a callback invoked before each retry sleep.
///
/// Receives: `(&error, delay, attempt_number)` where attempt is 1-based.
#[must_use]
pub fn on_retry<F>(mut self, f: F) -> Self
where
    F: Fn(&E, Duration, u32) + Send + Sync + 'static,
{
    self.on_retry = Some(Box::new(f));
    self
}
```

In `retry_with`, call before sleeping:

```rust
if let Some(ref notify) = config.on_retry {
    notify(&e, delay, attempt + 1);
}
```

### Step 4: Run test, verify PASS

### Step 5: Clippy + commit

---

## Task 4: Jitter Seed for Deterministic Tests

**Files:**
- Modify: `crates/resilience/src/retry.rs` (JitterConfig + apply_jitter)

### Step 1: Write failing test

```rust
#[test]
fn seeded_jitter_is_deterministic() {
    let delay = Duration::from_millis(100);
    let jitter = JitterConfig::Full { factor: 0.5, seed: Some(42) };

    let d1 = apply_jitter(delay, &jitter);
    // Reset — same seed should produce same result
    let jitter2 = JitterConfig::Full { factor: 0.5, seed: Some(42) };
    let d2 = apply_jitter(delay, &jitter2);

    assert_eq!(d1, d2, "same seed must produce same jitter");
    assert!(d1 > delay, "jitter should add to delay");
    assert!(d1 <= Duration::from_millis(150), "factor 0.5 caps at 50% extra");
}
```

### Step 2: Run test, verify FAIL

### Step 3: Implement

Change `JitterConfig::Full` to:

```rust
Full {
    /// Maximum jitter fraction (0.0–1.0).
    factor: f64,
    /// Optional seed for deterministic jitter (useful for testing).
    seed: Option<u64>,
},
```

Update `apply_jitter`:

```rust
fn apply_jitter(delay: Duration, jitter: &JitterConfig) -> Duration {
    match jitter {
        JitterConfig::None => delay,
        JitterConfig::Full { factor, seed } => {
            let base = delay.as_secs_f64();
            let rand_val = match seed {
                Some(s) => fastrand::Rng::with_seed(*s).f64(),
                None => fastrand::f64(),
            };
            let jitter_amount = base * factor * rand_val;
            Duration::from_secs_f64(base + jitter_amount)
        }
    }
}
```

Fix existing test `jitter_adds_delay_variance` — update `JitterConfig::Full` construction to include `seed: None`.

Fix existing pipeline code in `run_retry_step` that clones jitter — add `seed: None` wherever `JitterConfig::Full` is constructed if needed.

### Step 4: Run ALL tests (not just new one — seed field changes Full variant everywhere)

```bash
rtk cargo nextest run -p nebula-resilience
```

### Step 5: Clippy + commit

---

## Task 5: Custom Backoff Sequence

**Files:**
- Modify: `crates/resilience/src/retry.rs` (BackoffConfig enum + delay_for)

### Step 1: Write failing test

```rust
#[test]
fn custom_backoff_uses_provided_delays() {
    let delays = vec![
        Duration::from_millis(10),
        Duration::from_millis(50),
        Duration::from_millis(200),
    ];
    let cfg = BackoffConfig::Custom(delays);
    assert_eq!(cfg.delay_for(0), Duration::from_millis(10));
    assert_eq!(cfg.delay_for(1), Duration::from_millis(50));
    assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
    // Past end: use last delay
    assert_eq!(cfg.delay_for(3), Duration::from_millis(200));
    assert_eq!(cfg.delay_for(99), Duration::from_millis(200));
}

#[test]
fn custom_backoff_empty_returns_zero() {
    let cfg = BackoffConfig::Custom(vec![]);
    assert_eq!(cfg.delay_for(0), Duration::ZERO);
}
```

### Step 2: Run test, verify FAIL

### Step 3: Implement

Add variant to `BackoffConfig`:

```rust
/// A user-provided sequence of delays. If attempt exceeds the list, the last delay repeats.
Custom(Vec<Duration>),
```

In `delay_for`, add match arm:

```rust
Self::Custom(delays) => {
    if delays.is_empty() {
        Duration::ZERO
    } else {
        delays[delays.len().min(attempt as usize + 1) - 1]
    }
}
```

Actually simpler:

```rust
Self::Custom(delays) => delays
    .get(attempt as usize)
    .or(delays.last())
    .copied()
    .unwrap_or(Duration::ZERO),
```

### Step 4: Run test, verify PASS

### Step 5: Clippy + commit

---

## Task 6: Pipeline Fallback Support

**Files:**
- Modify: `crates/resilience/src/pipeline.rs` (add `call_with_fallback` method)
- Modify: `crates/resilience/src/lib.rs` (if needed for re-export)

### Step 1: Write failing test

Add to pipeline `mod tests`:

```rust
#[tokio::test]
async fn pipeline_call_with_fallback_recovers() {
    use crate::fallback::ValueFallback;

    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_millis(10))
        .build();

    let fallback = ValueFallback::new(99u32);

    let result = pipeline
        .call_with_fallback(
            || Box::pin(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<u32, &str>(42)
            }),
            &fallback,
        )
        .await;

    // Operation times out, fallback provides 99
    assert_eq!(result.unwrap(), 99);
}

#[tokio::test]
async fn pipeline_call_with_fallback_passes_through_on_success() {
    use crate::fallback::ValueFallback;

    let pipeline = ResiliencePipeline::<&str>::builder().build();
    let fallback = ValueFallback::new(0u32);

    let result = pipeline
        .call_with_fallback(
            || Box::pin(async { Ok::<u32, &str>(42) }),
            &fallback,
        )
        .await;

    assert_eq!(result.unwrap(), 42);
}
```

### Step 2: Run test, verify FAIL

### Step 3: Implement

Add method to `ResiliencePipeline`:

```rust
/// Execute `f` through the pipeline with a fallback strategy.
///
/// If the pipeline returns an error and the fallback's `should_fallback` returns true,
/// the fallback strategy is invoked to recover.
///
/// # Errors
///
/// Returns the fallback's error if both the pipeline and fallback fail.
pub async fn call_with_fallback<T, F, Fut>(
    &self,
    f: F,
    fallback: &dyn crate::fallback::FallbackStrategy<T, E>,
) -> Result<T, CallError<E>>
where
    T: Send + Sync + 'static,
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
{
    match self.call(f).await {
        Ok(v) => Ok(v),
        Err(err) => {
            if fallback.should_fallback(&err) {
                fallback.fallback(err).await
            } else {
                Err(err)
            }
        }
    }
}
```

### Step 4: Run test, verify PASS

### Step 5: Clippy + full test suite

```bash
rtk cargo fmt -p nebula-resilience
rtk cargo clippy -p nebula-resilience -- -D warnings
rtk cargo nextest run -p nebula-resilience
rtk cargo test --doc -p nebula-resilience
rtk cargo bench --no-run -p nebula-resilience
```

---

## Task 7: Final — Update docs and context

**Files:**
- Modify: `.claude/crates/resilience.md`
- Modify: `crates/resilience/src/lib.rs` (re-exports if needed)

### Step 1: Update context file

Add to Key Decisions:
- **Fibonacci backoff**: `BackoffConfig::Fibonacci { base, max }` — delays grow by Fibonacci sequence multiplied by base.
- **Total delay budget**: `RetryConfig::total_budget(Duration)` — stops retries if cumulative delay exceeds budget.
- **Retry notify**: `RetryConfig::on_retry(|err, delay, attempt|)` — callback before each retry sleep.
- **Jitter seed**: `JitterConfig::Full { factor, seed: Some(42) }` for deterministic tests.
- **Custom backoff**: `BackoffConfig::Custom(vec![...])` — explicit delay sequence.
- **Pipeline fallback**: `pipeline.call_with_fallback(f, &strategy)` — integrates FallbackStrategy.

### Step 2: Run full validation

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run -p nebula-resilience && rtk cargo test --doc -p nebula-resilience && rtk cargo bench --no-run -p nebula-resilience
```
