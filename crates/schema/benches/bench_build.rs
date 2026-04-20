use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{Field, FieldKey, Schema};

fn build_sample_schema(field_count: usize) -> Schema {
    let mut schema = Schema::new();
    for index in 0..field_count {
        let key = format!("field_{index}");
        let key = FieldKey::new(key).expect("generated key should be valid");
        schema = schema.add(Field::string(key).min_length(1).max_length(128));
    }
    schema
}

fn bench_schema_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema_build");
    for size in [10_usize, 50, 100, 500] {
        group.bench_function(format!("build_{size}"), |b| {
            b.iter(|| build_sample_schema(black_box(size)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_schema_build);
criterion_main!(benches);
