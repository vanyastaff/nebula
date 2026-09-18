use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_error::{Classify, ErrorCategory, ErrorCode, RetryHint, codes};

use super::*;
use crate::{CallError, RecordingSink, ResilienceEventKind};

/// Test error type implementing Classify. Always retryable.
#[derive(Debug, Clone, PartialEq)]
struct TransientErr(&'static str);
impl fmt::Display for TransientErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Classify for TransientErr {
    fn category(&self) -> ErrorCategory {
        ErrorCategory::External
    }
    fn code(&self) -> ErrorCode {
        codes::INTERNAL
    }
}

/// Test error with variants for retryable/non-retryable.
#[derive(Debug)]
enum TestApiErr {
    Timeout,
    AuthFailed,
    RateLimited(Duration),
}
impl fmt::Display for TestApiErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl Classify for TestApiErr {
    fn category(&self) -> ErrorCategory {
        match self {
            Self::Timeout => ErrorCategory::Timeout,
            Self::AuthFailed => ErrorCategory::Authentication,
            Self::RateLimited(_) => ErrorCategory::RateLimit,
        }
    }
    fn code(&self) -> ErrorCode {
        codes::INTERNAL
    }
    fn retry_hint(&self) -> Option<RetryHint> {
        match self {
            Self::RateLimited(d) => Some(RetryHint::after(*d)),
            _ => None,
        }
    }
}

fn fail_twice(counter: &AtomicU32) -> Result<u32, TransientErr> {
    let n = counter.fetch_add(1, Ordering::SeqCst);
    if n < 2 {
        Err(TransientErr("fail"))
    } else {
        Ok(99)
    }
}

#[tokio::test]
async fn retries_up_to_max_attempts() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

    let result: Result<(), CallError<TransientErr>> = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err(TransientErr("fail"))
    })
    .await;

    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted { attempts: 3, .. })
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn stops_on_success() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let config = RetryConfig::new(5)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

    let result: Result<u32, CallError<TransientErr>> =
        retry_with(config, async || fail_twice(&c)).await;

    assert_eq!(result.unwrap(), 99);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn retry_if_predicate_overrides_classify() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    // Timeout IS retryable by Classify, but predicate says no
    let config = RetryConfig::new(5)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
        .retry_if(|_: &TestApiErr| false);

    let result = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err::<u32, TestApiErr>(TestApiErr::Timeout)
    })
    .await;

    // Custom predicate takes precedence → 1 attempt
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(matches!(result, Err(CallError::Operation(_))));
}

#[tokio::test]
async fn emits_retry_attempt_events() {
    let sink = RecordingSink::new();
    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
        .with_sink(sink.clone());

    let _: Result<(), CallError<TransientErr>> =
        retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

    assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
}

#[test]
fn fibonacci_backoff_produces_correct_sequence() {
    let cfg = BackoffConfig::Fibonacci {
        base: Duration::from_millis(100),
        max: Duration::from_secs(5),
    };
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
    assert_eq!(cfg.delay_for(4), Duration::from_millis(250));
}

#[test]
fn linear_backoff_saturates_instead_of_panicking() {
    let cfg = BackoffConfig::Linear {
        base: Duration::MAX,
        max: Duration::MAX,
    };

    assert_eq!(cfg.delay_for(u32::MAX), Duration::MAX);
}

#[tokio::test]
async fn on_retry_callback_receives_error_and_delay() {
    let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
    let n = notifications.clone();

    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
        .on_retry(move |_err: &TransientErr, delay: Duration, attempt: u32| {
            n.lock().unwrap().push((attempt, delay));
        });

    let _: Result<(), CallError<TransientErr>> =
        retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

    let notifs = notifications.lock().unwrap();
    assert_eq!(notifs.len(), 2); // 2 retries (3 attempts - 1 initial)
    assert_eq!(notifs[0].0, 1);
    assert_eq!(notifs[1].0, 2);
    drop(notifs);
}

