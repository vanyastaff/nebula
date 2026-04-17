//! Benchmarks for `#[derive(Validator)]` on a realistic wide struct.
//!
//! 20 fields of mixed types (strings, numbers, booleans, collections,
//! optionals) with a representative mix of validation rules.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_validator::{Validator, foundation::Validate};

#[derive(Validator)]
#[validator(message = "record validation failed")]
struct WideRecord {
    #[validate(min_length = 3, max_length = 32, alphanumeric)]
    id: String,

    #[validate(min_length = 1, max_length = 128)]
    name: String,

    #[validate(email)]
    email: String,

    #[validate(url)]
    homepage: String,

    #[validate(uuid)]
    external_id: String,

    #[validate(regex = r"^v\d+\.\d+\.\d+$")]
    version: String,

    #[validate(min = 0u32, max = 10_000u32)]
    quota: u32,

    #[validate(range(min = -180_i32, max = 180_i32))]
    longitude: i32,

    #[validate(range(min = -90_i32, max = 90_i32))]
    latitude: i32,

    #[validate(min = 0.0_f64, max = 1.0_f64)]
    score: f64,

    #[validate(is_true)]
    enabled: bool,

    #[validate(is_false)]
    archived: bool,

    #[validate(min_size = 1, max_size = 20)]
    tags: Vec<String>,

    #[validate(each(min_length = 2, max_length = 24, regex = r"^[a-z][a-z0-9_-]*$"))]
    slugs: Vec<String>,

    #[validate(each(min = 0_i32, max = 100_i32))]
    weights: Vec<i32>,

    #[validate(required, min_length = 1)]
    description: Option<String>,

    #[validate(required, min = 1_u64)]
    created_at: Option<u64>,

    #[validate(each(email))]
    cc: Vec<String>,

    #[validate(size_range(min = 1, max = 5))]
    roles: Vec<String>,

    #[validate(min_size = 0, max_size = 16, each(uuid))]
    related: Vec<String>,
}

fn sample_valid() -> WideRecord {
    WideRecord {
        id: "rec_12345".into(),
        name: "Sample record".into(),
        email: "owner@example.com".into(),
        homepage: "https://example.com".into(),
        external_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        version: "v1.2.3".into(),
        quota: 500,
        longitude: 45,
        latitude: 30,
        score: 0.85,
        enabled: true,
        archived: false,
        tags: vec!["rust".into(), "systems".into()],
        slugs: vec!["core-lib".into(), "bench_target".into()],
        weights: vec![10, 20, 30],
        description: Some("A representative record".into()),
        created_at: Some(1_700_000_000),
        cc: vec!["cc@example.com".into()],
        roles: vec!["admin".into(), "reader".into()],
        related: vec!["550e8400-e29b-41d4-a716-446655440001".into()],
    }
}

fn sample_invalid() -> WideRecord {
    WideRecord {
        id: "!".into(),
        name: "".into(),
        email: "not-an-email".into(),
        homepage: "not a url".into(),
        external_id: "not-a-uuid".into(),
        version: "1.2".into(),
        quota: 20_000,
        longitude: 999,
        latitude: -999,
        score: 2.0,
        enabled: false,
        archived: true,
        tags: vec![],
        slugs: vec!["BAD".into()],
        weights: vec![-1, 200],
        description: None,
        created_at: None,
        cc: vec!["still-not-email".into()],
        roles: vec![],
        related: vec!["nope".into()],
    }
}

fn bench_wide_struct_valid(c: &mut Criterion) {
    let mut group = c.benchmark_group("derive_wide_struct");
    let subject = sample_valid();

    group.bench_function("validate_success", |b| {
        b.iter(|| subject.validate(black_box(&subject)))
    });

    group.bench_function("validate_fields_success", |b| {
        b.iter(|| black_box(&subject).validate_fields())
    });

    group.finish();
}

fn bench_wide_struct_invalid(c: &mut Criterion) {
    let mut group = c.benchmark_group("derive_wide_struct_errors");
    let subject = sample_invalid();

    group.bench_function("validate_collect_all", |b| {
        b.iter(|| black_box(&subject).validate_fields())
    });

    group.finish();
}

criterion_group!(benches, bench_wide_struct_valid, bench_wide_struct_invalid);
criterion_main!(benches);
