//! Micro-benchmarks for the circuit breaker's internal `OutcomeWindow`.
//!
//! Measures the two core operations:
//! - **`failure_count` / `slow_count`** — contiguous-byte sum over the active slice. With
//!   `Box<[u8]>` LLVM auto-vectorizes at window sizes ≥ ~32 entries.
//! - **`record`** — write to two byte arrays + ring-pointer advance.
//!
//! Also benchmarks `record_outcome` on a `CircuitBreaker` configured with a rate
//! threshold to verify the algebraic rewrite (`failures >= threshold * total`)
//! vs the old division form (`failures / total >= threshold`).
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench sliding_window_cb
//! ```

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    OutcomeWindow,
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome},
};

// ── failure_count / slow_count ────────────────────────────────────────────────

fn bench_failure_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliding_window_cb/failure_count");
    for window_size in [8usize, 32, 128, 512, 1_024] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window_size),
            &window_size,
            |b, &n| {
                let mut w = OutcomeWindow::new(n);
                for i in 0..n {
                    w.record(i.is_multiple_of(2), i.is_multiple_of(3));
                }
                b.iter(|| black_box(w.failure_count()));
            },
        );
    }
    group.finish();
}

fn bench_slow_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliding_window_cb/slow_count");
    for window_size in [8usize, 32, 128, 512, 1_024] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window_size),
            &window_size,
            |b, &n| {
                let mut w = OutcomeWindow::new(n);
                for i in 0..n {
                    w.record(i.is_multiple_of(2), i.is_multiple_of(3));
                }
                b.iter(|| black_box(w.slow_count()));
            },
        );
    }
    group.finish();
}

fn bench_record(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliding_window_cb/record");
    for window_size in [10usize, 100, 1_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window_size),
            &window_size,
            |b, &n| {
                let mut w = OutcomeWindow::new(n);
                for i in 0..n {
                    w.record(i.is_multiple_of(2), i.is_multiple_of(3));
                }
                let mut i = 0usize;
                b.iter(|| {
                    w.record(i.is_multiple_of(2), i.is_multiple_of(3));
                    i = i.wrapping_add(1);
                });
            },
        );
    }
    group.finish();
}

// ── record_outcome with rate threshold ───────────────────────────────────────

fn bench_record_outcome_failure_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliding_window_cb/record_outcome/failure_rate_check");
    for window_size in [10u32, 100, 500, 1_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window_size),
            &window_size,
            |b, &n| {
                let cb = CircuitBreaker::new(CircuitBreakerConfig {
                    sliding_window_size: n,
                    failure_rate_threshold: Some(0.8),
                    min_operations: 1,
                    failure_threshold: n * 2,
                    reset_timeout: Duration::from_hours(1),
                    ..Default::default()
                })
                .unwrap();
                for i in 0..n {
                    if i % 3 == 0 {
                        cb.record_outcome(Outcome::Failure);
                    } else {
                        cb.record_outcome(Outcome::Success);
                    }
                }
                b.iter(|| {
                    cb.record_outcome(black_box(Outcome::Success));
                    cb.record_outcome(black_box(Outcome::Failure));
                });
            },
        );
    }
    group.finish();
}

fn bench_record_outcome_slow_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliding_window_cb/record_outcome/slow_rate_check");
    for window_size in [10u32, 100, 500, 1_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window_size),
            &window_size,
            |b, &n| {
                let cb = CircuitBreaker::new(CircuitBreakerConfig {
                    sliding_window_size: n,
                    slow_call_threshold: Some(Duration::from_millis(100)),
                    slow_call_rate_threshold: 0.9,
                    failure_rate_threshold: Some(0.95),
                    min_operations: 1,
                    failure_threshold: n * 2,
                    reset_timeout: Duration::from_hours(1),
                    ..Default::default()
                })
                .unwrap();
                for i in 0..n {
                    if i % 5 == 0 {
                        cb.record_outcome(Outcome::SlowSuccess);
                    } else {
                        cb.record_outcome(Outcome::Success);
                    }
                }
                b.iter(|| {
                    cb.record_outcome(black_box(Outcome::Success));
                    cb.record_outcome(black_box(Outcome::SlowSuccess));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_failure_count,
    bench_slow_count,
    bench_record,
    bench_record_outcome_failure_rate,
    bench_record_outcome_slow_rate,
);
criterion_main!(benches);
