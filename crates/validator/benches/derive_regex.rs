//! Benchmarks for `#[derive(Validator)] #[validate(regex = "...")]`.
//!
//! Compares the derive-emitted code (regex pre-compiled once via
//! `LazyLock<Regex>`) against a naive baseline that re-compiles the regex
//! on every call. This quantifies the M1 refactor win and guards against
//! regressions.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_validator::{Validator, foundation::Validate};
use regex::Regex;

// ---------------------------------------------------------------------------
// Derive target — regex pre-compiled via LazyLock inside the generated
// `validate_fields` method body.
// ---------------------------------------------------------------------------

#[derive(Validator)]
struct DeriveRegex {
    #[validate(regex = r"^[a-z0-9_]{3,32}$")]
    username: String,
}

// ---------------------------------------------------------------------------
// Baseline — the "old" (pre-M1) style: call `Regex::new(pattern)` inside
// the validate path on every invocation.
// ---------------------------------------------------------------------------

struct NaiveRegex {
    pattern: &'static str,
}

impl NaiveRegex {
    fn validate(&self, input: &str) -> bool {
        // Re-compile per call — this is what the old derive emitted.
        Regex::new(self.pattern).is_ok_and(|re| re.is_match(input))
    }
}

// ---------------------------------------------------------------------------
// Benches
// ---------------------------------------------------------------------------

fn bench_derive_regex_hot(c: &mut Criterion) {
    let mut group = c.benchmark_group("regex_field_hot_path");
    let subject = DeriveRegex {
        username: "alice_42".into(),
    };
    let naive = NaiveRegex {
        pattern: r"^[a-z0-9_]{3,32}$",
    };

    group.bench_function("derive_precompiled", |b| {
        b.iter(|| subject.validate(black_box(&subject)))
    });

    group.bench_function("naive_recompile", |b| {
        b.iter(|| naive.validate(black_box("alice_42")))
    });

    group.finish();
}

fn bench_derive_regex_failure(c: &mut Criterion) {
    let mut group = c.benchmark_group("regex_field_failure_path");
    let subject = DeriveRegex {
        username: "BAD!name".into(),
    };
    let naive = NaiveRegex {
        pattern: r"^[a-z0-9_]{3,32}$",
    };

    group.bench_function("derive_precompiled", |b| {
        b.iter(|| subject.validate(black_box(&subject)))
    });

    group.bench_function("naive_recompile", |b| {
        b.iter(|| naive.validate(black_box("BAD!name")))
    });

    group.finish();
}

criterion_group!(benches, bench_derive_regex_hot, bench_derive_regex_failure);
criterion_main!(benches);
