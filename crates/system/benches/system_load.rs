//! Benchmarks for nebula-system hot paths.
//!
//! Measures the cost of each monitoring function — the numbers that matter
//! for deciding polling intervals in runtime/engine.
//!
//! Run: `cargo bench -p nebula-system`

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_system_load(c: &mut Criterion) {
    // Warm up sysinfo caches
    nebula_system::init().unwrap();

    let mut group = c.benchmark_group("system_load");

    group.bench_function("system_load (single lock)", |b| {
        b.iter(|| black_box(nebula_system::load::system_load()));
    });

    group.bench_function("cpu::usage (with Vec alloc)", |b| {
        b.iter(|| black_box(nebula_system::cpu::usage()));
    });

    group.bench_function("cpu::pressure (zero alloc)", |b| {
        b.iter(|| black_box(nebula_system::cpu::pressure()));
    });

    group.bench_function("memory::current", |b| {
        b.iter(|| black_box(nebula_system::memory::current()));
    });

    group.bench_function("memory::pressure", |b| {
        b.iter(|| black_box(nebula_system::memory::pressure()));
    });

    group.bench_function("cpu::features (cached)", |b| {
        // First call warms LazyLock
        let _ = nebula_system::cpu::features();
        b.iter(|| black_box(nebula_system::cpu::features()));
    });

    group.bench_function("SystemInfo::get (Arc clone)", |b| {
        // First call warms LazyLock
        let _ = nebula_system::info::SystemInfo::get();
        b.iter(|| black_box(nebula_system::info::SystemInfo::get()));
    });

    group.finish();
}

fn bench_headroom(c: &mut Criterion) {
    use nebula_system::cpu::CpuPressure;
    use nebula_system::load::SystemLoad;
    use nebula_system::memory::MemoryPressure;

    let load = SystemLoad {
        cpu: CpuPressure::Medium,
        memory: MemoryPressure::Low,
        cpu_usage_percent: 55.0,
        memory_usage_percent: 30.0,
    };

    c.bench_function("SystemLoad::headroom", |b| {
        b.iter(|| black_box(load.headroom()));
    });

    c.bench_function("SystemLoad::can_accept_work", |b| {
        b.iter(|| black_box(load.can_accept_work()));
    });
}

criterion_group!(benches, bench_system_load, bench_headroom);
criterion_main!(benches);
