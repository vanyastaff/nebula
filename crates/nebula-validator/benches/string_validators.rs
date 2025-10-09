//! Benchmarks for string validators
//!
//! Tests performance of various string validation operations including:
//! - Length validators (MinLength, MaxLength, ExactLength, LengthRange)
//! - Pattern validators (Contains, StartsWith, EndsWith, Alphanumeric)
//! - Content validators (Email, URL)
//! - Unicode handling

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nebula_validator::core::TypedValidator;
use nebula_validator::validators::string::*;

// ============================================================================
// LENGTH VALIDATORS
// ============================================================================

fn bench_min_length(c: &mut Criterion) {
    let mut group = c.benchmark_group("min_length");
    let validator = min_length(5);

    // Valid input (fast path)
    group.bench_function("valid_short", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    // Invalid input (error path)
    group.bench_function("invalid", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    // Edge case: exactly minimum
    group.bench_function("exact_minimum", |b| {
        b.iter(|| validator.validate(black_box("hello")))
    });

    group.finish();
}

fn bench_min_length_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("min_length_scaling");
    let validator = min_length(5);

    for size in [10, 100, 1_000, 10_000, 100_000].iter() {
        let input: String = "a".repeat(*size);
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| validator.validate(black_box(&input)))
        });
    }

    group.finish();
}

fn bench_max_length(c: &mut Criterion) {
    let mut group = c.benchmark_group("max_length");
    let validator = max_length(100);

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    group.bench_function("invalid", |b| {
        let long_string = "a".repeat(150);
        b.iter(|| validator.validate(black_box(&long_string)))
    });

    group.bench_function("exact_maximum", |b| {
        let exact = "a".repeat(100);
        b.iter(|| validator.validate(black_box(&exact)))
    });

    group.finish();
}

fn bench_exact_length(c: &mut Criterion) {
    let mut group = c.benchmark_group("exact_length");
    let validator = exact_length(10);

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("helloworld")))
    });

    group.bench_function("too_short", |b| {
        b.iter(|| validator.validate(black_box("hello")))
    });

    group.bench_function("too_long", |b| {
        b.iter(|| validator.validate(black_box("hello world is long")))
    });

    group.finish();
}

fn bench_length_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("length_range");
    let validator = length_range(5, 20);

    group.bench_function("valid_middle", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    group.bench_function("valid_min", |b| {
        b.iter(|| validator.validate(black_box("hello")))
    });

    group.bench_function("valid_max", |b| {
        let s = "a".repeat(20);
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.bench_function("invalid_too_short", |b| {
        b.iter(|| validator.validate(black_box("hi")))
    });

    group.bench_function("invalid_too_long", |b| {
        let s = "a".repeat(50);
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.finish();
}

fn bench_not_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("not_empty");
    let validator = not_empty();

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("hello")))
    });

    group.bench_function("invalid_empty", |b| {
        b.iter(|| validator.validate(black_box("")))
    });

    group.finish();
}

// ============================================================================
// PATTERN VALIDATORS
// ============================================================================

fn bench_contains(c: &mut Criterion) {
    let mut group = c.benchmark_group("contains");
    let validator = contains("@");

    group.bench_function("found_early", |b| {
        b.iter(|| validator.validate(black_box("@hello")))
    });

    group.bench_function("found_middle", |b| {
        b.iter(|| validator.validate(black_box("hello@world")))
    });

    group.bench_function("found_late", |b| {
        let s = "a".repeat(100) + "@";
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.bench_function("not_found", |b| {
        let s = "a".repeat(100);
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.finish();
}

fn bench_starts_with(c: &mut Criterion) {
    let mut group = c.benchmark_group("starts_with");
    let validator = starts_with("http");

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("https://example.com")))
    });

    group.bench_function("invalid", |b| {
        b.iter(|| validator.validate(black_box("ftp://example.com")))
    });

    group.finish();
}

fn bench_ends_with(c: &mut Criterion) {
    let mut group = c.benchmark_group("ends_with");
    let validator = ends_with(".com");

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("example.com")))
    });

    group.bench_function("invalid", |b| {
        b.iter(|| validator.validate(black_box("example.org")))
    });

    group.finish();
}

