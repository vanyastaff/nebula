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

/// Nested-field bench — exercises the RuleContext win from Task 16 more
/// directly. Phase 0 allocated a fresh `HashMap<String, Value>` on every
/// nested-object descent for predicate rule evaluation; the new walker
/// borrows from the live value tree via `RuleContext`.
fn nested_schema() -> Schema {
    Schema::new()
        .add(
            Field::object("user")
                .add(Field::string("name").required().min_length(2))
                .add(Field::string("email"))
                .add(Field::number("age").min(0).max(120))
                .required(),
        )
        .add(
            Field::object("settings")
                .add(Field::boolean("notify"))
                .add(Field::string("locale")),
        )
}

fn nested_values() -> FieldValues {
    let mut values = FieldValues::new();
    values.set_raw(
        "user",
        json!({ "name": "alice", "email": "a@b.com", "age": 30 }),
    );
    values.set_raw("settings", json!({ "notify": true, "locale": "en-US" }));
    values
}

fn bench_validate_nested(c: &mut Criterion) {
    let schema = nested_schema();
    let values = nested_values();

    c.bench_function("schema_validate_nested", |b| {
        b.iter(|| {
            let report = schema.validate(black_box(&values), ExecutionMode::StaticOnly);
            black_box(report);
        })
    });
}

criterion_group!(benches, bench_validate_static, bench_validate_nested);
criterion_main!(benches);
