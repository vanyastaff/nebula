//! Sentry integration

#[cfg(feature = "sentry")]
use sentry::ClientInitGuard;

/// Initialize Sentry from environment variables.
///
/// Reads `SENTRY_DSN`, `SENTRY_ENV` / `NEBULA_ENV`, `SENTRY_RELEASE`, and
/// `SENTRY_TRACES_SAMPLE_RATE`, then delegates to [`init_from_dsn`].
pub fn init() -> Option<ClientInitGuard> {
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

    init_from_dsn(&dsn, environment, release, sample_rate)
}

/// Pure initializer: build a Sentry client from an explicit DSN string.
///
/// On parse failure, emits both an `eprintln!` (guaranteed visible at startup
/// regardless of tracing subscriber state) and a `tracing::warn!` (captured by
/// any active subscriber), then returns `None`.
///
/// `eprintln!` is necessary because `sentry::init` runs from
/// `LoggerBuilder::build` *before* `try_init` installs the tracing subscriber,
/// so a `tracing::warn!` alone would land on the global no-op dispatcher and
/// be dropped.
fn init_from_dsn(
    dsn: &str,
    environment: String,
    release: Option<String>,
    sample_rate: f32,
) -> Option<ClientInitGuard> {
    let parsed_dsn = match dsn.parse::<sentry::types::Dsn>() {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "nebula-log: SENTRY_DSN is set but invalid ({e}); Sentry reporting is disabled"
            );
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

    if guard.is_enabled() {
        tracing::info!("Sentry initialized");
    }

    Some(guard)
}

#[cfg(test)]
mod tests {
    /// #377 — invalid DSN returns `None` via the pure helper. No env mutation,
    /// no `unsafe`, no tracing subscriber dependency — the test verifies the
    /// parse branch directly.
    ///
    /// The `eprintln!` + `tracing::warn!` side effects in the error path are
    /// not asserted here (stderr/tracing capture in a unit test is brittle).
    /// The #377 fix's observable contract is "returns None on invalid DSN",
    /// which this test locks in.
    #[test]
    fn invalid_dsn_returns_none() {
        let result = super::init_from_dsn("not-a-valid-dsn", "test".to_string(), None, 0.0);
        assert!(result.is_none());
    }
}
