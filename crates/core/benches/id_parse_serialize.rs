//! Benchmarks for ID parse/serialize hot paths (P-005).
//!
//! Measures throughput of ID creation, parse, and JSON round-trip.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_core::{ExecutionId, NodeKey, WorkflowId, node_key};

fn id_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("id/new");
    group.bench_function("ExecutionId::new", |b| {
        b.iter(|| black_box(ExecutionId::new()));
    });
    group.bench_function("WorkflowId::new", |b| {
        b.iter(|| black_box(WorkflowId::new()));
    });
    group.bench_function("NodeKey::new", |b| b.iter(|| black_box(node_key!("test"))));
    group.finish();
}

fn id_parse(c: &mut Criterion) {
    // Use actual prefixed ULID strings for parse benchmarks
    let exe_s = ExecutionId::new().to_string();
    let wf_s = WorkflowId::new().to_string();
    let node_s = node_key!("test").to_string();
    let mut group = c.benchmark_group("id/parse");
    group.bench_function("ExecutionId::parse", |b| {
        b.iter(|| {
            black_box(
                exe_s
                    .parse::<ExecutionId>()
                    .expect("pre-generated ExecutionId string must parse"),
            )
        });
    });
    group.bench_function("WorkflowId::parse", |b| {
        b.iter(|| {
            black_box(
                wf_s.parse::<WorkflowId>()
                    .expect("pre-generated WorkflowId string must parse"),
            )
        });
    });
    group.bench_function("NodeKey::parse", |b| {
        b.iter(|| {
            black_box(
                node_s
                    .parse::<NodeKey>()
                    .expect("pre-generated NodeKey string must parse"),
            )
        });
    });
    group.finish();
}

fn id_serde_json_roundtrip(c: &mut Criterion) {
    let id = ExecutionId::new();
    let json = serde_json::to_string(&id).expect("ExecutionId must serialize");
    let mut group = c.benchmark_group("id/serde_json");
    group.bench_function("to_string", |b| {
        b.iter(|| {
            black_box(serde_json::to_string(black_box(&id)).expect("ExecutionId must serialize"))
        });
    });
    group.bench_function("from_str", |b| {
        b.iter(|| {
            let parsed: ExecutionId = serde_json::from_str(black_box(&json))
                .expect("pre-serialized ExecutionId JSON must deserialize");
            black_box(parsed)
        });
    });
    group.bench_function("roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&id))
                .expect("ExecutionId must serialize in roundtrip");
            let parsed: ExecutionId = serde_json::from_str(&json)
                .expect("freshly-serialized ExecutionId must deserialize");
            black_box(parsed)
        });
    });
    group.finish();
}

criterion_group!(benches, id_new, id_parse, id_serde_json_roundtrip);
criterion_main!(benches);
