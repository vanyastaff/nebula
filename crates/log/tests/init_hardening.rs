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