fn bench_alphanumeric(c: &mut Criterion) {
    let mut group = c.benchmark_group("alphanumeric");
    let validator = alphanumeric();

    group.bench_function("valid_short", |b| {
        b.iter(|| validator.validate(black_box("abc123")))
    });

    group.bench_function("valid_long", |b| {
        let s = "a".repeat(1000);
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.bench_function("invalid_early", |b| {
        b.iter(|| validator.validate(black_box("!abc123")))
    });

    group.bench_function("invalid_late", |b| {
        let s = "a".repeat(100) + "!";
        b.iter(|| validator.validate(black_box(&s)))
    });

    group.finish();
}

fn bench_alphabetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("alphabetic");
    let validator = alphabetic();

    group.bench_function("valid", |b| {
        b.iter(|| validator.validate(black_box("HelloWorld")))
    });

    group.bench_function("invalid_with_digits", |b| {
        b.iter(|| validator.validate(black_box("Hello123")))
    });

    group.finish();
}

// ============================================================================
// CONTENT VALIDATORS
// ============================================================================

fn bench_email(c: &mut Criterion) {
    let mut group = c.benchmark_group("email");
    let validator = email();

    group.bench_function("valid_simple", |b| {
        b.iter(|| validator.validate(black_box("user@example.com")))
    });

    group.bench_function("valid_complex", |b| {
        b.iter(|| validator.validate(black_box("user.name+tag@sub.example.co.uk")))
    });

    group.bench_function("invalid_no_at", |b| {
        b.iter(|| validator.validate(black_box("userexample.com")))
    });

    group.bench_function("invalid_no_domain", |b| {
        b.iter(|| validator.validate(black_box("user@")))
    });

    group.bench_function("invalid_format", |b| {
        b.iter(|| validator.validate(black_box("not an email")))
    });

    group.finish();
}

fn bench_url(c: &mut Criterion) {
    let mut group = c.benchmark_group("url");
    let validator = url();

    group.bench_function("valid_http", |b| {
        b.iter(|| validator.validate(black_box("http://example.com")))
    });

    group.bench_function("valid_https", |b| {
        b.iter(|| validator.validate(black_box("https://example.com/path?query=value")))
    });

    group.bench_function("invalid_no_scheme", |b| {
        b.iter(|| validator.validate(black_box("example.com")))
    });

    group.bench_function("invalid_format", |b| {
        b.iter(|| validator.validate(black_box("not a url")))
    });

    group.finish();
}

// ============================================================================
// UNICODE HANDLING
// ============================================================================

fn bench_unicode_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("unicode");
    let validator = min_length(5);

    // ASCII only
    group.bench_function("ascii", |b| {
        b.iter(|| validator.validate(black_box("hello world")))
    });

    // Latin extended
    group.bench_function("latin_extended", |b| {
        b.iter(|| validator.validate(black_box("héllo wörld")))
    });

    // Cyrillic
    group.bench_function("cyrillic", |b| {
        b.iter(|| validator.validate(black_box("привет мир")))
    });

    // Chinese
    group.bench_function("chinese", |b| {
        b.iter(|| validator.validate(black_box("你好世界")))
    });

    // Emoji
    group.bench_function("emoji", |b| {
        b.iter(|| validator.validate(black_box("👋🌍🚀💻🎉")))
    });

    // Mixed
    group.bench_function("mixed", |b| {
        b.iter(|| validator.validate(black_box("Hello мир 世界 🌍")))
    });

    group.finish();
}

// ============================================================================
// COMPOSITION
// ============================================================================

fn bench_composition(c: &mut Criterion) {
    use nebula_validator::core::ValidatorExt;

    let mut group = c.benchmark_group("composition");

    // Single validator
    let single = min_length(5);
    group.bench_function("single_validator", |b| {
        b.iter(|| single.validate(black_box("hello")))
    });

    // Two validators
    let double = min_length(5).and(max_length(20));
    group.bench_function("two_validators", |b| {
        b.iter(|| double.validate(black_box("hello")))
    });

    // Three validators
    let triple = min_length(5).and(max_length(20)).and(alphanumeric());
    group.bench_function("three_validators", |b| {
        b.iter(|| triple.validate(black_box("hello")))
    });

    // Five validators (complex composition)
    let complex = min_length(5)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("h"))
        .and(ends_with("o"));
    group.bench_function("five_validators", |b| {
        b.iter(|| complex.validate(black_box("hello")))
    });

    group.finish();
}

// ============================================================================
// EARLY TERMINATION
// ============================================================================

fn bench_early_termination(c: &mut Criterion) {
    use nebula_validator::core::ValidatorExt;

    let mut group = c.benchmark_group("early_termination");

    let validator = min_length(5).and(max_length(20)).and(alphanumeric());

    // Fail on first validator
    group.bench_function("fail_first", |b| {
        b.iter(|| validator.validate(black_box("hi"))) // Too short
    });

    // Fail on second validator
    group.bench_function("fail_second", |b| {
        b.iter(|| validator.validate(black_box("a".repeat(30)))) // Too long
    });

    // Fail on third validator
    group.bench_function("fail_third", |b| {
        b.iter(|| validator.validate(black_box("hello!"))) // Not alphanumeric
    });

    // Success (all validators pass)
    group.bench_function("success_all", |b| {
        b.iter(|| validator.validate(black_box("hello123")))
    });

    group.finish();
}

// ============================================================================
// REAL WORLD: USERNAME VALIDATION
// ============================================================================

fn bench_username_validation(c: &mut Criterion) {
    use nebula_validator::core::ValidatorExt;

    let mut group = c.benchmark_group("username_validation");

    let validator = min_length(3)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with("a")); // Must start with letter (simplified)

    group.bench_function("valid_short", |b| {
        b.iter(|| validator.validate(black_box("alice")))
    });

    group.bench_function("valid_long", |b| {
        b.iter(|| validator.validate(black_box("alicewonderland123")))
    });

    group.bench_function("invalid_too_short", |b| {
        b.iter(|| validator.validate(black_box("al")))
    });

    group.bench_function("invalid_special_char", |b| {
        b.iter(|| validator.validate(black_box("alice@123")))
    });

    group.finish();
}

// ============================================================================
// BENCHMARK GROUPS
// ============================================================================

criterion_group!(
    length_benches,
    bench_min_length,
    bench_min_length_scaling,
    bench_max_length,
    bench_exact_length,
    bench_length_range,
    bench_not_empty
);

criterion_group!(
    pattern_benches,
    bench_contains,
    bench_starts_with,
    bench_ends_with,
    bench_alphanumeric,
    bench_alphabetic
);

criterion_group!(
    content_benches,
    bench_email,
    bench_url
);

criterion_group!(
    unicode_benches,
    bench_unicode_handling
);

criterion_group!(
    composition_benches,
    bench_composition,
    bench_early_termination,
    bench_username_validation
);

criterion_main!(
    length_benches,
    pattern_benches,
    content_benches,
    unicode_benches,
    composition_benches
);
