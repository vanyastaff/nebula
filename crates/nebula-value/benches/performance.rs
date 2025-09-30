use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_value::{Value, Array, Object};
use std::collections::HashMap;

/// Benchmark JSON roundtrip performance
fn bench_json_roundtrip(c: &mut Criterion) {
    // Create a complex nested value with 1000 simple objects
    let mut values = Vec::new();
    for i in 0..1000 {
        let mut obj = HashMap::new();
        obj.insert("id".to_string(), Value::from(i));
        obj.insert("name".to_string(), Value::from(format!("user_{}", i)));
        obj.insert("active".to_string(), Value::from(i % 2 == 0));
        obj.insert("score".to_string(), Value::from(i as f64 * 0.5));
        values.push(Value::from(obj));
    }
    let array_value = Value::from(values);

    c.bench_function("json_roundtrip_1k_objects", |b| {
        b.iter(|| {
            // Serialize to JSON
            let json = serde_json::to_string(&array_value).unwrap();
            // Deserialize back
            let value: Value = serde_json::from_str(&json).unwrap();
            black_box(value)
        })
    });
}

/// Benchmark deep path access
fn bench_path_access(c: &mut Criterion) {
    // Create a deeply nested object
    let mut root = HashMap::new();
    let mut current = &mut root;

    // Create 10 levels deep nesting
    for i in 0..10 {
        let mut next = HashMap::new();
        if i == 9 {
            // Last level - add some data
            next.insert("data".to_string(), Value::from(42));
            next.insert("name".to_string(), Value::from("deep_value"));
        }
        current.insert(format!("level_{}", i), Value::from(next.clone()));
        // This is a bit awkward but works for the benchmark setup
    }

    let value = Value::from(root);
    let path = "level_0.level_1.level_2.level_3.level_4.level_5.level_6.level_7.level_8.level_9.data";

    c.bench_function("deep_path_access", |b| {
        b.iter(|| {
            black_box(value.get_path(path))
        })
    });
}

/// Benchmark array operations
fn bench_array_operations(c: &mut Criterion) {
    // Create array with 10k elements
    let values: Vec<Value> = (0..10_000).map(|i| Value::from(i)).collect();
    let array = Array::new(values);

    c.bench_function("array_iteration_10k", |b| {
        b.iter(|| {
            let sum: i64 = array.iter()
                .filter_map(|v| v.as_int())
                .sum();
            black_box(sum)
        })
    });

    c.bench_function("array_get_random_access", |b| {
        b.iter(|| {
            // Access random elements
            for i in (0..1000).step_by(100) {
                black_box(array.get(i));
            }
        })
    });
}

/// Benchmark object operations
fn bench_object_operations(c: &mut Criterion) {
    // Create object with 1k key-value pairs
    let mut map = HashMap::with_capacity(1000);
    for i in 0..1000 {
        map.insert(format!("key_{}", i), Value::from(i));
    }
    let object = Object::from_map(map.into_iter().collect());

    c.bench_function("object_iteration_1k", |b| {
        b.iter(|| {
            let sum: i64 = object.values_iter()
                .filter_map(|v| v.as_int())
                .sum();
            black_box(sum)
        })
    });

    c.bench_function("object_get_random_access", |b| {
        b.iter(|| {
            // Access random keys
            for i in (0..100).step_by(10) {
                black_box(object.get(&format!("key_{}", i)));
            }
        })
    });
}

/// Benchmark memory allocation patterns
fn bench_allocations(c: &mut Criterion) {
    c.bench_function("array_with_capacity_vs_push", |b| {
        b.iter(|| {
            // Using with_capacity should be faster
            let mut array = Array::with_capacity(1000);
            let mut values = Vec::new();
            for i in 0..1000 {
                values.push(Value::from(i));
            }
            black_box(Array::new(values))
        })
    });

    c.bench_function("object_with_capacity_vs_insert", |b| {
        b.iter(|| {
            // Using with_capacity should be faster
            let mut map = HashMap::with_capacity(1000);
            for i in 0..1000 {
                map.insert(format!("key_{}", i), Value::from(i));
            }
            black_box(Object::from_map(map.into_iter().collect()))
        })
    });
}

criterion_group!(
    benches,
    bench_json_roundtrip,
    bench_path_access,
    bench_array_operations,
    bench_object_operations,
    bench_allocations
);
criterion_main!(benches);