//! Benchmarks for `CancellableFuture`'s per-poll cost.
//!
//! Regression guard for #632. `CancellableFuture` used to build a fresh
//! `WaitForCancellationFuture` on **every** poll, re-registering a waker with
//! the token each time. That cost is invisible on a future that yields once and
//! dominant on one that yields constantly — which is exactly what a busy
//! workflow step looks like.
//!
//! The wrapped case is measured against the same future unwrapped, so the
//! benchmark reports the *overhead of the wrapper* rather than the cost of
//! yielding. A reintroduced per-poll rebuild shows up as that gap widening with
//! the yield count.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{CallError, CancellationExt};
use tokio_util::sync::CancellationToken;

/// Yield counts spanning "one poll" to "hot loop". The per-poll cost is only
/// visible as the count grows, so a single size would hide the regression.
const YIELD_COUNTS: [u32; 4] = [1, 16, 256, 4096];

async fn yield_n(count: u32) -> u32 {
    for _ in 0..count {
        tokio::task::yield_now().await;
    }
    count
}

fn bench_cancellable_overhead(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime builds");

    let mut group = c.benchmark_group("cancellable_future");
    for count in YIELD_COUNTS {
        group.bench_with_input(BenchmarkId::new("unwrapped", count), &count, |b, &count| {
            b.iter(|| runtime.block_on(async { black_box(yield_n(black_box(count)).await) }));
        });

        // Never cancelled: this measures the polling path, which is where the
        // per-poll rebuild lived.
        let token = CancellationToken::new();
        group.bench_with_input(BenchmarkId::new("wrapped", count), &count, |b, &count| {
            b.iter(|| {
                runtime.block_on(async {
                    let outcome: Result<u32, CallError<()>> = yield_n(black_box(count))
                        .with_cancellation(token.clone())
                        .await;
                    black_box(outcome.expect("an uncancelled future completes"))
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_cancellable_overhead);
criterion_main!(benches);
