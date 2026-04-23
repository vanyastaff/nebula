//! Property-based tests for [`BackoffConfig::delay_for`] — mathematical invariants
//! verified across the full input space via `proptest`.

use std::time::Duration;

use nebula_resilience::retry::BackoffConfig;
use proptest::prelude::*;

// ── Strategy helpers ─────────────────────────────────────────────────────────

/// Generate attempt numbers 0..=100.
fn attempt_small() -> impl Strategy<Value = u32> {
    0u32..=100
}

/// Generate attempt numbers up to 1000 for overflow testing.
fn attempt_large() -> impl Strategy<Value = u32> {
    0u32..=1000
}

// ── Fixed backoff ─────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn fixed_delay_is_constant_regardless_of_attempt(
        delay_ms in 10u64..=5000,
        attempt in attempt_small(),
    ) {
        let cfg = BackoffConfig::Fixed(Duration::from_millis(delay_ms));
        let expected = Duration::from_millis(delay_ms);
        prop_assert_eq!(cfg.delay_for(attempt), expected);
    }
}

// ── Linear backoff ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn linear_delay_attempt_zero_equals_base(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Linear {
            base: Duration::from_millis(base_ms),
            max: Duration::from_millis(max_ms),
        };
        prop_assert_eq!(cfg.delay_for(0), Duration::from_millis(base_ms));
    }

    #[test]
    fn linear_delay_increases_by_base_each_attempt(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        n in 1u32..=50,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Linear {
            base: Duration::from_millis(base_ms),
            max: Duration::from_millis(max_ms),
        };
        // Formula: delay(n) = base * n, capped at max
        // (attempt.max(1) means n >= 1 always, so delay(n) = base * n for n >= 1)
        let expected_raw = Duration::from_millis(base_ms) * n;
        let expected = expected_raw.min(Duration::from_millis(max_ms));
        prop_assert_eq!(cfg.delay_for(n), expected);
    }

    #[test]
    fn linear_delay_monotonic_non_decreasing(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        n in 0u32..=99,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Linear {
            base: Duration::from_millis(base_ms),
            max: Duration::from_millis(max_ms),
        };
        prop_assert!(cfg.delay_for(n) <= cfg.delay_for(n + 1));
    }

    #[test]
    fn linear_delay_never_exceeds_max(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        attempt in attempt_small(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let cfg = BackoffConfig::Linear {
            base: Duration::from_millis(base_ms),
            max,
        };
        prop_assert!(cfg.delay_for(attempt) <= max);
    }
}

// ── Exponential backoff ───────────────────────────────────────────────────────

proptest! {
    #[test]
    fn exponential_delay_attempt_zero_equals_base(
        base_ms in 10u64..=5000,
        multiplier in 1.0f64..=3.0,
        max_ms in 10u64..=30000,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(base_ms),
            multiplier,
            max: Duration::from_millis(max_ms),
        };
        prop_assert_eq!(cfg.delay_for(0), Duration::from_millis(base_ms));
    }

    #[test]
    fn exponential_delay_monotonic_non_decreasing(
        base_ms in 10u64..=5000,
        multiplier in 1.0f64..=3.0,
        max_ms in 10u64..=30000,
        n in 0u32..=50,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(base_ms),
            multiplier,
            max: Duration::from_millis(max_ms),
        };
        prop_assert!(cfg.delay_for(n) <= cfg.delay_for(n + 1));
    }

    #[test]
    fn exponential_delay_never_exceeds_max(
        base_ms in 10u64..=5000,
        multiplier in 1.0f64..=3.0,
        max_ms in 10u64..=30000,
        attempt in attempt_small(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(base_ms),
            multiplier,
            max,
        };
        prop_assert!(cfg.delay_for(attempt) <= max);
    }

    #[test]
    fn exponential_delay_no_panic_for_large_attempts(
        base_ms in 10u64..=5000,
        multiplier in 1.0f64..=3.0,
        max_ms in 10u64..=30000,
        attempt in attempt_large(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(base_ms),
            multiplier,
            max,
        };
        // Must not panic; result must be valid
        let delay = cfg.delay_for(attempt);
        prop_assert!(delay >= Duration::ZERO);
        prop_assert!(delay <= max);
    }

    #[test]
    fn exponential_delay_grows_exponentially(
        base_ms in 10u64..=5000,
        multiplier in 1.5f64..=3.0,
        max_ms in 10u64..=30000,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Exponential {
            base: Duration::from_millis(base_ms),
            multiplier,
            max: Duration::from_millis(max_ms),
        };
        // For multiplier > 1, delay should strictly increase until capped
        let d0 = cfg.delay_for(0);
        let d1 = cfg.delay_for(1);
        let d2 = cfg.delay_for(2);
        // d1 > d0 unless already at max
        if d0 < Duration::from_millis(max_ms) {
            prop_assert!(d1 > d0, "exponential should grow: d1={d1:?} > d0={d0:?}");
        }
        // d2 > d1 unless already at max
        if d1 < Duration::from_millis(max_ms) {
            prop_assert!(d2 > d1, "exponential should grow: d2={d2:?} > d1={d1:?}");
        }
    }
}

