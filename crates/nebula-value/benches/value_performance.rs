use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nebula_value::{Value, Array, Object, Bytes, Text};
use std::collections::HashMap;

// ============================================================================
// Core Value Operations
// ============================================================================

fn bench_value_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_creation");

    group.bench_function("null", |b| {
        b.iter(|| black_box(Value::Null))
    });

    group.bench_function("bool", |b| {
        b.iter(|| black_box(Value::bool(true)))
    });

    group.bench_function("int", |b| {
        b.iter(|| black_box(Value::int(42)))
    });

    group.bench_function("float", |b| {
        b.iter(|| black_box(Value::float(3.14)))
    });

    group.bench_function("string_short", |b| {
        b.iter(|| black_box(Value::string("hello")))
    });

    group.bench_function("string_long", |b| {
        let long_str = "a".repeat(1000);
        b.iter(|| black_box(Value::string(&long_str)))
    });

    group.bench_function("bytes_small", |b| {
        let data = vec![1, 2, 3, 4, 5];
        b.iter(|| black_box(Value::Bytes(Bytes::new(data.clone()))))
    });

    group.bench_function("bytes_large", |b| {
        let data = vec![42; 10000];
        b.iter(|| black_box(Value::Bytes(Bytes::new(data.clone()))))
    });

    group.finish();
}

fn bench_value_cloning(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_cloning");

    let values = [
        ("null", Value::Null),
        ("bool", Value::bool(true)),
        ("int", Value::int(42)),
        ("float", Value::float(3.14)),
        ("string_short", Value::string("hello")),
        ("string_long", Value::string(&"a".repeat(1000))),
        ("bytes_small", Value::Bytes(Bytes::new(vec![1, 2, 3, 4, 5]))),
        ("bytes_large", Value::Bytes(Bytes::new(vec![42; 10000]))),
    ];

    for (name, value) in values {
        group.bench_with_input(BenchmarkId::new("clone", name), &value, |b, v| {
            b.iter(|| black_box(v.clone()))
        });
    }

    group.finish();
}

fn bench_value_equality(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_equality");

    let value1 = Value::string("hello world");
    let value2 = Value::string("hello world");
    let value3 = Value::string("goodbye world");

    group.bench_function("string_equal", |b| {
        b.iter(|| black_box(value1 == value2))
    });

    group.bench_function("string_unequal", |b| {
        b.iter(|| black_box(value1 == value3))
    });

    let array1 = Value::Array(Array::new((0..100).map(Value::int).collect()));
    let array2 = Value::Array(Array::new((0..100).map(Value::int).collect()));
    let array3 = Value::Array(Array::new((0..99).map(Value::int).collect()));

    group.bench_function("array_equal", |b| {
        b.iter(|| black_box(array1 == array2))
    });

    group.bench_function("array_unequal", |b| {
        b.iter(|| black_box(array1 == array3))
    });

    group.finish();
}

fn bench_value_hashing(c: &mut Criterion) {
    use std::hash::{Hash, Hasher, DefaultHasher};

    let mut group = c.benchmark_group("value_hashing");

    let values = [
        ("null", Value::Null),
        ("bool", Value::bool(true)),
        ("int", Value::int(42)),
        ("float", Value::float(3.14)),
        ("string", Value::string("hello world")),
        ("array", Value::Array(Array::new((0..100).map(Value::int).collect()))),
        ("bytes", Value::Bytes(Bytes::new(vec![42; 1000]))),
    ];

    for (name, value) in values {
        group.bench_with_input(BenchmarkId::new("hash", name), &value, |b, v| {
            b.iter(|| {
                let mut hasher = DefaultHasher::new();
                v.hash(&mut hasher);
                black_box(hasher.finish())
            })
        });
    }

    group.finish();
}

// ============================================================================
// Collection Operations
// ============================================================================

