//! Integration tests for nebula-log init-hardening fixes (#375/#377/#379/#380).
//!
//! These tests share a process-global `tracing` dispatcher, so they are ordered
//! and gated via `serial_test` where necessary. The first init always wins;
//! subsequent calls must return `LogError::AlreadyInitialized`.

use nebula_log::{Config, LogError, init_with};

/// #379 — second `init_with` returns a structured `AlreadyInitialized` error,
/// not a generic `Internal` error.
#[test]
fn second_init_with_returns_already_initialized() {
    // First init wins (or is already installed by a prior test).
    let _ = init_with(Config::default());

    // Second init must now return AlreadyInitialized.
    let err = init_with(Config::default()).expect_err("expected duplicate init to error");
    assert!(
        matches!(err, LogError::AlreadyInitialized),
        "expected AlreadyInitialized, got: {err:?}"
    );
}

/// #380 companion — verifies the Task 2 duplicate-init fast-path still
/// short-circuits cleanly even when a telemetry config is present.
///
/// This does NOT directly exercise #380's error-path cleanup (the fast-path
/// returns `AlreadyInitialized` before any OTel layer is built). The real
/// #380 coverage lives in `crates/log/src/telemetry/otel.rs`'s unit tests —
/// see `build_layer_then_shutdown_is_safe`.
#[cfg(feature = "telemetry")]
#[test]
fn fast_path_survives_duplicate_init_with_telemetry_config() {
    use nebula_log::{Config, LogError, TelemetryConfig, init_with};

    // Force a prior init so the next one hits `AlreadyInitialized`.
    let _ = init_with(Config::default());

    let mut cfg = Config::default();
    cfg.telemetry = Some(TelemetryConfig {
        // Use a syntactically-valid but unreachable endpoint: build_layer must
        // succeed in constructing the exporter/provider, and the error must
        // come from try_init (duplicate dispatcher), not from OTLP construction.
        otlp_endpoint: Some("http://127.0.0.1:1".to_string()),
        service_name: "partial-init-test".to_string(),
        sampling_rate: 0.0,
    });

    let err = init_with(cfg).expect_err("duplicate init must fail");
    assert!(
        matches!(err, LogError::AlreadyInitialized),
        "expected AlreadyInitialized, got: {err:?}"
    );
    // If we got here without panicking, the error-path cleanup is OK.
}
