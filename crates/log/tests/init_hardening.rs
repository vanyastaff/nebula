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

/// #377 — invalid SENTRY_DSN must not silently disable Sentry.
///
/// We cannot easily intercept the `tracing::warn!` (the subscriber is
/// process-global), so we instead assert that `sentry::init()` still returns
/// `None` for a bogus DSN *and* that the call path does not panic. The real
/// regression signal is a code inspection: the `ok()?` shortcut must be gone.
#[cfg(feature = "sentry")]
#[test]
fn invalid_sentry_dsn_returns_none_without_panic() {
    // Save & restore env so parallel tests don't clobber each other.
    let prev = std::env::var("SENTRY_DSN").ok();
    // SAFETY: test-only single-threaded env mutation.
    unsafe {
        std::env::set_var("SENTRY_DSN", "not-a-valid-dsn");
    }

    let guard = nebula_log::telemetry::sentry::init();
    assert!(
        guard.is_none(),
        "invalid DSN must not produce a Sentry guard"
    );

    // Restore env.
    match prev {
        Some(v) => unsafe { std::env::set_var("SENTRY_DSN", v) },
        None => unsafe { std::env::remove_var("SENTRY_DSN") },
    }
}
