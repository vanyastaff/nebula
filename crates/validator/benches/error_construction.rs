//! Benchmarks for error construction and allocation paths
//!
//! Measures the cost of creating ValidationError instances across different
//! configurations: bare errors, errors with params, nested errors, and
//! convenience constructors.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_validator::foundation::{ErrorSeverity, ValidationError};

// ============================================================================
// BARE ERROR CONSTRUCTION
// ============================================================================

fn bench_error_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_new");

    // Static strings (zero-alloc Cow::Borrowed)
    group.bench_function("static_strings", |b| {
        b.iter(|| {
            black_box(ValidationError::new("min_length", "String is too short"));
        })
    });

    // Dynamic message (Cow::Owned — allocates)
    group.bench_function("dynamic_message", |b| {
        b.iter(|| {
            black_box(ValidationError::new(
                "min_length",
                format!("Must be at least {} characters", black_box(5)),
            ));
        })
    });

    // With field path (dot-notation → JSON Pointer conversion)
    group.bench_function("with_field_dot", |b| {
        b.iter(|| {
            black_box(ValidationError::new("min_length", "too short").with_field("user.name"));
        })
    });

    // With field path (already JSON Pointer — no conversion)
    group.bench_function("with_field_pointer", |b| {
        b.iter(|| {
            black_box(ValidationError::new("min_length", "too short").with_pointer("/user/name"));
        })
    });

    group.finish();
}

// ============================================================================
// ERROR WITH PARAMS (SmallVec path)
// ============================================================================

fn bench_error_params(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_params");

    // 1 param — triggers ErrorExtras Box allocation + SmallVec inline
    group.bench_function("one_param", |b| {
        b.iter(|| {
            black_box(ValidationError::new("min_length", "too short").with_param("min", "5"));
        })
    });

    // 2 params — still inline in SmallVec<[_;2]>
    group.bench_function("two_params_inline", |b| {
        b.iter(|| {
            black_box(
                ValidationError::new("min_length", "too short")
                    .with_param("min", "5")
                    .with_param("actual", "3"),
            );
        })
    });

    // 3 params — spills to heap (beyond SmallVec<[_;2]> capacity)
    group.bench_function("three_params_spill", |b| {
        b.iter(|| {
            black_box(
                ValidationError::new("out_of_range", "value out of range")
                    .with_param("min", "1")
                    .with_param("max", "100")
                    .with_param("actual", "150"),
            );
        })
    });

    group.finish();
}

// ============================================================================
// CONVENIENCE CONSTRUCTORS (format! + params)
// ============================================================================

fn bench_convenience_constructors(c: &mut Criterion) {
    let mut group = c.benchmark_group("constructors");

    group.bench_function("required", |b| {
        b.iter(|| black_box(ValidationError::required("email")))
    });

    group.bench_function("min_length", |b| {
        b.iter(|| black_box(ValidationError::min_length("name", 3, 1)))
    });

    group.bench_function("max_length", |b| {
        b.iter(|| black_box(ValidationError::max_length("name", 100, 150)))
    });

    group.bench_function("invalid_format", |b| {
        b.iter(|| black_box(ValidationError::invalid_format("email", "email")))
    });

    group.bench_function("out_of_range", |b| {
        b.iter(|| black_box(ValidationError::out_of_range("age", 0, 120, 150)))
    });

    group.bench_function("custom", |b| {
        b.iter(|| black_box(ValidationError::custom("Something went wrong")))
    });

    group.finish();
}

// ============================================================================
// NESTED ERRORS
// ============================================================================

fn bench_nested_errors(c: &mut Criterion) {
    let mut group = c.benchmark_group("nested_errors");

    // Single nested child
    group.bench_function("one_nested", |b| {
        b.iter(|| {
            black_box(
                ValidationError::new("or_failed", "no alternative succeeded")
                    .with_nested_error(ValidationError::new("min_length", "too short")),
            );
        })
    });

    // Three nested children
    group.bench_function("three_nested", |b| {
        b.iter(|| {
            black_box(
                ValidationError::new("or_failed", "no alternative succeeded").with_nested(vec![
                    ValidationError::new("min_length", "too short"),
                    ValidationError::new("max_length", "too long"),
                    ValidationError::new("alphanumeric", "not alphanumeric"),
                ]),
            );
        })
    });

    // With severity + help (full extras)
    group.bench_function("full_extras", |b| {
        b.iter(|| {
            black_box(
                ValidationError::new("min_length", "too short")
                    .with_field("user.name")
                    .with_param("min", "3")
                    .with_param("actual", "1")
                    .with_severity(ErrorSeverity::Error)
                    .with_help("usernames must be at least 3 characters"),
            );
        })
    });

    group.finish();
}

// ============================================================================
// SERIALIZATION
// ============================================================================

fn bench_to_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("to_json");

    let simple = ValidationError::new("min_length", "too short");
    group.bench_function("simple_error", |b| {
        b.iter(|| black_box(simple.to_json_value()))
    });

    let with_params = ValidationError::new("min_length", "too short")
        .with_field("user.name")
        .with_param("min", "3")
        .with_param("actual", "1");
    group.bench_function("with_params", |b| {
        b.iter(|| black_box(with_params.to_json_value()))
    });

    let nested = ValidationError::new("or_failed", "no alternative succeeded").with_nested(vec![
        ValidationError::new("min_length", "too short").with_param("min", "3"),
        ValidationError::new("alphanumeric", "not alphanumeric"),
    ]);
    group.bench_function("with_nested", |b| {
        b.iter(|| black_box(nested.to_json_value()))
    });

    group.finish();
}

// ============================================================================
// MEMORY SIZE ASSERTIONS
// ============================================================================

fn bench_memory_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_layout");

    // Measure drop cost
    group.bench_function("drop_simple", |b| {
        b.iter(|| {
            let err = ValidationError::new("min_length", "too short");
            drop(black_box(err));
        })
    });

    group.bench_function("drop_with_extras", |b| {
        b.iter(|| {
            let err = ValidationError::new("min_length", "too short")
                .with_param("min", "3")
                .with_param("actual", "1");
            drop(black_box(err));
        })
    });

    group.bench_function("clone_simple", |b| {
        let err = ValidationError::new("min_length", "too short");
        b.iter(|| black_box(err.clone()))
    });

    group.bench_function("clone_with_extras", |b| {
        let err = ValidationError::new("min_length", "too short")
            .with_param("min", "3")
            .with_param("actual", "1");
        b.iter(|| black_box(err.clone()))
    });

    group.finish();
}

// ============================================================================
// BENCHMARK GROUPS
// ============================================================================

criterion_group!(
    construction,
    bench_error_new,
    bench_error_params,
    bench_convenience_constructors,
);

criterion_group!(nesting, bench_nested_errors);

criterion_group!(serialization, bench_to_json);

criterion_group!(memory, bench_memory_layout);

criterion_main!(construction, nesting, serialization, memory);
