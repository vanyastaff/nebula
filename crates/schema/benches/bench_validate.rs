use criterion::{Criterion, black_box};
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

fn sample_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(Field::string(field_key!("name")).required().min_length(2))
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
        .expect("valid bench schema")
}

fn sample_values() -> FieldValues {
    let mut values = FieldValues::new();
    values
        .try_set_raw("name", json!("nebula"))
        .expect("test-only known-good key");
    values
        .try_set_raw("retries", json!(3))
        .expect("test-only known-good key");
    values
        .try_set_raw("mode", json!("sync"))
        .expect("test-only known-good key");
    values
}

fn bench_validate_static(c: &mut Criterion) {
    let schema = sample_schema();
    let values = sample_values();

    c.bench_function("schema_validate_static", |b| {
        b.iter(|| {
            let result = schema.validate(black_box(&values));
            let _ = black_box(result);
        });
    });
}

/// Nested-field bench — exercises the `RuleContext` win from Task 16 more
/// directly. Phase 0 allocated a fresh `HashMap<String, Value>` on every
/// nested-object descent for predicate rule evaluation; the new walker
/// borrows from the live value tree via `RuleContext`.
fn nested_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::object(field_key!("user"))
                .add(Field::string(field_key!("name")).required().min_length(2))
                .add(Field::string(field_key!("email")))
                .add(Field::number(field_key!("age")).min(0).max(120))
                .required(),
        )
        .add(
            Field::object(field_key!("settings"))
                .add(Field::boolean(field_key!("notify")))
                .add(Field::string(field_key!("locale"))),
        )
        .build()
        .expect("valid nested bench schema")
}

fn nested_values() -> FieldValues {
    let mut values = FieldValues::new();
    values
        .try_set_raw(
            "user",
            json!({ "name": "alice", "email": "a@b.com", "age": 30 }),
        )
        .expect("test-only known-good key");
    values
        .try_set_raw("settings", json!({ "notify": true, "locale": "en-US" }))
        .expect("test-only known-good key");
    values
}

fn bench_validate_nested(c: &mut Criterion) {
    let schema = nested_schema();
    let values = nested_values();

    c.bench_function("schema_validate_nested", |b| {
        b.iter(|| {
            let result = schema.validate(black_box(&values));
            let _ = black_box(result);
        });
    });
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_validate_static(&mut criterion);
    bench_validate_nested(&mut criterion);
    criterion.final_summary();
}
