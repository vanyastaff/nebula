use criterion::{BatchSize, Criterion, black_box};
use nebula_schema::{AuthoredValue, Field, Schema, field_key};
use serde_json::json;

fn bench_resolve_literal_only(c: &mut Criterion) {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .add(Field::number(field_key!("score")))
        .build()
        .expect("schema is valid");

    let values = AuthoredValue::from_data(json!({"name": "hello", "score": 42})).unwrap();
    let valid = schema.validate(values).expect("values are valid");
    assert_eq!(
        valid
            .clone()
            .resolve_data()
            .expect("data completes")
            .into_json(),
        json!({"name": "hello", "score": 42})
    );

    c.bench_function("resolve_literal_only", |b| {
        b.iter_batched(
            || valid.clone(),
            |valid| {
                black_box(
                    valid
                        .resolve_data()
                        .expect("data completes without an engine"),
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_validate_static(c: &mut Criterion) {
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
        AuthoredValue::from_data(json!({"name": "nebula", "retries": 3, "mode": "sync"})).unwrap();

    c.bench_function("schema_validate_static", |b| {
        b.iter_batched(
            || values.clone(),
            |values| {
                black_box(
                    schema
                        .validate(black_box(values))
                        .expect("valid static data"),
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_resolve_literal_only(&mut criterion);
    bench_validate_static(&mut criterion);
    criterion.final_summary();
}
