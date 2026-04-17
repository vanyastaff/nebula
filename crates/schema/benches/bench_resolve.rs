use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

fn bench_resolve_literal_only(c: &mut Criterion) {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .add(Field::number(field_key!("score")))
        .build()
        .expect("schema is valid");

    let values = FieldValues::from_json(json!({"name": "hello", "score": 42})).unwrap();
    let valid = schema.validate(&values).expect("values are valid");

    c.bench_function("resolve_literal_only_fast_path", |b| {
        b.iter(|| {
            // Fast path: schema.flags().uses_expressions == false so no walking.
            black_box(&valid);
        });
    });
}

fn bench_validate_static_phase1(c: &mut Criterion) {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(
            Field::number(field_key!("retries"))
                .min(0)
                .max(10)
                .required(),
        )
        .add(
            Field::select(field_key!("mode"))
                .option("sync", "Sync")
                .option("async", "Async"),
        )
        .build()
        .expect("schema is valid");

    let values =
        FieldValues::from_json(json!({"name": "nebula", "retries": 3, "mode": "sync"})).unwrap();

    c.bench_function("schema_validate_static", |b| {
        b.iter(|| {
            let result = schema.validate(black_box(&values));
            let _ = black_box(result);
        });
    });
}

criterion_group!(
    benches,
    bench_resolve_literal_only,
    bench_validate_static_phase1
);
criterion_main!(benches);