#[tokio::test]
async fn jitter_adds_delay_variance() {
    // With full jitter (factor=1.0), total delay should be between
    // base and 2×base on average. We verify it doesn't exceed 3×base
    // (generous bound to avoid flakiness).
    let base = Duration::from_millis(10);
    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(base))
        .jitter(JitterConfig::Additive {
            max_fraction: 1.0,
            seed: None,
        });

    let start = std::time::Instant::now();
    let _: Result<(), CallError<TransientErr>> =
        retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;
    let elapsed = start.elapsed();

    // 2 retries × base = 20ms minimum (no jitter on first attempt which uses attempt=0)
    // With jitter factor 1.0, max theoretical = 2 × 2×base = 40ms
    // Use a generous upper bound to avoid flakiness
    assert!(
        elapsed >= Duration::from_millis(20),
        "expected >= 20ms, got {elapsed:?}"
    );
}

#[test]
fn seeded_jitter_is_deterministic_for_same_attempt() {
    let delay = Duration::from_millis(100);
    let jitter = JitterConfig::Additive {
        max_fraction: 0.5,
        seed: Some(42),
    };
    let d1 = apply_jitter(delay, &jitter, 0);
    let d2 = apply_jitter(delay, &jitter, 0);

    assert_eq!(d1, d2, "same seed + same attempt must produce same jitter");
    assert!(d1 > delay, "jitter should add to delay");
    assert!(
        d1 <= Duration::from_millis(150),
        "factor 0.5 caps at 50% extra"
    );
}

#[test]
fn seeded_jitter_varies_across_attempts() {
    let delay = Duration::from_millis(100);
    let jitter = JitterConfig::Additive {
        max_fraction: 0.5,
        seed: Some(42),
    };
    let d0 = apply_jitter(delay, &jitter, 0);
    let d1 = apply_jitter(delay, &jitter, 1);
    let d2 = apply_jitter(delay, &jitter, 2);

    // Different attempts should (almost certainly) produce different jitter
    assert!(
        d0 != d1 || d1 != d2,
        "seeded jitter should vary per attempt: d0={d0:?}, d1={d1:?}, d2={d2:?}"
    );
}

#[test]
fn jitter_with_nan_factor_falls_back_to_base_delay() {
    let delay = Duration::from_millis(100);
    let nan_jitter = JitterConfig::Additive {
        max_fraction: f64::NAN,
        seed: Some(42),
    };
    assert_eq!(apply_jitter(delay, &nan_jitter, 0), delay);

    let neg_jitter = JitterConfig::Additive {
        max_fraction: -1.0,
        seed: Some(42),
    };
    assert_eq!(apply_jitter(delay, &neg_jitter, 0), delay);

    let zero_jitter = JitterConfig::Additive {
        max_fraction: 0.0,
        seed: Some(42),
    };
    assert_eq!(apply_jitter(delay, &zero_jitter, 0), delay);
}

#[test]
fn jitter_with_infinite_factor_clamps_to_one() {
    let delay = Duration::from_millis(100);
    let jitter = JitterConfig::Additive {
        max_fraction: f64::INFINITY,
        seed: Some(42),
    };
    // Infinity is clamped to 1.0 by factor.min(1.0), so jitter is applied
    let result = apply_jitter(delay, &jitter, 0);
    assert!(result >= delay, "clamped infinity factor should add jitter");
}

#[tokio::test]
async fn total_budget_check_handles_large_backoff_without_panic() {
    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Custom(SmallVec::from_slice(&[
            Duration::MAX,
        ])))
        .total_budget(Duration::from_secs(1));

    let result: Result<(), CallError<TransientErr>> =
        retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
}

#[test]
fn custom_backoff_uses_provided_delays() {
    let cfg = BackoffConfig::Custom(SmallVec::from_slice(&[
        Duration::from_millis(10),
        Duration::from_millis(50),
        Duration::from_millis(200),
    ]));
    assert_eq!(cfg.delay_for(0), Duration::from_millis(10));
    assert_eq!(cfg.delay_for(1), Duration::from_millis(50));
    assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
    assert_eq!(cfg.delay_for(3), Duration::from_millis(200));
    assert_eq!(cfg.delay_for(99), Duration::from_millis(200));
}

#[test]
fn custom_backoff_empty_returns_zero() {
    let cfg = BackoffConfig::Custom(SmallVec::new());
    assert_eq!(cfg.delay_for(0), Duration::ZERO);
}

#[test]
fn exponential_backoff_invalid_multiplier_falls_back_to_base_delay() {
    let max = Duration::from_secs(10);

    for multiplier in [f64::NAN, f64::INFINITY, 0.0, -2.0, 0.5] {
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(25),
            multiplier,
            max,
        };

        assert_eq!(cfg.delay_for(3), Duration::from_millis(25));
    }
}

