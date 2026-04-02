//! Micro-benchmarks for the circuit breaker's internal `SlidingWindow`.
//!
//! Measures the two core operations:
//! - **`failure_count` / `slow_count`** — contiguous-byte sum over the active slice.
//!   With `Box<[u8]>` LLVM auto-vectorizes at window sizes ≥ ~32 entries.
//! - **`record`** — write to two byte arrays + ring-pointer advance.
//!
//! Also benchmarks `record_outcome` on a `CircuitBreaker` configured with a rate
//! threshold to verify the algebraic rewrite (`failures >= threshold * total`)
//! vs the old division form (`failures / total >= threshold`).
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench sliding_window_cb --features bench
//! ```

use std::hint::black_box;
use std::time::Duration;

use nebula_resilience::BenchSlidingWindow;
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};

fn main() {
    divan::main();
}

// ── failure_count / slow_count — pure sum over u8 slice ───────────────────────

/// Benchmark `failure_count()` at various window sizes.
///
/// This is a contiguous-byte sum. LLVM generates scalar code for small windows
/// and SIMD (SSE2/AVX2) instructions for larger ones. Expect near-linear
/// scaling up to the SIMD threshold, then sub-linear.
#[divan::bench(
    name = "failure_count",
    args = [8, 16, 32, 64, 128, 256, 512, 1_024],
    sample_count = 2_000,
)]
fn failure_count(bencher: divan::Bencher, window_size: usize) {
    let mut w = BenchSlidingWindow::new(window_size);
    // Fill with ~50% failures, ~30% slow — realistic CB workload
    for i in 0..window_size {
        w.record(i % 2 == 0, i % 3 == 0);
    }
    bencher.bench_local(|| black_box(w.failure_count()));
}

/// Same but measuring `slow_count` — separate byte array, independent SIMD path.
#[divan::bench(
    name = "slow_count",
    args = [8, 16, 32, 64, 128, 256, 512, 1_024],
    sample_count = 2_000,
)]
fn slow_count(bencher: divan::Bencher, window_size: usize) {
    let mut w = BenchSlidingWindow::new(window_size);
    for i in 0..window_size {
        w.record(i % 2 == 0, i % 3 == 0);
    }
    bencher.bench_local(|| black_box(w.slow_count()));
}

/// Benchmark both counts computed sequentially — typical CB hot path.
#[divan::bench(
    name = "failure_and_slow_count",
    args = [8, 32, 128, 512, 1_024],
    sample_count = 2_000,
)]
fn failure_and_slow_count(bencher: divan::Bencher, window_size: usize) {
    let mut w = BenchSlidingWindow::new(window_size);
    for i in 0..window_size {
        w.record(i % 2 == 0, i % 3 == 0);
    }
    bencher.bench_local(|| {
        black_box(w.failure_count());
        black_box(w.slow_count());
    });
}

// ── record — write to ring buffer ─────────────────────────────────────────────

/// Benchmark `record()` — writes `is_failure` and `is_slow` bytes into their
/// respective ring arrays and advances the head pointer.
#[divan::bench(
    name = "record",
    args = [10, 100, 1_000],
    sample_count = 2_000,
)]
fn record(bencher: divan::Bencher, window_size: usize) {
    let mut w = BenchSlidingWindow::new(window_size);
    // Warm up so we're always overwriting (ring full)
    for i in 0..window_size {
        w.record(i % 2 == 0, i % 3 == 0);
    }
    let mut i = 0usize;
    bencher.bench_local(|| {
        w.record(i % 2 == 0, i % 3 == 0);
        i = i.wrapping_add(1);
    });
}

// ── record_outcome with rate threshold (divsd hot path) ───────────────────────

/// Benchmark `CircuitBreaker::record_outcome` on the failure path with an active
/// `failure_rate_threshold`. This exercises `should_trip_on_failure` which
/// previously contained a `divsd` instruction.
///
/// CB is configured so the rate never actually trips (failures < 50%) to keep
/// it in the Closed → measure loop without state changes.
#[divan::bench(
    name = "record_outcome/failure_rate_check",
    args = [10, 100, 500, 1_000],
    sample_count = 500,
)]
fn record_outcome_failure_rate(bencher: divan::Bencher, window_size: u32) {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        sliding_window_size: window_size,
        failure_rate_threshold: Some(0.8), // trips at 80% — we'll stay below
        min_operations: 1,
        failure_threshold: window_size * 2, // fallback count threshold won't trip
        reset_timeout: Duration::from_secs(3600),
        ..Default::default()
    })
    .unwrap();

    // Pre-fill window with 30% failures (below 80% threshold → stays Closed)
    for i in 0..window_size {
        if i % 3 == 0 {
            cb.record_outcome(Outcome::Failure);
        } else {
            cb.record_outcome(Outcome::Success);
        }
    }

    bencher.bench(|| {
        // Alternate success/failure to keep failure rate stable
        cb.record_outcome(black_box(Outcome::Success));
        cb.record_outcome(black_box(Outcome::Failure));
    });
}

/// Same but with slow call rate threshold — exercises `slow_rate_trips`.
#[divan::bench(
    name = "record_outcome/slow_rate_check",
    args = [10, 100, 500, 1_000],
    sample_count = 500,
)]
fn record_outcome_slow_rate(bencher: divan::Bencher, window_size: u32) {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        sliding_window_size: window_size,
        slow_call_threshold: Some(Duration::from_millis(100)),
        slow_call_rate_threshold: 0.9, // trips at 90% — we stay below
        failure_rate_threshold: Some(0.95),
        min_operations: 1,
        failure_threshold: window_size * 2,
        reset_timeout: Duration::from_secs(3600),
        ..Default::default()
    })
    .unwrap();

    for i in 0..window_size {
        if i % 5 == 0 {
            cb.record_outcome(Outcome::SlowSuccess);
        } else {
            cb.record_outcome(Outcome::Success);
        }
    }

    bencher.bench(|| {
        cb.record_outcome(black_box(Outcome::Success));
        cb.record_outcome(black_box(Outcome::SlowSuccess));
    });
}