// ── Fibonacci backoff ─────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn fibonacci_delay_attempt_zero_equals_base(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(base_ms),
            max: Duration::from_millis(max_ms),
        };
        // fib(0) = 1, so delay(0) = base * 1 = base
        prop_assert_eq!(cfg.delay_for(0), Duration::from_millis(base_ms));
    }

    #[test]
    fn fibonacci_follows_sequence_pattern(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let base = Duration::from_millis(base_ms);
        let cfg = BackoffConfig::Fibonacci { base, max };

        // Fibonacci: fib(0)=1, fib(1)=1, fib(2)=2, fib(3)=3, fib(4)=5, fib(5)=8
        // delay(n) = base * fib(n), capped at max
        // Verify: delay(n) = delay(n-1) + delay(n-2) for n >= 2 (before capping)
        // After capping, the relationship may not hold, so we verify for small n
        // where the cap is unlikely to be hit.
        if base * 8 <= max {
            // fib sequence: 1, 1, 2, 3, 5, 8
            prop_assert_eq!(cfg.delay_for(0), base * 1);
            prop_assert_eq!(cfg.delay_for(1), base * 1);
            prop_assert_eq!(cfg.delay_for(2), base * 2);
            prop_assert_eq!(cfg.delay_for(3), base * 3);
            prop_assert_eq!(cfg.delay_for(4), base * 5);
            prop_assert_eq!(cfg.delay_for(5), base * 8);

            // Fibonacci recurrence: delay(n) = delay(n-1) + delay(n-2)
            for n in 2u32..=5 {
                prop_assert_eq!(
                    cfg.delay_for(n),
                    cfg.delay_for(n - 1) + cfg.delay_for(n - 2),
                    "fibonacci recurrence broken"
                );
            }
        }
    }

    #[test]
    fn fibonacci_delay_monotonic_non_decreasing(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        n in 0u32..=50,
    ) {
        prop_assume!(max_ms >= base_ms);
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(base_ms),
            max: Duration::from_millis(max_ms),
        };
        prop_assert!(cfg.delay_for(n) <= cfg.delay_for(n + 1));
    }

    #[test]
    fn fibonacci_delay_never_exceeds_max(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        attempt in attempt_small(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(base_ms),
            max,
        };
        prop_assert!(cfg.delay_for(attempt) <= max);
    }

    #[test]
    fn fibonacci_delay_no_panic_for_large_attempts(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        attempt in attempt_large(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let max = Duration::from_millis(max_ms);
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(base_ms),
            max,
        };
        // saturating_mul prevents overflow; must not panic
        let delay = cfg.delay_for(attempt);
        prop_assert!(delay >= Duration::ZERO);
        prop_assert!(delay <= max);
    }
}

// ── Custom backoff ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn custom_backoff_returns_correct_delay_for_valid_index(
        delays_ms in prop::collection::vec(10u64..=5000, 1..=8),
        idx in 0usize..=7,
    ) {
        prop_assume!(idx < delays_ms.len());
        let delays: Vec<Duration> = delays_ms.iter().map(|&ms| Duration::from_millis(ms)).collect();
        let cfg = BackoffConfig::Custom(delays.clone().into());
        prop_assert_eq!(cfg.delay_for(idx as u32), delays[idx]);
    }

    #[test]
    fn custom_backoff_repeats_last_delay_beyond_length(
        delays_ms in prop::collection::vec(10u64..=5000, 1..=4),
        extra in 1u32..=50,
    ) {
        let delays: Vec<Duration> = delays_ms.iter().map(|&ms| Duration::from_millis(ms)).collect();
        let cfg = BackoffConfig::Custom(delays.clone().into());
        let beyond = (delays.len() as u32) + extra;
        prop_assert_eq!(cfg.delay_for(beyond), *delays.last().unwrap());
    }

    #[test]
    fn custom_backoff_empty_returns_zero(
        attempt in attempt_small(),
    ) {
        let cfg = BackoffConfig::Custom(smallvec::SmallVec::new());
        prop_assert_eq!(cfg.delay_for(attempt), Duration::ZERO);
    }
}

