//! Emit-path latency benchmarks for nebula-eventbus.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_eventbus::{BackPressurePolicy, EventBus};

fn bench_emit_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("eventbus/emit_latency");

    // Keep one subscriber alive so we benchmark the successful send path.
    // Large buffers reduce interference from overwrite/drop behavior during the run.
    for &buffer in &[4_096_usize, 65_536_usize] {
        let bus = EventBus::<u64>::with_policy(buffer, BackPressurePolicy::DropOldest);
        let _sub = bus.subscribe();

        group.bench_with_input(BenchmarkId::new("drop_oldest", buffer), &buffer, |b, _| {
            let mut seq = 0_u64;
            b.iter(|| {
                seq = seq.wrapping_add(1);
                let outcome = bus.emit(black_box(seq));
                black_box(outcome);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_emit_latency);
criterion_main!(benches);
