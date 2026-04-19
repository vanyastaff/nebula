//! Log-redaction test helper for `nebula-credential`.
//!
//! Verifies the §12.5 invariant from `docs/PRODUCT_CANON.md`:
//! **no secrets in logs, error strings, or metrics labels.**
//!
//! The helper installs a `tracing-subscriber` that captures every
//! formatted event into an in-memory buffer, runs the supplied closure,
//! and then asserts that a caller-supplied "forbidden" substring never
//! appears in the capture.
//!
//! A positive test exercises `SecretString` / `SecretToken` through
//! `tracing::info!` / `tracing::error!` and confirms the secret never
//! surfaces (only the `[REDACTED]` sentinel does). A negative test
//! deliberately logs the raw secret and asserts that the helper
//! **panics**, confirming the check is load-bearing rather than
//! silently passing.

#![allow(clippy::missing_panics_doc)]

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

use nebula_credential::{SecretString, scheme::SecretToken};
use tracing_subscriber::fmt::MakeWriter;

// ---------------------------------------------------------------------
// Capture buffer + MakeWriter plumbing
// ---------------------------------------------------------------------

/// Shared buffer that every captured event is appended to.
#[derive(Clone, Default)]
struct CaptureBuf(Arc<Mutex<Vec<u8>>>);

impl CaptureBuf {
    fn as_string(&self) -> String {
        let guard = self.0.lock().expect("capture buffer poisoned");
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl Write for CaptureBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.0.lock().expect("capture buffer poisoned");
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureBuf {
    type Writer = CaptureBuf;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// ---------------------------------------------------------------------
// Public helper
// ---------------------------------------------------------------------

/// Runs `body` with a capturing `tracing` subscriber installed for the
/// current thread, then asserts no `forbidden` substring leaked into any
/// captured event.
///
/// The assertion failure is deliberate — it is the only way this helper
/// signals a leak. Use `#[should_panic]` on a test to verify the check
/// fires (see the negative test below).
pub fn assert_no_secret_in_logs<F>(forbidden: &str, body: F)
where
    F: FnOnce(),
{
    assert!(
        !forbidden.is_empty(),
        "the forbidden substring must be non-empty; \
         testing against an empty string would pass trivially"
    );

    let buf = CaptureBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_target(false)
        .with_level(true)
        .finish();

    tracing::subscriber::with_default(subscriber, body);

    let captured = buf.as_string();
    assert!(
        !captured.contains(forbidden),
        "log-redaction invariant violated: the forbidden substring \
         {forbidden:?} leaked into captured tracing output.\n\
         ---- captured ----\n{captured}\n------------------"
    );
}

// ---------------------------------------------------------------------
// Positive: SecretString / SecretToken never leak through tracing
// ---------------------------------------------------------------------

#[test]
fn secret_string_debug_and_display_never_leak_to_logs() {
    let raw = "sk-positive-12345-never-logged";
    let secret = SecretString::new(raw);

    assert_no_secret_in_logs(raw, || {
        tracing::info!(secret = ?secret, "debug formatting secret");
        tracing::warn!("display formatting secret: {secret}");
        tracing::error!("error path carrying a secret: {secret:?} -- still redacted");
    });
}

#[test]
fn secret_token_never_leaks_through_tracing() {
    let raw = "api-key-positive-abcdef-never-logged";
    let token = SecretToken::new(SecretString::new(raw));

    assert_no_secret_in_logs(raw, || {
        tracing::info!(token = ?token, "logging a SecretToken");
        tracing::error!("token in message template: {token:?}");
    });
}

#[test]
fn helper_records_the_redacted_sentinel_when_secrets_are_formatted() {
    // Sanity check on the *positive* half of the contract: the
    // `[REDACTED]` marker is what actually ends up in the logs.
    let secret = SecretString::new("sk-sentinel-check");

    let buf = CaptureBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(secret = ?secret, "with a secret");
    });

    let captured = buf.as_string();
    assert!(
        captured.contains("[REDACTED]"),
        "expected the [REDACTED] sentinel in captured output, got:\n{captured}"
    );
}

// ---------------------------------------------------------------------
// Negative: the helper *must* panic when a raw secret is logged
// ---------------------------------------------------------------------

#[test]
#[should_panic(expected = "log-redaction invariant violated")]
fn helper_panics_when_raw_secret_is_logged() {
    // Intentionally bypass SecretString and log a raw string. The
    // helper must catch it; otherwise the whole redaction test suite
    // would silently pass on a real leak.
    let raw = "this-would-be-a-real-leak-0xDEAD";

    assert_no_secret_in_logs(raw, || {
        tracing::info!("oh no, we logged the raw secret: {raw}");
    });
}

#[test]
#[should_panic(expected = "forbidden substring must be non-empty")]
fn helper_rejects_empty_forbidden_substring() {
    assert_no_secret_in_logs("", || {
        tracing::info!("nothing forbidden — the helper must refuse this");
    });
}
