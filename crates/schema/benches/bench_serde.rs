use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{Field, Schema};

fn sample_schema() -> Schema {
    Schema::new()
        .add(Field::string("username").required().min_length(3))
        .add(Field::secret("api_key").required().reveal_last(4))
        .add(Field::list("tags").item(Field::string("tag")))
        .add(Field::object("config").add(Field::boolean("enabled")))
        .add(Field::mode("auth").variant("none", "None", Field::hidden("none")))
}

fn bench_schema_serde(c: &mut Criterion) {
    let schema = sample_schema();
    c.bench_function("schema_serde_roundtrip", |b| {
        b.iter(|| {
            let encoded = serde_json::to_vec(black_box(&schema)).expect("serialize schema");
            let decoded: Schema = serde_json::from_slice(&encoded).expect("deserialize schema");
            black_box(decoded);
        })
    });
}

criterion_group!(benches, bench_schema_serde);
criterion_main!(benches);
