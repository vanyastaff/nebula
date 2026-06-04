//! Lock-free read hot-path benchmarks for [`SlotCell`].
//!
//! `SlotCell` is the per-credential-slot holder a resource reads on **every**
//! acquire (via the derive-emitted `<field>_slot()` accessor), so its read
//! methods are the crate's hottest path. This bench covers:
//!
//! - `load` / `load_versioned` — the resolved-guard snapshot reads;
//! - `generation` on a **bound** cell (the live-entry path) and on an
//!   **empty** cell (the `next_generation` fallback whose ordering was relaxed
//!   from `Acquire` to `Relaxed`) — the exact code this baseline guards;
//! - `is_some`;
//! - the rare serialized `store` / `take` write path, for completeness.
//!
//! These read methods are the `#[inline]` candidates the perf research deferred
//! as "benchmark-gate"; this bench is the baseline an `#[inline]` change can be
//! measured against on CodSpeed.

use std::{hint::black_box, sync::Arc};

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resource::SlotCell;

fn bench_slotcell_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource/slotcell");

    // Bound cell: a resolved value is present — the common acquire-time read.
    let bound: SlotCell<u64> = SlotCell::empty();
    bound.store(Arc::new(0x00C0_FFEE_u64));

    group.bench_function("load_bound", |b| {
        b.iter(|| black_box(black_box(&bound).load()));
    });
    group.bench_function("load_versioned_bound", |b| {
        b.iter(|| black_box(black_box(&bound).load_versioned()));
    });
    group.bench_function("generation_bound", |b| {
        b.iter(|| black_box(black_box(&bound).generation()));
    });
    group.bench_function("is_some_bound", |b| {
        b.iter(|| black_box(black_box(&bound).is_some()));
    });

    // Empty cell: the `generation()` no-entry fallback reads `next_generation`
    // directly (the `Relaxed` load this crate relaxed from `Acquire`).
    let empty: SlotCell<u64> = SlotCell::empty();
    group.bench_function("generation_empty", |b| {
        b.iter(|| black_box(black_box(&empty).generation()));
    });
    group.bench_function("load_empty", |b| {
        b.iter(|| black_box(black_box(&empty).load()));
    });

    group.finish();
}

fn bench_slotcell_writes(c: &mut Criterion) {
    // The rare rotation/revoke write path (serialized under the write lock).
    // Not hot, but a number here makes a future write-path regression visible.
    let mut group = c.benchmark_group("resource/slotcell_write");

    group.bench_function("store", |b| {
        let cell: SlotCell<u64> = SlotCell::empty();
        let mut v = 0_u64;
        b.iter(|| {
            v = v.wrapping_add(1);
            cell.store(Arc::new(black_box(v)));
        });
    });
    group.bench_function("store_then_take", |b| {
        let cell: SlotCell<u64> = SlotCell::empty();
        b.iter(|| {
            cell.store(Arc::new(black_box(1_u64)));
            black_box(cell.take());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_slotcell_reads, bench_slotcell_writes);
criterion_main!(benches);
