use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{ExecutionMode, Field, FieldValues, Schema};
use serde_json::json;

fn sample_schema() -> Schema {
    Schema::new()
        .add(Field::string("name").required().min_length(2))
        .add(Field::number("retries").min(0).max(10).required())
        .add(
            Field::select("mode")
                .option("sync", "Sync")
                .option("async", "Async"),
        )
}

fn sample_values() -> FieldValues {
    let mut values = FieldValues::new();
    values.set_raw("name", json!("nebula"));
    values.set_raw("retries", json!(3));
    values.set_raw("mode", json!("sync"));
    values
}

fn bench_validate_static(c: &mut Criterion) {
    let schema = sample_schema();
    let values = sample_values();

    c.bench_function("schema_validate_static", |b| {
        b.iter(|| {
            let report = schema.validate(black_box(&values), ExecutionMode::StaticOnly);
            black_box(report);
        })
    });
}

criterion_group!(benches, bench_validate_static);
criterion_main!(benches);
