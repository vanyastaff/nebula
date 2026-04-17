use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_schema::{Field, FieldKey, FieldPath, Schema};

fn bench_find_by_path_100_fields(c: &mut Criterion) {
    let mut b = Schema::builder();
    for i in 0..100 {
        let key = FieldKey::new(format!("field_{i}")).unwrap();
        b = b.add(Field::string(key));
    }
    let s = b.build().expect("schema with 100 fields is valid");
    let target = FieldPath::parse("field_42").unwrap();

    c.bench_function("find_by_path_100_fields", |bench| {
        bench.iter(|| {
            black_box(s.find_by_path(&target));
        });
    });
}

fn bench_find_by_key_100_fields(c: &mut Criterion) {
    let mut b = Schema::builder();
    for i in 0..100 {
        let key = FieldKey::new(format!("field_{i}")).unwrap();
        b = b.add(Field::string(key));
    }
    let s = b.build().expect("schema with 100 fields is valid");
    let target = FieldKey::new("field_42").unwrap();

    c.bench_function("find_by_key_100_fields", |bench| {
        bench.iter(|| {
            black_box(s.find(&target));
        });
    });
}

criterion_group!(
    benches,
    bench_find_by_path_100_fields,
    bench_find_by_key_100_fields
);
criterion_main!(benches);
