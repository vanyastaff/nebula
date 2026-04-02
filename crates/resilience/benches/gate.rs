//! Benchmarks for the `Gate` graceful-shutdown primitive.
//!
//! Covers:
//! - **`enter` uncontended** — single-thread acquire + drop of `GateGuard`.
//!   The critical path is one `try_acquire()` on a semaphore plus a closing-flag check.
//! - **`enter` contended** — N concurrent tasks all calling `enter()` while the gate
//!   is open.  Measures semaphore permit contention overhead.
//! - **`is_closed` hot check** — polling the closing flag (used by long-running tasks
//!   to detect shutdown without holding the gate).
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench gate
//! ```

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::gate::Gate;
use std::hint::black_box;
use std::sync::Arc;

// ── enter / drop (uncontended) ────────────────────────────────────────────────

fn bench_enter_uncontended(c: &mut Criterion) {
    let gate = Gate::new();
    c.bench_function("gate/enter/uncontended", |b| {
        b.iter(|| {
            let guard = black_box(gate.enter()).unwrap();
            drop(black_box(guard));
        });
    });
}

// ── is_closed hot check ───────────────────────────────────────────────────────

fn bench_is_closed(c: &mut Criterion) {
    let gate = Gate::new();
    c.bench_function("gate/is_closed/open", |b| {
        b.iter(|| black_box(gate.is_closed()));
    });
}

// ── enter contended ───────────────────────────────────────────────────────────

async fn enter_and_drop(gate: Arc<Gate>) {
    let guard = gate.enter().unwrap();
    black_box(&guard);
    drop(guard);
}

fn bench_enter_contended(c: &mut Criterion) {
    let mut group = c.benchmark_group("gate/enter/contended");
    let rt = tokio::runtime::Runtime::new().unwrap();

    for tasks in [2usize, 8, 32, 128] {
        group.bench_with_input(BenchmarkId::from_parameter(tasks), &tasks, |b, &n| {
            b.to_async(&rt).iter(|| async move {
                let gate = Arc::new(Gate::new());
                let handles: Vec<_> = (0..n)
                    .map(|_| tokio::spawn(enter_and_drop(gate.clone())))
                    .collect();
                for h in handles {
                    h.await.unwrap();
                }
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_enter_uncontended,
    bench_is_closed,
    bench_enter_contended
);
criterion_main!(benches);
