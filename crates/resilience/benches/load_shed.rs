//! Benchmarks for the `load_shed` function.
//!
//! `load_shed` runs on every incoming request, so its predicate evaluation
//! and control-flow overhead must be minimal.  Two scenarios:
//!
//! - **Pass-through** — predicate always returns `false`; measures the overhead of the extra
//!   `async` wrapper + closure call when no shedding occurs.
//! - **Reject** — predicate always returns `true`; measures the fast rejection path where the
//!   operation is never started.
//! - **Atomic predicate** — realistic production pattern: predicate reads an `AtomicBool` set
//!   externally (e.g., by a health-check loop).
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench load_shed
//! ```

use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resilience::load_shed;

fn bench_pass_through(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("load_shed/pass_through", |b| {
        b.to_async(&rt).iter(|| async {
            let result = load_shed(|| false, || async { Ok::<u64, ()>(black_box(42)) }).await;
            black_box(result.unwrap());
        });
    });
}

fn bench_reject(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("load_shed/reject", |b| {
        b.to_async(&rt).iter(|| async {
            let result = load_shed(|| true, || async { Ok::<u64, ()>(black_box(42)) }).await;
            black_box(result.is_err());
        });
    });
}

fn bench_atomic_predicate(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("load_shed/atomic_predicate");

    // Predicate reads AtomicBool (false) — the realistic production hot path
    group.bench_function("not_shedding", |b| {
        let flag = Arc::new(AtomicBool::new(false));
        b.to_async(&rt).iter(|| {
            let flag = flag.clone();
            async move {
                let result = load_shed(
                    move || flag.load(Ordering::Relaxed),
                    || async { Ok::<u64, ()>(black_box(42)) },
                )
                .await;
                black_box(result.unwrap());
            }
        });
    });

    // Predicate reads AtomicBool (true) — overload state
    group.bench_function("shedding", |b| {
        let flag = Arc::new(AtomicBool::new(true));
        b.to_async(&rt).iter(|| {
            let flag = flag.clone();
            async move {
                let result = load_shed(
                    move || flag.load(Ordering::Relaxed),
                    || async { Ok::<u64, ()>(black_box(42)) },
                )
                .await;
                black_box(result.is_err());
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_pass_through,
    bench_reject,
    bench_atomic_predicate
);
criterion_main!(benches);
