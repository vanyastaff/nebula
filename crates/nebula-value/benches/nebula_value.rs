// Comprehensive benchmarks for nebula-value
//
// This file benchmarks all major performance-critical operations

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use nebula_value::{Array, Bytes, Float, Integer, Object, Text, Value};
use serde_json::json;

// ===== INTEGER =====

fn bench_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("integer");
    group.bench_function("create", |b| b.iter(|| Integer::new(black_box(42))));
    group.bench_function("checked_add", |b| {
        let a = Integer::new(100);
        b.iter(|| a.checked_add(black_box(Integer::new(200))));
    });
    group.finish();
}

// ===== FLOAT =====

fn bench_float(c: &mut Criterion) {
    let mut group = c.benchmark_group("float");
    group.bench_function("create", |b| b.iter(|| Float::new(black_box(3.14))));
    group.bench_function("add", |b| {
        let a = Float::new(1.5);
        b.iter(|| a + black_box(Float::new(2.5)));
    });
    group.finish();
}

// ===== TEXT =====

fn bench_text(c: &mut Criterion) {
    let mut group = c.benchmark_group("text");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("create", size), size, |b, &size| {
            let s = "a".repeat(size);
            b.iter(|| Text::new(black_box(s.clone())));
        });

        group.bench_with_input(BenchmarkId::new("clone", size), size, |b, &size| {
            let text = Text::new("a".repeat(size));
            b.iter(|| black_box(&text).clone());
        });
    }

    group.bench_function("concat", |b| {
        let small = Text::from_str("hello");
        b.iter(|| small.concat(black_box(&small)));
    });

    group.finish();
}

// ===== BYTES =====

fn bench_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytes");

    for size in [64, 1024, 65536].iter() {
        group.bench_with_input(BenchmarkId::new("create", size), size, |b, &size| {
            let data = vec![0u8; size];
            b.iter(|| Bytes::new(black_box(data.clone())));
        });

        group.bench_with_input(BenchmarkId::new("clone", size), size, |b, &size| {
            let bytes = Bytes::new(vec![0u8; size]);
            b.iter(|| black_box(&bytes).clone());
        });
    }

    group.finish();
}

// ===== ARRAY =====

fn bench_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("array");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("from_vec", size), size, |b, &size| {
            let items: Vec<_> = (0..size).map(|i| Value::integer(i as i64)).collect();
            b.iter(|| Array::from_vec(black_box(items.clone())));
        });

        group.bench_with_input(BenchmarkId::new("clone", size), size, |b, &size| {
            let items: Vec<_> = (0..size).map(|i| Value::integer(i as i64)).collect();
            let array = Array::from_vec(items);
            b.iter(|| black_box(&array).clone());
        });

        group.bench_with_input(BenchmarkId::new("get", size), size, |b, &size| {
            let items: Vec<_> = (0..size).map(|i| Value::integer(i as i64)).collect();
            let array = Array::from_vec(items);
            let mid = size / 2;
            b.iter(|| black_box(&array).get(black_box(mid)));
        });
    }

    group.bench_function("push", |b| {
        let base = Array::from_vec(vec![Value::integer(1), Value::integer(2), Value::integer(3)]);
        b.iter(|| base.push(black_box(Value::integer(4))));
    });

    group.bench_function("concat", |b| {
        let base = Array::from_vec(vec![Value::integer(1), Value::integer(2), Value::integer(3)]);
        b.iter(|| base.concat(black_box(&base)));
    });

    group.finish();
}

// ===== OBJECT =====

