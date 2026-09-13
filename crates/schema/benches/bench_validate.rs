use criterion::{BatchSize, Criterion, black_box};
use nebula_schema::{
    AuthoredValue, Field, FieldKey, LoaderContext, LoaderRegistry, LoaderResult, Predicate, Rule,
    Schema, field_key,
};
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

fn sample_values() -> AuthoredValue {
    let mut values = AuthoredValue::object();
    values
        .insert_data("name", json!("nebula"))
        .expect("test-only known-good key");
    values
        .insert_data("retries", json!(3))
        .expect("test-only known-good key");
    values
        .insert_data("mode", json!("sync"))
        .expect("test-only known-good key");
    values
}

fn bench_validate_static(c: &mut Criterion) {
    let schema = sample_schema();
    let values = sample_values();

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

/// Nested fields exercise preparation and validation across object boundaries.
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

fn nested_values() -> AuthoredValue {
    let mut values = AuthoredValue::object();
    values
        .insert_data(
            "user",
            json!({ "name": "alice", "email": "a@b.com", "age": 30 }),
        )
        .expect("test-only known-good key");
    values
        .insert_data("settings", json!({ "notify": true, "locale": "en-US" }))
        .expect("test-only known-good key");
    values
}

fn bench_validate_nested(c: &mut Criterion) {
    let schema = nested_schema();
    let values = nested_values();

    c.bench_function("schema_validate_nested", |b| {
        b.iter_batched(
            || values.clone(),
            |values| {
                black_box(
                    schema
                        .validate(black_box(values))
                        .expect("valid nested data"),
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn contextual_nested_fixture() -> (nebula_schema::ValidSchema, AuthoredValue) {
    const LEVELS: usize = 16;
    let mut field = Field::boolean(field_key!("enabled")).into_field();
    let mut value = json!({"enabled": true});
    let mut segments = Vec::with_capacity(LEVELS + 1);
    for level in (0..LEVELS).rev() {
        let key = format!("level_{level}");
        segments.push(key.clone());
        field = Field::object(FieldKey::new(&key).expect("valid generated field key"))
            .add(field)
            .into_field();
        value = json!({key: value});
    }
    segments.reverse();
    segments.push("enabled".to_owned());
    let predicate_path = format!("/{}", segments.join("/"));
    let schema = Schema::builder()
        .add(field)
        .root_rule(
            Rule::predicate(
                Predicate::eq(predicate_path, json!(true)).expect("valid generated predicate path"),
            )
            .expect("bounded contextual benchmark rule"),
        )
        .build()
        .expect("valid contextual nested bench schema");
    let values = AuthoredValue::from_data(value).expect("valid contextual nested bench values");
    (schema, values)
}

fn bench_validate_nested_contextual(c: &mut Criterion) {
    let (schema, values) = contextual_nested_fixture();
    c.bench_function("schema_validate_nested_contextual", |b| {
        b.iter_batched(
            || values.clone(),
            |values| {
                black_box(
                    schema
                        .validate(black_box(values))
                        .expect("valid contextual nested data"),
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_loader_rejects_oversized_page(c: &mut Criterion) {
    let schema = Schema::builder()
        .add(Field::dynamic(field_key!("records")).loader("oversized"))
        .build()
        .expect("valid loader bench schema");
    let first = "a".repeat(600_000);
    let second = "b".repeat(600_000);
    let registry = LoaderRegistry::new().register_record("oversized", move |_context| {
        let first = first.clone();
        let second = second.clone();
        async move {
            Ok(LoaderResult::done(vec![
                serde_json::Value::String(first),
                serde_json::Value::String(second),
            ]))
        }
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build benchmark runtime");
    c.bench_function("loader_rejects_oversized_page", |b| {
        b.iter(|| {
            let result = runtime.block_on(schema.load_dynamic_records(
                "records",
                &registry,
                LoaderContext::new("records", AuthoredValue::object()),
            ));
            black_box(result.expect_err("oversized loader page must be rejected"));
        });
    });
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_validate_static(&mut criterion);
    bench_validate_nested(&mut criterion);
    bench_validate_nested_contextual(&mut criterion);
    bench_loader_rejects_oversized_page(&mut criterion);
    criterion.final_summary();
}
