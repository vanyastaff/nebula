// Baseline benchmarks for NebulaError performance
// Run with: cargo bench

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_error::{ErrorContext, NebulaError, ErrorKind, kinds::ClientError};
use std::time::Duration;

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

/// Benchmark static vs dynamic error creation (optimization comparison)
fn bench_error_creation_static_vs_dynamic(c: &mut Criterion) {
    c.bench_function("error_creation_static", |b| {
        b.iter(|| {
            let error = NebulaError::new_static(
                ErrorKind::Client(ClientError::Validation {
                    message: "Invalid input".to_string(),
                }),
                "Invalid input"
            );
            black_box(error);
        });
    });

    c.bench_function("error_creation_dynamic", |b| {
        b.iter(|| {
            let error = NebulaError::validation(black_box("Invalid input"));
            black_box(error);
        });
    });
}

/// Benchmark error code access (now method call vs field access)
fn bench_error_code_access(c: &mut Criterion) {
    let error = NebulaError::validation("Invalid input");

    c.bench_function("error_code_access", |b| {
        b.iter(|| {
            black_box(error.error_code());
        });
    });
}

/// Benchmark macro-based error creation
fn bench_macro_error_creation(c: &mut Criterion) {
    use nebula_error::{validation_error, internal_error, not_found_error};

    c.bench_function("macro_validation_error", |b| {
        b.iter(|| {
            let error = validation_error!("Invalid input");
            black_box(error);
        });
    });

    c.bench_function("macro_internal_error", |b| {
        b.iter(|| {
            let error = internal_error!("Database error");
            black_box(error);
        });
    });

    c.bench_function("macro_not_found_error", |b| {
        b.iter(|| {
            let error = not_found_error!("User", "123");
            black_box(error);
        });
    });
}

/// Benchmark error serialization (for network transfer)
fn bench_error_serialization(c: &mut Criterion) {
    let error = NebulaError::validation("Invalid input")
        .with_context(
            ErrorContext::new("Processing request")
                .with_user_id("user123")
                .with_request_id("req456")
        );

    c.bench_function("error_serialization", |b| {
        b.iter(|| {
            let serialized = bincode::serialize(&error).unwrap();
            black_box(serialized);
        });
    });

    let serialized = bincode::serialize(&error).unwrap();
    c.bench_function("error_deserialization", |b| {
        b.iter(|| {
            let deserialized: NebulaError = bincode::deserialize(&serialized).unwrap();
            black_box(deserialized);
        });
    });
}

/// Benchmark memory usage of different error types
fn bench_error_memory_footprint(c: &mut Criterion) {
    c.bench_function("error_size_simple", |b| {
        b.iter(|| {
            let error = NebulaError::validation("Invalid input");
            black_box(std::mem::size_of_val(&error));
        });
    });

    c.bench_function("error_size_with_context", |b| {
        b.iter(|| {
            let error = NebulaError::validation("Invalid input")
                .with_context(ErrorContext::new("Processing request"));
            black_box(std::mem::size_of_val(&error));
        });
    });
}

/// Benchmark retry logic performance
fn bench_retry_operations(c: &mut Criterion) {
    use nebula_error::{RetryStrategy, retry};
    
    let strategy = RetryStrategy::default()
        .with_max_attempts(3)
        .with_base_delay(Duration::from_millis(1)); // Very short for benchmarking

    c.bench_function("retry_immediate_success", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let result = retry(|| async {
                Ok::<_, NebulaError>("success")
            }, &strategy).await;
            black_box(result);
        });
    });

    c.bench_function("retry_with_failures", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut attempt = 0;
            let result = retry(|| async {
                attempt += 1;
                if attempt < 3 {
                    Err(NebulaError::internal("temporary error"))
                } else {
                    Ok("success")
                }
            }, &strategy).await;
            black_box(result);
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
    bench_error_creation_static_vs_dynamic,
    bench_error_code_access,
    bench_macro_error_creation,
    bench_error_serialization,
    bench_error_memory_footprint,
    bench_retry_operations,
);

criterion_main!(benches);