fn bench_object(c: &mut Criterion) {
    let mut group = c.benchmark_group("object");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("from_iter", size), size, |b, &size| {
            let entries: Vec<_> = (0..size).map(|i| (format!("key{}", i), Value::integer(i as i64))).collect();
            b.iter(|| Object::from_iter(black_box(entries.clone())));
        });

        group.bench_with_input(BenchmarkId::new("clone", size), size, |b, &size| {
            let entries: Vec<_> = (0..size).map(|i| (format!("key{}", i), Value::integer(i as i64))).collect();
            let object = Object::from_iter(entries);
            b.iter(|| black_box(&object).clone());
        });

        group.bench_with_input(BenchmarkId::new("get", size), size, |b, &size| {
            let entries: Vec<_> = (0..size).map(|i| (format!("key{}", i), Value::integer(i as i64))).collect();
            let object = Object::from_iter(entries);
            b.iter(|| black_box(&object).get(black_box("key50")));
        });
    }

    group.bench_function("insert", |b| {
        let base = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        b.iter(|| base.insert("c".to_string(), black_box(Value::integer(3))));
    });

    group.bench_function("merge", |b| {
        let base = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        b.iter(|| base.merge(black_box(&base)));
    });

    group.finish();
}

// ===== VALUE OPERATIONS =====

fn bench_value_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_ops");

    // Arithmetic
    let int_a = Value::integer(100);
    let int_b = Value::integer(200);
    group.bench_function("int_add", |b| b.iter(|| int_a.add(black_box(&int_b))));

    let float_a = Value::float(1.5);
    let float_b = Value::float(2.5);
    group.bench_function("float_add", |b| b.iter(|| float_a.add(black_box(&float_b))));

    // Mixed coercion
    let int = Value::integer(100);
    let float = Value::float(1.5);
    group.bench_function("mixed_add", |b| b.iter(|| int.add(black_box(&float))));

    // Text concat
    let text_a = Value::text("hello");
    let text_b = Value::text("world");
    group.bench_function("text_concat", |b| b.iter(|| text_a.add(black_box(&text_b))));

    // Comparison
    group.bench_function("int_eq", |b| b.iter(|| int_a.eq(black_box(&int_b))));

    // Logical
    let true_val = Value::boolean(true);
    let false_val = Value::boolean(false);
    group.bench_function("and", |b| b.iter(|| true_val.and(black_box(&false_val))));
    group.bench_function("not", |b| b.iter(|| true_val.not()));

    // Clone (structural sharing)
    let text_large = Value::text(&"hello".repeat(100));
    group.bench_function("clone_text", |b| b.iter(|| black_box(&text_large).clone()));

    let arr = Value::Array(Array::from_vec((0..1000).map(|i| Value::integer(i)).collect()));
    group.bench_function("clone_array_1000", |b| b.iter(|| black_box(&arr).clone()));

    group.finish();
}

// ===== SERDE =====

#[cfg(feature = "serde")]
fn bench_serde(c: &mut Criterion) {
    let mut group = c.benchmark_group("serde");

    let simple = Value::Object(Object::from_iter(vec![
        ("id".to_string(), Value::integer(1)),
        ("name".to_string(), Value::text("test")),
        ("active".to_string(), Value::boolean(true)),
    ]));

    group.bench_function("serialize_simple", |b| {
        b.iter(|| serde_json::to_string(black_box(&simple)).unwrap());
    });

    let json_str = r#"{"id":1,"name":"test","active":true}"#;
    group.bench_function("deserialize_simple", |b| {
        b.iter(|| serde_json::from_str::<Value>(black_box(json_str)).unwrap());
    });

    let array = Value::Array(Array::from_vec((0..100).map(|i| Value::integer(i)).collect()));
    group.bench_function("serialize_array_100", |b| {
        b.iter(|| serde_json::to_string(black_box(&array)).unwrap());
    });

    group.bench_function("roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&simple)).unwrap();
            let _value: Value = serde_json::from_str(&json).unwrap();
        });
    });

    group.finish();
}

// ===== CRITERION GROUPS =====

criterion_group!(
    benches,
    bench_integer,
    bench_float,
    bench_text,
    bench_bytes,
    bench_array,
    bench_object,
    bench_value_ops,
);

#[cfg(feature = "serde")]
criterion_group!(serde_benches, bench_serde);

#[cfg(feature = "serde")]
criterion_main!(benches, serde_benches);

#[cfg(not(feature = "serde"))]
criterion_main!(benches);