// ── Jitter properties ─────────────────────────────────────────────────────────
//
// `apply_jitter` is a private function, so we test jitter invariants through the
// public API: construct a seeded `JitterConfig::Full` and verify that the
// deterministic output respects its contract. Since jitter is applied on top of
// the backoff delay inside the retry loop (not via `delay_for`), we verify the
// contract of the seeded jitter directly by re-implementing the deterministic
// path from `apply_jitter_full`.

/// Replicate the deterministic seeded-jitter computation from `apply_jitter_full`
/// to verify invariants without depending on the private function.
fn seeded_jitter_delay(delay: Duration, factor: f64, seed: u64, attempt: u32) -> Duration {
    if !(factor > 0.0) {
        return delay;
    }
    let base = delay.as_secs_f64();
    let clamped_factor = factor.min(1.0);
    let rand_val = fastrand::Rng::with_seed(seed.wrapping_add(u64::from(attempt))).f64();
    let total = base + clamped_factor * base * rand_val;
    if !total.is_finite() {
        return delay;
    }
    Duration::from_secs_f64(total.min(Duration::MAX.as_secs_f64()))
}

proptest! {
    #[test]
    fn seeded_full_jitter_never_exceeds_factor_cap(
        delay_ms in 10u64..=5000,
        factor in 0.01f64..=1.0,
        seed in any::<u64>(),
        attempt in attempt_small(),
    ) {
        let delay = Duration::from_millis(delay_ms);
        let jittered = seeded_jitter_delay(delay, factor, seed, attempt);
        // With factor f, max jittered = delay * (1 + f)
        let upper = delay.mul_f64(1.0 + factor);
        prop_assert!(
            jittered <= upper + Duration::from_micros(1), // floating-point tolerance
            "jittered={jittered:?} exceeds upper bound={upper:?}"
        );
    }

    #[test]
    fn seeded_full_jitter_never_below_base_delay(
        delay_ms in 10u64..=5000,
        factor in 0.01f64..=1.0,
        seed in any::<u64>(),
        attempt in attempt_small(),
    ) {
        let delay = Duration::from_millis(delay_ms);
        let jittered = seeded_jitter_delay(delay, factor, seed, attempt);
        // Full jitter adds to the delay; it never subtracts
        prop_assert!(
            jittered >= delay,
            "jittered={jittered:?} is below base delay={delay:?}"
        );
    }

    #[test]
    fn seeded_jitter_deterministic_for_same_inputs(
        delay_ms in 10u64..=5000,
        factor in 0.01f64..=1.0,
        seed in any::<u64>(),
        attempt in attempt_small(),
    ) {
        let delay = Duration::from_millis(delay_ms);
        let d1 = seeded_jitter_delay(delay, factor, seed, attempt);
        let d2 = seeded_jitter_delay(delay, factor, seed, attempt);
        prop_assert_eq!(d1, d2, "same inputs must produce same output");
    }
}

// ── Universal properties (all strategies) ─────────────────────────────────────

proptest! {
    #[test]
    fn all_backoff_delays_are_non_negative(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        multiplier in 1.0f64..=3.0,
        attempt in attempt_small(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let base = Duration::from_millis(base_ms);
        let max = Duration::from_millis(max_ms);

        let strategies = vec![
            BackoffConfig::Fixed(base),
            BackoffConfig::Linear { base, max },
            BackoffConfig::Exponential { base, multiplier, max },
            BackoffConfig::Fibonacci { base, max },
        ];

        for cfg in &strategies {
            let delay = cfg.delay_for(attempt);
            prop_assert!(
                delay >= Duration::ZERO,
                "negative delay from {cfg:?} at attempt {attempt}"
            );
        }
    }

    #[test]
    fn all_capped_backoff_delays_respect_max(
        base_ms in 10u64..=5000,
        max_ms in 10u64..=30000,
        multiplier in 1.0f64..=3.0,
        attempt in attempt_small(),
    ) {
        prop_assume!(max_ms >= base_ms);
        let base = Duration::from_millis(base_ms);
        let max = Duration::from_millis(max_ms);

        let strategies = vec![
            BackoffConfig::Linear { base, max },
            BackoffConfig::Exponential { base, multiplier, max },
            BackoffConfig::Fibonacci { base, max },
        ];

        for cfg in &strategies {
            let delay = cfg.delay_for(attempt);
            prop_assert!(
                delay <= max,
                "delay {delay:?} exceeds max {max:?} from {cfg:?} at attempt {attempt}"
            );
        }
    }
}
