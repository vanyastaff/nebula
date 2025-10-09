//! Benchmarks for combinator validators
//!
//! Tests performance of:
//! - Basic combinators (And, Or, Not)
//! - Advanced combinators (Map, When, Optional)
//! - Cached combinator with various hit rates
//! - Nested compositions

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nebula_validator::core::{TypedValidator, ValidatorExt};
use nebula_validator::validators::string::*;

// ============================================================================
// BASIC COMBINATORS
// ============================================================================

fn bench_and_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("and_combinator");

    // Two validators
    let validator = min_length(5).and(max_length(20));
    group.bench_function("two_validators_success", |b| {
        b.iter(|| validator.validate(black_box("hello")))
    });

    group.bench_function("two_validators_fail_first", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    group.bench_function("two_validators_fail_second", |b| {
        b.iter(|| validator.validate(black_box("verylongstringthatexceedslimit")))
    });

    // Three validators
    let validator3 = min_length(5).and(max_length(20)).and(alphanumeric());
    group.bench_function("three_validators_success", |b| {
        b.iter(|| validator3.validate(black_box("hello123")))
    });

    // Five validators
    let validator5 = min_length(5)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("h"))
        .and(ends_with("o"));
    group.bench_function("five_validators_success", |b| {
        b.iter(|| validator5.validate(black_box("hello")))
    });

    group.finish();
}

fn bench_or_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("or_combinator");

    let validator = exact_length(5).or(exact_length(10));

    group.bench_function("success_first", |b| {
        b.iter(|| validator.validate(black_box("hello"))) // 5 chars
    });

    group.bench_function("success_second", |b| {
        b.iter(|| validator.validate(black_box("helloworld"))) // 10 chars
    });

    group.bench_function("both_fail", |b| {
        b.iter(|| validator.validate(black_box("hi"))) // 2 chars
    });

    // Multiple options
    let validator_multi = exact_length(5)
        .or(exact_length(10))
        .or(exact_length(15))
        .or(exact_length(20));

    group.bench_function("four_options_success_first", |b| {
        b.iter(|| validator_multi.validate(black_box("hello")))
    });

    group.bench_function("four_options_success_last", |b| {
        b.iter(|| validator_multi.validate(black_box("a".repeat(20))))
    });

    group.bench_function("four_options_all_fail", |b| {
        b.iter(|| validator_multi.validate(black_box("hi")))
    });

    group.finish();
}

fn bench_not_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("not_combinator");

    let validator = contains("admin").not();

    group.bench_function("success_no_match", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    group.bench_function("fail_matches", |b| {
        b.iter(|| validator.validate(black_box("admin user")))
    });

    group.finish();
}

// ============================================================================
// ADVANCED COMBINATORS
// ============================================================================

fn bench_map_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_combinator");

    let validator = min_length(5).map(|_| "Valid!");

    group.bench_function("success", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    group.bench_function("failure", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    group.finish();
}

fn bench_when_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("when_combinator");

    let validator = min_length(10).when(|s: &&str| s.starts_with("long"));

    group.bench_function("condition_true_valid", |b| {
        b.iter(|| validator.validate(black_box("longstring123")))
    });

    group.bench_function("condition_true_invalid", |b| {
        b.iter(|| validator.validate(black_box("longstr")))
    });

    group.bench_function("condition_false_skipped", |b| {
        b.iter(|| validator.validate(black_box("short")))
    });

    group.finish();
}

fn bench_optional_combinator(c: &mut Criterion) {
    let mut group = c.benchmark_group("optional_combinator");

    let validator = min_length(5).optional();

    group.bench_function("some_valid", |b| {
        b.iter(|| validator.validate(black_box(&Some("hello world"))))
    });

    group.bench_function("some_invalid", |b| {
        b.iter(|| validator.validate(black_box(&Some("hi"))))
    });

    group.bench_function("none", |b| {
        b.iter(|| validator.validate(black_box(&None::<&str>)))
    });

    group.finish();
}

// ============================================================================
// CACHED COMBINATOR
// ============================================================================

fn bench_cached_combinator_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("cached_cold");

    // Expensive validator simulation (alphanumeric check is relatively cheap, but we'll use it)
    let validator = alphanumeric().cached();

    group.bench_function("unique_inputs", |b| {
        let mut counter = 0;
        b.iter(|| {
            let input = format!("input{}", counter);
            counter += 1;
            validator.validate(black_box(&input))
        })
    });

    group.finish();
}

fn bench_cached_combinator_hot(c: &mut Criterion) {
    let mut group = c.benchmark_group("cached_hot");

    let validator = alphanumeric().cached();

    // Warm up cache
    for _ in 0..10 {
        let _ = validator.validate("hello123");
    }

    group.bench_function("repeated_input", |b| {
        b.iter(|| validator.validate(black_box("hello123")))
    });

    group.finish();
}

fn bench_cached_hit_rates(c: &mut Criterion) {
    let mut group = c.benchmark_group("cached_hit_rates");

    let validator = alphanumeric().cached_with_capacity(100);

    // 100% hit rate (single repeated input)
    group.bench_function("hit_rate_100", |b| {
        let _ = validator.validate("test");
        b.iter(|| validator.validate(black_box("test")))
    });

    // ~50% hit rate (two inputs alternating)
    group.bench_function("hit_rate_50", |b| {
        let mut toggle = false;
        b.iter(|| {
            toggle = !toggle;
            let input = if toggle { "test1" } else { "test2" };
            validator.validate(black_box(input))
        })
    });

    // ~0% hit rate (always unique)
    group.bench_function("hit_rate_0", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            let input = format!("unique{}", counter);
            counter += 1;
            validator.validate(black_box(&input))
        })
    });

    group.finish();
}

