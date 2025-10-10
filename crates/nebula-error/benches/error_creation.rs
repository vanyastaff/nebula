// Baseline benchmarks for NebulaError performance
// Run with: cargo bench

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_error::{ErrorContext, NebulaError};

/// Benchmark creating basic error without context
fn bench_error_creation_simple(c: &mut Criterion) {
    c.bench_function("error_creation_simple", |b| {
        b.iter(|| {
            let error = NebulaError::validation(black_box("Invalid input"));
            black_box(error);
        });
    });
}

/// Benchmark creating error with rich context
fn bench_error_creation_with_context(c: &mut Criterion) {
    c.bench_function("error_creation_with_context", |b| {
        b.iter(|| {
            let context = ErrorContext::new(black_box("Processing user request"))
                .with_user_id(black_box("user123"))
                .with_request_id(black_box("req-456"))
                .with_metadata(black_box("endpoint"), black_box("/api/users"));

            let error = NebulaError::validation(black_box("Invalid input"))
                .with_context(context)
                .with_details(black_box("Email format is incorrect"));

            black_box(error);
        });
    });
}

/// Benchmark cloning errors (important for retry logic)
fn bench_error_clone(c: &mut Criterion) {
    let error = NebulaError::internal("Database connection failed")
        .with_context(
            ErrorContext::new("Database query")
                .with_user_id("user123")
                .with_metadata("table", "users"),
        )
        .with_details("Connection pool exhausted");

    c.bench_function("error_clone", |b| {
        b.iter(|| {
            let cloned = black_box(error.clone());
            black_box(cloned);
        });
    });
}

/// Benchmark error classification checks (hot path)
fn bench_error_classification(c: &mut Criterion) {
    let validation_error = NebulaError::validation("Invalid input");
    let internal_error = NebulaError::internal("Server error");
    let timeout_error = NebulaError::timeout("API call", std::time::Duration::from_secs(30));

    c.bench_function("error_is_retryable", |b| {
        b.iter(|| {
            black_box(validation_error.is_retryable());
            black_box(internal_error.is_retryable());
            black_box(timeout_error.is_retryable());
        });
    });

    c.bench_function("error_classification", |b| {
        b.iter(|| {
            black_box(validation_error.is_client_error());
            black_box(internal_error.is_server_error());
            black_box(timeout_error.is_system_error());
        });
    });
}

/// Benchmark error message access (logging hot path)
fn bench_error_message_access(c: &mut Criterion) {
    let error = NebulaError::internal("Server error").with_details("Detailed error information");

    c.bench_function("error_message_access", |b| {
        b.iter(|| {
            black_box(error.error_code());
            black_box(error.user_message());
            black_box(error.details());
        });
    });
}

/// Benchmark error display formatting
fn bench_error_display(c: &mut Criterion) {
    let error = NebulaError::validation("Invalid email format")
        .with_context(
            ErrorContext::new("User registration")
                .with_user_id("user123")
                .with_metadata("field", "email"),
        )
        .with_details("Must be a valid email address");

    c.bench_function("error_display", |b| {
        b.iter(|| {
            let s = format!("{}", black_box(&error));
            black_box(s);
        });
    });
}

/// Benchmark common error constructors
fn bench_error_constructors(c: &mut Criterion) {
    c.bench_function("constructor_validation", |b| {
        b.iter(|| {
            let error = NebulaError::validation(black_box("Invalid input"));
            black_box(error);
        });
    });

    c.bench_function("constructor_not_found", |b| {
        b.iter(|| {
            let error = NebulaError::not_found(black_box("User"), black_box("123"));
            black_box(error);
        });
    });

    c.bench_function("constructor_internal", |b| {
        b.iter(|| {
            let error = NebulaError::internal(black_box("Database error"));
            black_box(error);
        });
    });

    c.bench_function("constructor_workflow", |b| {
        b.iter(|| {
            let error = NebulaError::workflow_not_found(black_box("user-onboarding"));
            black_box(error);
        });
    });
}

criterion_group!(
    benches,
    bench_error_creation_simple,
    bench_error_creation_with_context,
    bench_error_clone,
    bench_error_classification,
    bench_error_message_access,
    bench_error_display,
    bench_error_constructors,
);

criterion_main!(benches);