#[test]
fn exponential_backoff_infinite_result_caps_at_max() {
    let cfg = BackoffConfig::Exponential {
        base: Duration::from_secs(1),
        multiplier: f64::MAX,
        max: Duration::from_secs(9),
    };

    assert_eq!(cfg.delay_for(2), Duration::from_secs(9));
}

// ── Classify integration tests ──────────────────────────────────────────

#[tokio::test]
async fn retry_auto_skips_non_retryable() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let result = retry(NonZeroU32::new(3).unwrap(), async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(TestApiErr::AuthFailed)
    })
    .await;

    // Auth is not retryable — stops after 1 attempt
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(matches!(
        result,
        Err(CallError::Operation(TestApiErr::AuthFailed))
    ));
}

#[tokio::test]
async fn retry_auto_retries_retryable() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let result = retry(NonZeroU32::new(3).unwrap(), async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(TestApiErr::Timeout)
    })
    .await;

    // Timeout IS retryable — exhausts all 3 attempts
    assert_eq!(counter.load(Ordering::SeqCst), 3);
    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted { attempts: 3, .. })
    ));
}

#[tokio::test]
async fn retry_respects_hint_floor() {
    let start = std::time::Instant::now();
    let config = RetryConfig::new(2)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

    // RateLimited error with 50ms hint — should override 1ms backoff
    let _: Result<(), CallError<TestApiErr>> = retry_with(config, || {
        Box::pin(async { Err(TestApiErr::RateLimited(Duration::from_millis(50))) })
    })
    .await;

    let elapsed = start.elapsed();
    // 1 retry with hint floor of 50ms — should take at least ~50ms
    assert!(
        elapsed >= Duration::from_millis(45),
        "expected >= 45ms (hint floor), got {elapsed:?}"
    );
}

#[tokio::test]
async fn total_budget_stops_retries_early() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let config = RetryConfig::new(100)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::from_millis(50)))
        .total_budget(Duration::from_millis(120));

    let start = std::time::Instant::now();
    let _: Result<(), CallError<TransientErr>> = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err(TransientErr("fail"))
    })
    .await;
    let elapsed = start.elapsed();

    let attempts = counter.load(Ordering::SeqCst);
    assert!(attempts <= 4, "expected <= 4, got {attempts}");
    assert!(
        elapsed < Duration::from_millis(300),
        "took too long: {elapsed:?}"
    );
}

// ── B3: total_budget works with zero-delay backoff ───────────────────

#[tokio::test]
async fn total_budget_limits_zero_delay_retries() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    // Zero delay + tiny budget → budget should still stop retries
    // because wall-clock time of executing ops eventually exceeds budget.
    let config = RetryConfig::new(1000)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::ZERO))
        .total_budget(Duration::from_millis(50));

    let _: Result<(), CallError<TransientErr>> = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        // Each op sleeps 5ms → after ~10 ops, 50ms budget is hit
        tokio::time::sleep(Duration::from_millis(5)).await;
        Err(TransientErr("fail"))
    })
    .await;

    let attempts = counter.load(Ordering::SeqCst);
    // With 5ms per op and 50ms budget, should stop around 10 attempts (not 1000)
    assert!(
        attempts < 50,
        "expected budget to stop retries, got {attempts} attempts"
    );
}

