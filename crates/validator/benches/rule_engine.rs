//! Benchmarks for the declarative [`Rule`] engine.
//!
//! Covers `validate_rules` under different mixes: value-only, deferred,
//! combinators, and large rule sets, across `ExecutionMode` variants.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_validator::{ExecutionMode, Rule, validate_rules};
use serde_json::json;

fn small_value_ruleset() -> Vec<Rule> {
    vec![
        Rule::min_length(3),
        Rule::max_length(32),
        Rule::pattern(r"^[a-z0-9_]+$"),
    ]
}

fn mixed_ruleset() -> Vec<Rule> {
    vec![
        Rule::min_length(3),
        Rule::pattern(r"^[a-z]+$"),
        Rule::custom("skipped"),
        Rule::unique_by("id").unwrap(),
        Rule::all([Rule::min_length(1), Rule::max_length(64)]),
    ]
}

fn combinator_ruleset() -> Vec<Rule> {
    vec![Rule::all([
        Rule::any([Rule::min_length(10), Rule::max_length(3)]),
        Rule::not(Rule::pattern(r"[0-9]")),
    ])]
}

fn large_ruleset(n: usize) -> Vec<Rule> {
    (0..n).map(|i| Rule::min_length(i % 5)).collect()
}

fn bench_value_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("rule_engine_value");
    let rules = small_value_ruleset();
    let value = json!("alice_42");

    group.bench_function("static_only", |b| {
        b.iter(|| {
            validate_rules(
                black_box(&value),
                black_box(&rules),
                ExecutionMode::StaticOnly,
            )
        })
    });

    group.bench_function("full_mode", |b| {
        b.iter(|| validate_rules(black_box(&value), black_box(&rules), ExecutionMode::Full))
    });

    group.finish();
}

fn bench_mixed_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("rule_engine_mixed");
    let rules = mixed_ruleset();
    let value = json!("alice");

    for mode in [
        ExecutionMode::StaticOnly,
        ExecutionMode::Deferred,
        ExecutionMode::Full,
    ] {
        let label = format!("{mode:?}");
        group.bench_function(label, |b| {
            b.iter(|| validate_rules(black_box(&value), black_box(&rules), mode))
        });
    }

    group.finish();
}

fn bench_combinator_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("rule_engine_combinator");
    let rules = combinator_ruleset();

    group.bench_function("depth_3_passes", |b| {
        let v = json!("abcdefghij");
        b.iter(|| validate_rules(black_box(&v), black_box(&rules), ExecutionMode::StaticOnly))
    });

    group.bench_function("depth_3_fails", |b| {
        let v = json!("hello5");
        b.iter(|| validate_rules(black_box(&v), black_box(&rules), ExecutionMode::StaticOnly))
    });

    group.finish();
}

fn bench_large_ruleset(c: &mut Criterion) {
    let mut group = c.benchmark_group("rule_engine_scaling");
    let value = json!("anything");

    for &size in &[10usize, 100, 1_000] {
        let rules = large_ruleset(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                validate_rules(
                    black_box(&value),
                    black_box(&rules),
                    ExecutionMode::StaticOnly,
                )
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_value_rules,
    bench_mixed_rules,
    bench_combinator_rules,
    bench_large_ruleset
);
criterion_main!(benches);
