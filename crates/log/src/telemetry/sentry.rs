//! Sentry integration

#[cfg(feature = "sentry")]
use sentry::ClientInitGuard;

/// Initialize Sentry
pub fn init() -> Option<ClientInitGuard> {
    // Check if Sentry DSN is configured
    let dsn = std::env::var("SENTRY_DSN").ok()?;

    if dsn.is_empty() || dsn == "disabled" {
        return None;
    }

    let environment = std::env::var("SENTRY_ENV")
        .or_else(|_| std::env::var("NEBULA_ENV"))
        .unwrap_or_else(|_| "development".to_string());

    let release = std::env::var("SENTRY_RELEASE")
        .ok()
        .or_else(|| option_env!("CARGO_PKG_VERSION").map(String::from));

    let sample_rate = std::env::var("SENTRY_TRACES_SAMPLE_RATE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.1);

    let parsed_dsn = match dsn.parse::<sentry::types::Dsn>() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "SENTRY_DSN is set but invalid; Sentry reporting is disabled"
            );
            return None;
        },
    };

    let guard = sentry::init(sentry::ClientOptions {
        dsn: Some(parsed_dsn),
        environment: Some(environment.into()),
        release: release.map(|s| s.into()),
        traces_sample_rate: sample_rate,
        attach_stacktrace: true,
        send_default_pii: false,
        ..Default::default()
    });

    // Integrate with tracing
    if guard.is_enabled() {
        tracing::info!("Sentry initialized");
    }

    Some(guard)
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    use tracing_subscriber::fmt::MakeWriter;

    /// In-memory writer for capturing `tracing` output in tests.
    ///
    /// Implements `std::io::Write` so `tracing_subscriber::fmt` can write into
    /// it, and `MakeWriter` so the subscriber can produce fresh writers per
    /// event. All writes land in a shared `Arc<Mutex<Vec<u8>>>` the test reads
    /// at the end.
    #[derive(Clone)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for BufWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// #377 — invalid `SENTRY_DSN` must emit a `tracing::warn!` before returning
    /// `None`. Previously the silent `.ok()?` shortcut meant operators who set
    /// a malformed DSN got "no Sentry events" with no startup signal.
    ///
    /// We capture `tracing` output via a thread-local subscriber and assert the
    /// warn message is actually emitted.
    #[test]
    fn invalid_sentry_dsn_emits_warning() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = BufWriter(buf.clone());

        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();

        // Save & restore SENTRY_DSN around the mutation. `cargo-nextest` runs
        // each test in its own process, so the env var is process-local, but
        // under `cargo test` we rely on no other test in this binary touching
        // SENTRY_DSN concurrently.
        let prev = std::env::var("SENTRY_DSN").ok();
        // SAFETY: std::env::set_var / remove_var are unsafe in Rust edition
        // 2024 because they race with concurrent readers of std::env in other
        // threads. This test does not spawn threads and nothing else in this
        // binary reads SENTRY_DSN concurrently while the test holds it.
        unsafe {
            std::env::set_var("SENTRY_DSN", "not-a-valid-dsn");
        }

        let guard = tracing::subscriber::with_default(subscriber, || super::init());

        // Restore env before any assertions so a failing assert cannot leak
        // state into sibling tests. SAFETY: same as above.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SENTRY_DSN", v),
                None => std::env::remove_var("SENTRY_DSN"),
            }
        }

        assert!(
            guard.is_none(),
            "invalid DSN must not produce a Sentry guard"
        );

        let captured =
            String::from_utf8(buf.lock().unwrap().clone()).expect("tracing output must be UTF-8");
        assert!(
            captured.contains("SENTRY_DSN is set but invalid"),
            "expected warn log about invalid SENTRY_DSN, got: {captured}"
        );
    }
}