fn bench_array_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("array_operations");

    // Array creation with different sizes
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("create", size), size, |b, &size| {
            b.iter(|| {
                let vec: Vec<Value> = (0..size).map(Value::int).collect();
                black_box(Array::new(vec))
            })
        });
    }

    // Array access patterns
    let large_array = Array::new((0..1000).map(Value::int).collect());

    group.bench_function("sequential_access", |b| {
        b.iter(|| {
            let mut sum = 0i64;
            for i in 0..large_array.len() {
                if let Some(Value::Int(n)) = large_array.get(i) {
                    sum += n.value();
                }
            }
            black_box(sum)
        })
    });

    group.bench_function("iterator_access", |b| {
        b.iter(|| {
            let sum: i64 = large_array
                .iter()
                .filter_map(|v| v.as_int())
                .sum();
            black_box(sum)
        })
    });

    group.bench_function("push_operations", |b| {
        b.iter(|| {
            let mut arr = Array::new(vec![]);
            for i in 0..100 {
                arr = arr.push(Value::int(i));
            }
            black_box(arr)
        })
    });

    group.finish();
}

fn bench_object_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("object_operations");

    // Object creation with different sizes
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("create", size), size, |b, &size| {
            b.iter(|| {
                let mut obj = Object::new();
                for i in 0..*size {
                    obj = obj.insert(format!("key_{}", i), Value::int(i));
                }
                black_box(obj)
            })
        });
    }

    // Object lookup patterns
    let large_object = (0..1000).fold(Object::new(), |obj, i| {
        obj.insert(format!("key_{}", i), Value::int(i))
    });

    group.bench_function("key_lookup_hit", |b| {
        b.iter(|| {
            for i in (0..1000).step_by(10) {
                let key = format!("key_{}", i);
                black_box(large_object.get(&key));
            }
        })
    });

    group.bench_function("key_lookup_miss", |b| {
        b.iter(|| {
            for i in (0..100).step_by(10) {
                let key = format!("missing_key_{}", i);
                black_box(large_object.get(&key));
            }
        })
    });

    group.bench_function("iteration", |b| {
        b.iter(|| {
            let mut count = 0;
            for (key, value) in large_object.iter() {
                count += key.len() + if let Value::Int(n) = value { n.value() as usize } else { 0 };
            }
            black_box(count)
        })
    });

    group.finish();
}

// ============================================================================
// String Operations
// ============================================================================

fn bench_text_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_operations");

    let short_text = Text::new("hello world".to_string());
    let long_text = Text::new("a".repeat(10000));
    let unicode_text = Text::new("Hello 🌍 世界 Здравствуй мир".to_string());

    group.bench_function("char_count_short", |b| {
        b.iter(|| black_box(short_text.char_count()))
    });

    group.bench_function("char_count_long", |b| {
        b.iter(|| black_box(long_text.char_count()))
    });

    group.bench_function("char_count_unicode", |b| {
        b.iter(|| black_box(unicode_text.char_count()))
    });

    group.bench_function("substring_short", |b| {
        b.iter(|| black_box(short_text.substring(1, 5)))
    });

    group.bench_function("substring_long", |b| {
        b.iter(|| black_box(long_text.substring(100, 200)))
    });

    group.bench_function("contains_short", |b| {
        b.iter(|| black_box(short_text.contains("world")))
    });

    group.bench_function("contains_long", |b| {
        b.iter(|| black_box(long_text.contains("aaaaaaaaaa")))
    });

    group.finish();
}

// ============================================================================
// Bytes Operations
// ============================================================================

fn bench_bytes_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytes_operations");

    let small_bytes = Bytes::new(vec![42; 100]);
    let large_bytes = Bytes::new(vec![42; 10000]);
    let pattern = vec![1, 2, 3, 4, 5];

    group.bench_function("slice_small", |b| {
        b.iter(|| black_box(small_bytes.slice(10, 50)))
    });

    group.bench_function("slice_large", |b| {
        b.iter(|| black_box(large_bytes.slice(1000, 5000)))
    });

    group.bench_function("find_pattern_small", |b| {
        b.iter(|| black_box(small_bytes.find(&pattern)))
    });

    group.bench_function("find_pattern_large", |b| {
        b.iter(|| black_box(large_bytes.find(&pattern)))
    });

    group.bench_function("entropy_small", |b| {
        b.iter(|| black_box(small_bytes.entropy()))
    });

    group.bench_function("entropy_large", |b| {
        b.iter(|| black_box(large_bytes.entropy()))
    });

    #[cfg(feature = "base64")]
    {
        group.bench_function("base64_encode_small", |b| {
            b.iter(|| black_box(small_bytes.to_base64()))
        });

        group.bench_function("base64_encode_large", |b| {
            b.iter(|| black_box(large_bytes.to_base64()))
        });
    }

    group.finish();
}

