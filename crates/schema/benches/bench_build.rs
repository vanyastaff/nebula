use criterion::{Criterion, black_box};
use nebula_schema::{Field, FieldKey, Schema, ValidSchema};

fn build_sample_schema(field_count: usize) -> ValidSchema {
    let mut builder = Schema::builder();
    for index in 0..field_count {
        let key = format!("field_{index}");
        let key = FieldKey::new(key).expect("generated key should be valid");
        builder = builder.add(Field::string(key).min_length(1).max_length(128));
    }
    builder.build().expect("benchmark schema should build")
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

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_schema_build(&mut criterion);
    criterion.final_summary();
}
