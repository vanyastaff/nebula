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

    let guard = sentry::init(sentry::ClientOptions {
        dsn: Some(dsn.parse().ok()?),
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