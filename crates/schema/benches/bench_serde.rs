use criterion::{Criterion, black_box};
use nebula_schema::{Field, Schema, field_key};
use serde_json::json;

fn sample_schema() -> Schema {
    serde_json::from_value(json!({
        "fields": [
            Field::string(field_key!("username")).required().min_length(3).into_field(),
            Field::secret(field_key!("api_key")).required().reveal_last(4).into_field(),
            Field::list(field_key!("tags")).item(Field::string(field_key!("tag"))).into_field(),
            Field::object(field_key!("config")).add(Field::boolean(field_key!("enabled"))).into_field(),
            Field::mode(field_key!("auth")).variant(
                "none",
                "None",
                Field::string(field_key!("none")).visible(nebula_schema::VisibilityMode::Never),
            ).into_field()
        ]
    }))
    .expect("benchmark schema should deserialize")
}

fn bench_schema_serde(c: &mut Criterion) {
    let schema = sample_schema();
    c.bench_function("schema_serde_roundtrip", |b| {
        b.iter(|| {
            let encoded = serde_json::to_vec(black_box(&schema)).expect("serialize schema");
            let decoded: Schema = serde_json::from_slice(&encoded).expect("deserialize schema");
            black_box(decoded);
        });
    });
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_schema_serde(&mut criterion);
    criterion.final_summary();
}
