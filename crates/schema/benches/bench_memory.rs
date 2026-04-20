use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{Field, FieldKey, Schema};

fn schema_with_fields(field_count: usize) -> Schema {
    let mut schema = Schema::new();
    for index in 0..field_count {
        let key = format!("k_{index}");
        let key = FieldKey::new(key).expect("generated key should be valid");
        schema = schema.add(Field::string(key).required().min_length(1));
    }
    schema
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

criterion_group!(benches, bench_schema_clone_memory);
criterion_main!(benches);
