// Benchmarks for type conversions
//
// Tests performance of TryFrom implementations and ValueConversion trait

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_value::{Array, Bytes, Float, Integer, Object, Text, Value};
use serde_json::json;
use std::convert::TryFrom;

fn bench_try_from_value(c: &mut Criterion) {
    let mut group = c.benchmark_group("try_from_value");

    // Primitive conversions
    let int_val = Value::integer(42);
    group.bench_function("value_to_i64", |b| {
        b.iter(|| i64::try_from(black_box(int_val.clone())).unwrap());
    });

    group.bench_function("value_to_i32", |b| {
        b.iter(|| i32::try_from(black_box(int_val.clone())).unwrap());
    });

    let float_val = Value::float(3.14);
    group.bench_function("value_to_f64", |b| {
        b.iter(|| f64::try_from(black_box(float_val.clone())).unwrap());
    });

    let bool_val = Value::boolean(true);
    group.bench_function("value_to_bool", |b| {
        b.iter(|| bool::try_from(black_box(bool_val.clone())).unwrap());
    });

    let text_val = Value::text("hello");
    group.bench_function("value_to_string", |b| {
        b.iter(|| String::try_from(black_box(text_val.clone())).unwrap());
    });

    let bytes_val = Value::bytes(vec![1, 2, 3]);
    group.bench_function("value_to_vec_u8", |b| {
        b.iter(|| Vec::<u8>::try_from(black_box(bytes_val.clone())).unwrap());
    });

    // Scalar type conversions
    group.bench_function("value_to_integer", |b| {
        b.iter(|| Integer::try_from(black_box(int_val.clone())).unwrap());
    });

    group.bench_function("value_to_float", |b| {
        b.iter(|| Float::try_from(black_box(float_val.clone())).unwrap());
    });

    group.bench_function("value_to_text", |b| {
        b.iter(|| Text::try_from(black_box(text_val.clone())).unwrap());
    });

    group.bench_function("value_to_bytes", |b| {
        b.iter(|| Bytes::try_from(black_box(bytes_val.clone())).unwrap());
    });

    // Collection conversions
    let array_val = Value::Array(Array::from_vec(vec![json!(1), json!(2), json!(3)]));
    group.bench_function("value_to_array", |b| {
        b.iter(|| Array::try_from(black_box(array_val.clone())).unwrap());
    });

    let object_val = Value::Object(Object::from_iter(vec![("key".to_string(), json!(1))]));
    group.bench_function("value_to_object", |b| {
        b.iter(|| Object::try_from(black_box(object_val.clone())).unwrap());
    });

    group.finish();
}

fn bench_from_primitives(c: &mut Criterion) {
    let mut group = c.benchmark_group("from_primitives");

    group.bench_function("i64_to_value", |b| {
        b.iter(|| Value::integer(black_box(42)));
    });

    group.bench_function("f64_to_value", |b| {
        b.iter(|| Value::float(black_box(3.14)));
    });

    group.bench_function("bool_to_value", |b| {
        b.iter(|| Value::boolean(black_box(true)));
    });

    group.bench_function("string_to_value", |b| {
        b.iter(|| Value::text(black_box("hello".to_string())));
    });

    group.bench_function("str_to_value", |b| {
        b.iter(|| Value::text(black_box("hello")));
    });

    group.bench_function("vec_u8_to_value", |b| {
        b.iter(|| Value::bytes(black_box(vec![1, 2, 3])));
    });

    group.finish();
}

fn bench_type_coercion(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_coercion");

    // Integer to Float coercion
    let int = Value::integer(100);
    let float = Value::float(1.5);

    group.bench_function("int_float_add", |b| {
        b.iter(|| int.add(black_box(&float)));
    });

    group.bench_function("float_int_add", |b| {
        b.iter(|| float.add(black_box(&int)));
    });

    group.bench_function("int_float_mul", |b| {
        b.iter(|| int.mul(black_box(&float)));
    });

    // Comparison with coercion
    group.bench_function("int_float_eq", |b| {
        let int_100 = Value::integer(100);
        let float_100 = Value::float(100.0);
        b.iter(|| int_100.eq(black_box(&float_100)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_try_from_value,
    bench_from_primitives,
    bench_type_coercion,
);
criterion_main!(benches);