fn bench_cached_capacity(c: &mut Criterion) {
    let mut group = c.benchmark_group("cached_capacity");

    for capacity in [10, 100, 1000, 10000].iter() {
        let validator = alphanumeric().cached_with_capacity(*capacity);

        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            capacity,
            |b, _cap| {
                // Access patterns that fit in cache
                b.iter(|| {
                    for i in 0..10 {
                        let input = format!("test{}", i);
                        validator.validate(black_box(&input));
                    }
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// COMPOSITION DEPTH
// ============================================================================

fn bench_composition_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("composition_depth");

    // Depth 1
    let depth1 = min_length(5);
    group.bench_function("depth_1", |b| {
        b.iter(|| depth1.validate(black_box("hello")))
    });

    // Depth 2
    let depth2 = min_length(5).and(max_length(20));
    group.bench_function("depth_2", |b| {
        b.iter(|| depth2.validate(black_box("hello")))
    });

    // Depth 5
    let depth5 = min_length(5)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("h"))
        .and(ends_with("o"));
    group.bench_function("depth_5", |b| {
        b.iter(|| depth5.validate(black_box("hello")))
    });

    // Depth 10
    let depth10 = min_length(5)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("h"))
        .and(ends_with("o"))
        .and(contains("e"))
        .and(contains("l"))
        .and(not_empty())
        .and(min_length(4))
        .and(max_length(25));
    group.bench_function("depth_10", |b| {
        b.iter(|| depth10.validate(black_box("hello")))
    });

    group.finish();
}

// ============================================================================
// MIXED COMBINATORS
// ============================================================================

fn bench_mixed_combinators(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_combinators");

    // AND + OR
    let and_or = min_length(5)
        .and(max_length(20))
        .or(exact_length(3));
    group.bench_function("and_or_success_left", |b| {
        b.iter(|| and_or.validate(black_box("hello")))
    });

    group.bench_function("and_or_success_right", |b| {
        b.iter(|| and_or.validate(black_box("abc")))
    });

    // Complex: (A AND B) OR (C AND D)
    let complex = min_length(5)
        .and(alphanumeric())
        .or(exact_length(3).and(alphabetic()));
    group.bench_function("complex_success_left", |b| {
        b.iter(|| complex.validate(black_box("hello123")))
    });

    group.bench_function("complex_success_right", |b| {
        b.iter(|| complex.validate(black_box("abc")))
    });

    group.bench_function("complex_fail", |b| {
        b.iter(|| complex.validate(black_box("ab")))
    });

    group.finish();
}

// ============================================================================
// ERROR PATH OVERHEAD
// ============================================================================

fn bench_error_paths(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_paths");

    let validator = min_length(5).and(max_length(20)).and(alphanumeric());

    // Success path (no errors)
    group.bench_function("success_no_error", |b| {
        b.iter(|| validator.validate(black_box("hello123")))
    });

    // Fail fast (first validator fails)
    group.bench_function("fail_fast", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    // Fail late (last validator fails)
    group.bench_function("fail_late", |b| {
        b.iter(|| validator.validate(black_box("hello!")))
    });

    group.finish();
}

// ============================================================================
// REAL WORLD: FORM VALIDATION
// ============================================================================

fn bench_form_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("form_validation");

    // Username validator
    let username = min_length(3)
        .and(max_length(20))
        .and(alphanumeric());

    // Email validator
    let email_val = email();

    // Password validator
    let password = min_length(8).and(max_length(128));

    group.bench_function("username_valid", |b| {
        b.iter(|| username.validate(black_box("alice123")))
    });

    group.bench_function("email_valid", |b| {
        b.iter(|| email_val.validate(black_box("alice@example.com")))
    });

    group.bench_function("password_valid", |b| {
        b.iter(|| password.validate(black_box("SecurePass123!")))
    });

    // Simulate full form validation
    group.bench_function("full_form_valid", |b| {
        b.iter(|| {
            let u = username.validate(black_box("alice123"));
            let e = email_val.validate(black_box("alice@example.com"));
            let p = password.validate(black_box("SecurePass123!"));
            (u.is_ok(), e.is_ok(), p.is_ok())
        })
    });

    group.bench_function("full_form_one_invalid", |b| {
        b.iter(|| {
            let u = username.validate(black_box("al")); // Invalid
            let e = email_val.validate(black_box("alice@example.com"));
            let p = password.validate(black_box("SecurePass123!"));
            (u.is_ok(), e.is_ok(), p.is_ok())
        })
    });

    group.finish();
}

// ============================================================================
// BENCHMARK GROUPS
// ============================================================================

criterion_group!(
    basic_combinators,
    bench_and_combinator,
    bench_or_combinator,
    bench_not_combinator
);

criterion_group!(
    advanced_combinators,
    bench_map_combinator,
    bench_when_combinator,
    bench_optional_combinator
);

criterion_group!(
    cached_combinators,
    bench_cached_combinator_cold,
    bench_cached_combinator_hot,
    bench_cached_hit_rates,
    bench_cached_capacity
);

criterion_group!(
    composition,
    bench_composition_depth,
    bench_mixed_combinators,
    bench_error_paths
);

criterion_group!(
    real_world,
    bench_form_validation
);

criterion_main!(
    basic_combinators,
    advanced_combinators,
    cached_combinators,
    composition,
    real_world
);
