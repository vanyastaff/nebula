use criterion::{Criterion, black_box};
use nebula_schema::{Field, FieldKey, Schema};
use serde_json::json;

fn schema_with_fields(field_count: usize) -> Schema {
    let mut fields = Vec::with_capacity(field_count);
    for index in 0..field_count {
        let key = format!("k_{index}");
        let key = FieldKey::new(key).expect("generated key should be valid");
        fields.push(Field::string(key).required().min_length(1).into_field());
    }
    serde_json::from_value(json!({ "fields": fields }))
        .expect("benchmark schema should deserialize")
}

fn bench_schema_clone_memory(c: &mut Criterion) {
    let schema = schema_with_fields(200);
    c.bench_function("schema_clone_200_fields", |b| {
        b.iter(|| {
            let cloned = black_box(&schema).clone();
            black_box(cloned.len());
        });
    });
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_schema_clone_memory(&mut criterion);
    criterion.final_summary();
}
