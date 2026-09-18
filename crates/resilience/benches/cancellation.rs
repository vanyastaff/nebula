//! Benchmarks for `with_cancellation`'s per-poll cost.
//!
//! `CancellationExt::with_cancellation` delegates to tokio-util's
//! `run_until_cancelled_owned`, which stores its cancellation wait instead of
//! rebuilding it per poll. This benchmark measures the remaining wrapper
//! overhead against the same future run unwrapped, so a regression that
//! reintroduced per-poll work would show up as that gap widening with the
//! yield count.

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
