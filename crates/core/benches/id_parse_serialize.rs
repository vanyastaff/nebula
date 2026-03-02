//! Benchmarks for ID parse/serialize hot paths (P-005).
//!
//! Measures throughput of ID creation, parse, and JSON round-trip.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_core::{ExecutionId, NodeId, WorkflowId};

fn id_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("id/new");
    group.bench_function("ExecutionId::new", |b| {
        b.iter(|| black_box(ExecutionId::new()));
    });
    group.bench_function("WorkflowId::new", |b| {
        b.iter(|| black_box(WorkflowId::new()));
    });
    group.bench_function("NodeId::new", |b| b.iter(|| black_box(NodeId::new())));
    group.finish();
}

fn id_parse(c: &mut Criterion) {
    let s = "550e8400-e29b-41d4-a716-446655440000";
    let mut group = c.benchmark_group("id/parse");
    group.bench_function("ExecutionId::parse", |b| {
        b.iter(|| black_box(ExecutionId::parse(s).unwrap()));
    });
    group.bench_function("WorkflowId::parse", |b| {
        b.iter(|| black_box(WorkflowId::parse(s).unwrap()));
    });
    group.bench_function("NodeId::parse", |b| {
        b.iter(|| black_box(NodeId::parse(s).unwrap()));
    });
    group.finish();
}

fn id_serde_json_roundtrip(c: &mut Criterion) {
    let id = ExecutionId::new();
    let json = serde_json::to_string(&id).unwrap();
    let mut group = c.benchmark_group("id/serde_json");
    group.bench_function("to_string", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&id)).unwrap()));
    });
    group.bench_function("from_str", |b| {
        b.iter(|| {
            let parsed: ExecutionId = serde_json::from_str(black_box(&json)).unwrap();
            black_box(parsed)
        });
    });
    group.bench_function("roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&id)).unwrap();
            let parsed: ExecutionId = serde_json::from_str(&json).unwrap();
            black_box(parsed)
        });
    });
    group.finish();
}

criterion_group!(benches, id_new, id_parse, id_serde_json_roundtrip);
criterion_main!(benches);
