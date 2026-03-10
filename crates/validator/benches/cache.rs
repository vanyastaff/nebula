//! Benchmarks for the Cached combinator (moka LRU cache)
//!
//! Measures:
//! - Cache hit/miss performance
//! - Hash computation overhead
//! - Capacity impact on lookup speed
//! - Concurrent access patterns

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_validator::foundation::Validate;
use nebula_validator::prelude::*;
use std::hint::black_box;

// ============================================================================
// CACHE HIT / MISS
// ============================================================================

fn bench_cache_hit_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_hit_miss");

    let validator = cached(min_length(5));

    // Prime the cache
    let _ = validator.validate("hello world");
    let _ = validator.validate("hi");

    group.bench_function("hit_success", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    group.bench_function("hit_error", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    // Uncached validator for comparison
    let bare = min_length(5);
    group.bench_function("bare_no_cache_success", |b| {
        b.iter(|| bare.validate(black_box("hello world")))
    });

    group.bench_function("bare_no_cache_error", |b| {
        b.iter(|| bare.validate(black_box("hi")))
    });

    group.finish();
}

// ============================================================================
// CACHE MISS (COLD)
// ============================================================================

fn bench_cache_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_cold");

    // Measure first-call cost (hash + validate + insert)
    group.bench_function("cold_miss_simple", |b| {
        b.iter_batched(
            || cached(min_length(5)),
            |v| v.validate(black_box("hello world")),
            criterion::BatchSize::SmallInput,
        )
    });

    // Cold miss with expensive validator
    let regex_v = matches_regex(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
    group.bench_function("cold_miss_regex", |b| {
        b.iter_batched(
            || cached(regex_v.clone()),
            |v| v.validate(black_box("user@example.com")),
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ============================================================================
// CACHE WITH COMPOSITION
// ============================================================================

fn bench_cached_composition(c: &mut Criterion) {
    let mut group = c.benchmark_group("cached_composition");

    // Cache wrapping a chain of 5 validators
    let chain = min_length(3)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("a"))
        .and(ends_with("e"));

    let cached_chain = cached(chain);
    // Prime the cache
    let _ = cached_chain.validate("alice");

    group.bench_function("cached_chain_5_hit", |b| {
        b.iter(|| cached_chain.validate(black_box("alice")))
    });

    // Without cache for comparison
    let bare_chain = min_length(3)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("a"))
        .and(ends_with("e"));

    group.bench_function("bare_chain_5", |b| {
        b.iter(|| bare_chain.validate(black_box("alice")))
    });

    group.finish();
}

// ============================================================================
// VARYING INPUT SIZES
// ============================================================================

fn bench_cache_input_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_input_sizes");

    let validator = cached(min_length(5));

    for size in [10, 100, 1_000, 10_000] {
        let input: String = "a".repeat(size);
        // Prime the cache
        let _ = validator.validate(&input);

        group.bench_with_input(BenchmarkId::new("hit", size), &input, |b, input| {
            b.iter(|| validator.validate(black_box(input.as_str())))
        });
    }

    group.finish();
}

// ============================================================================
// CACHE CAPACITY IMPACT
// ============================================================================

fn bench_cache_capacity(c: &mut Criterion) {
    use nebula_validator::combinators::cached::Cached;

    let mut group = c.benchmark_group("cache_capacity");

    for capacity in [10, 100, 1000, 10_000] {
        let validator = Cached::with_capacity(min_length(5), capacity);
        // Prime the cache
        let _ = validator.validate("hello world");

        group.bench_with_input(BenchmarkId::new("lookup", capacity), &capacity, |b, _| {
            b.iter(|| validator.validate(black_box("hello world")))
        });
    }

    group.finish();
}

// ============================================================================
// BENCHMARK GROUPS
// ============================================================================

criterion_group!(cache_basics, bench_cache_hit_miss, bench_cache_cold,);

criterion_group!(
    cache_advanced,
    bench_cached_composition,
    bench_cache_input_sizes,
    bench_cache_capacity,
);

criterion_main!(cache_basics, cache_advanced);