// ============================================================================
// Serialization Performance
// ============================================================================

#[cfg(feature = "serde")]
fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization");

    let simple_value = Value::Object(Object::new()
        .insert("name".to_string(), Value::string("test"))
        .insert("age".to_string(), Value::int(30))
        .insert("active".to_string(), Value::bool(true)));

    let complex_value = Value::Object(Object::new()
        .insert("users".to_string(), Value::Array(Array::new(
            (0..100).map(|i| Value::Object(Object::new()
                .insert("id".to_string(), Value::int(i))
                .insert("name".to_string(), Value::string(&format!("user_{}", i)))
                .insert("scores".to_string(), Value::Array(Array::new(
                    vec![Value::int(95), Value::int(87), Value::int(92)]
                )))
            )).collect()
        )))
        .insert("metadata".to_string(), Value::Object(Object::new()
            .insert("version".to_string(), Value::int(1))
            .insert("created".to_string(), Value::string("2024-01-01")))));

    group.bench_function("serialize_simple", |b| {
        b.iter(|| black_box(serde_json::to_string(&simple_value).unwrap()))
    });

    group.bench_function("serialize_complex", |b| {
        b.iter(|| black_box(serde_json::to_string(&complex_value).unwrap()))
    });

    let simple_json = serde_json::to_string(&simple_value).unwrap();
    let complex_json = serde_json::to_string(&complex_value).unwrap();

    group.bench_function("deserialize_simple", |b| {
        b.iter(|| {
            let value: Value = serde_json::from_str(&simple_json).unwrap();
            black_box(value)
        })
    });

    group.bench_function("deserialize_complex", |b| {
        b.iter(|| {
            let value: Value = serde_json::from_str(&complex_json).unwrap();
            black_box(value)
        })
    });

    group.finish();
}

// ============================================================================
// Memory Usage Patterns
// ============================================================================

fn bench_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_patterns");

    group.bench_function("deep_clone_array", |b| {
        let array = Value::Array(Array::new(
            (0..1000).map(|i| Value::Array(Array::new(
                (0..10).map(|j| Value::int(i * 10 + j)).collect()
            ))).collect()
        ));

        b.iter(|| black_box(array.clone()))
    });

    group.bench_function("arc_sharing_benefit", |b| {
        let base_array = Array::new((0..1000).map(Value::int).collect());

        b.iter(|| {
            let mut copies = Vec::new();
            for _ in 0..100 {
                copies.push(base_array.clone());
            }
            black_box(copies)
        })
    });

    group.bench_function("hashmap_with_values", |b| {
        b.iter(|| {
            let mut map = HashMap::new();
            for i in 0..1000 {
                map.insert(
                    Value::string(&format!("key_{}", i)),
                    Value::Array(Array::new(vec![Value::int(i), Value::int(i * 2)]))
                );
            }
            black_box(map)
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark Groups
// ============================================================================

criterion_group!(
    value_benches,
    bench_value_creation,
    bench_value_cloning,
    bench_value_equality,
    bench_value_hashing
);

criterion_group!(
    collection_benches,
    bench_array_operations,
    bench_object_operations
);

criterion_group!(
    string_benches,
    bench_text_operations
);

criterion_group!(
    bytes_benches,
    bench_bytes_operations
);

#[cfg(feature = "serde")]
criterion_group!(
    serde_benches,
    bench_serialization
);

criterion_group!(
    memory_benches,
    bench_memory_patterns
);

#[cfg(feature = "serde")]
criterion_main!(
    value_benches,
    collection_benches,
    string_benches,
    bytes_benches,
    serde_benches,
    memory_benches
);

#[cfg(not(feature = "serde"))]
criterion_main!(
    value_benches,
    collection_benches,
    string_benches,
    bytes_benches,
    memory_benches
);