/// An operation that fails *instantly* — no await of its own — must still be
/// bounded by the budget, not by `max_attempts`.
///
/// `Deadline::timeout` re-reads the remaining budget before every attempt, so
/// a million zero-cost attempts cannot burn through the budget unobserved.
#[tokio::test]
async fn total_budget_bounds_instant_failing_operation() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let config = RetryConfig::new(1_000_000)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::ZERO))
        .total_budget(Duration::from_millis(5));

    let start = std::time::Instant::now();
    let result: Result<(), CallError<TransientErr>> = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        Err(TransientErr("instant fail"))
    })
    .await;

    assert!(
        matches!(result, Err(CallError::Timeout(_))),
        "instant retries must exhaust the budget, got {result:?}"
    );
    let attempts = counter.load(Ordering::SeqCst);
    assert!(
        attempts < 1_000_000,
        "the attempt count, not the budget, bounded the loop: {attempts}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "budget of 5ms must not run for {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn total_budget_times_out_hung_attempt() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let config = RetryConfig::new(3)
        .unwrap()
        .backoff(BackoffConfig::Fixed(Duration::ZERO))
        .total_budget(Duration::from_millis(20));

    let start = std::time::Instant::now();
    let result: Result<(), CallError<TransientErr>> = retry_with(config, async || {
        c.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(10)).await;
        Err(TransientErr("fail"))
    })
    .await;

    assert!(matches!(result, Err(CallError::Timeout(d)) if d == Duration::from_millis(20)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "hung attempt was not bounded by retry budget"
    );
}

// ── B4: pipeline forwards retry_after from rate limiter ──────────────

#[tokio::test]
async fn pipeline_forwards_rate_limiter_retry_after() {
    use crate::pipeline::{RateLimitCheck, ResiliencePipeline};

    let hint = Duration::from_secs(42);
    let rl: RateLimitCheck = Arc::new(move || {
        Box::pin(async move {
            Err(CallError::RateLimited {
                retry_after: Some(hint),
            })
        })
    });

    let pipeline = ResiliencePipeline::<&str>::builder()
        .rate_limiter(rl)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    match result {
        Err(CallError::RateLimited { retry_after }) => {
            assert_eq!(
                retry_after,
                Some(hint),
                "retry_after hint should be forwarded"
            );
        },
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

// ── D1: rate_limiter_from convenience method ─────────────────────────

#[tokio::test]
async fn pipeline_rate_limiter_from_works() {
    use crate::{pipeline::ResiliencePipeline, rate_limiter::TokenBucket};

    let rl = Arc::new(TokenBucket::new(1, 0.001).unwrap());

    let pipeline = ResiliencePipeline::<&str>::builder()
        .rate_limiter_from(Arc::clone(&rl))
        .build();

    // First call succeeds (1 token available)
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;
    assert_eq!(result.unwrap(), 42);

    // Second call should be rate limited (no tokens left)
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;
    assert!(matches!(result, Err(CallError::RateLimited { .. })));
}

// ── General exponential path: numeric parity with the old powi formula ──────

/// The general exponential path (multiplier not 1.0 or 2.0) must produce the
/// same delay as `base_millis as f64 * multiplier.powi(attempt as i32) *`
/// truncated to milliseconds, capped at `max`.
///
/// That was the implementation before `powi_nonnegative` replaced the
/// `__powidf2` libcall; it stays the oracle so a future "optimization" cannot
/// silently change retry timing.
#[test]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test oracle deliberately reproduces the old f64/powi formula, including its truncation"
)]
fn exponential_general_path_matches_powi_oracle() {
    let attempts = [0u32, 1, 2, 3, 5, 8, 13, 21, 34, 55];
    let multipliers = [1.1f64, 1.25, 1.5, 1.7, 2.5, 3.0];
    let cases = [(10u64, 5_000u64), (100, 30_000), (1_000, 60_000), (7, 9)];

    for (base_ms, max_ms) in cases {
        let base = Duration::from_millis(base_ms);
        let max = Duration::from_millis(max_ms);
        for multiplier in multipliers {
            let cfg = BackoffConfig::Exponential {
                base,
                multiplier,
                max,
            };
            for attempt in attempts {
                let oracle_ms = base_ms as f64 * multiplier.powi(attempt as i32);
                let oracle = if !oracle_ms.is_finite() || oracle_ms >= max_ms as f64 {
                    max
                } else {
                    Duration::from_millis(oracle_ms as u64).min(max)
                };
                assert_eq!(
                    cfg.delay_for(attempt),
                    oracle,
                    "base={base_ms}ms max={max_ms}ms multiplier={multiplier} attempt={attempt}"
                );
            }
        }
    }
}

/// A multiplier above 1.0 must strictly grow the delay until the cap, on the
/// general path as well as the doubling path.
#[test]
fn exponential_general_path_grows_until_capped() {
    let cfg = BackoffConfig::Exponential {
        base: Duration::from_millis(50),
        multiplier: 1.5,
        max: Duration::from_secs(10),
    };

    assert_eq!(cfg.delay_for(0), Duration::from_millis(50));
    assert_eq!(cfg.delay_for(1), Duration::from_millis(75));
    assert_eq!(cfg.delay_for(2), Duration::from_millis(112));
    let capped = cfg.delay_for(50);
    assert_eq!(capped, Duration::from_secs(10));
    assert!(cfg.delay_for(25) <= capped);
}
