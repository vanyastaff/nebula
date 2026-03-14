//! Example: configuring nebula-log with Sentry error tracking.
//!
//! This example shows how to add Sentry integration to `nebula-log`.  When
//! enabled, `ERROR`-level events automatically become Sentry issues and
//! `WARN`-level events are forwarded as breadcrumbs.
//!
//! # Enabling the feature
//!
//! Add to `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! nebula-log = { version = "…", features = ["sentry"] }
//! ```
//!
//! # Environment variables
//!
//! | Variable | Purpose | Default |
//! |----------|---------|---------|
//! | `SENTRY_DSN` | **Required.** Sentry project DSN (e.g. `https://key@sentry.io/123`). Leave unset to disable Sentry. | — |
//! | `SENTRY_ENV` | Sentry environment tag (e.g. `production`, `staging`). Falls back to `NEBULA_ENV`. | `development` |
//! | `SENTRY_RELEASE` | Release identifier forwarded to Sentry. | `CARGO_PKG_VERSION` |
//! | `SENTRY_TRACES_SAMPLE_RATE` | Fraction of transactions sampled for performance monitoring (0.0–1.0). | `0.1` |
//!
//! # Event filter policy (hardcoded)
//!
//! | tracing level | Sentry action |
//! |---|---|
//! | `ERROR` | Creates a Sentry **issue** |
//! | `WARN` | Records a Sentry **breadcrumb** |
//! | `INFO` / `DEBUG` / `TRACE` | Ignored by Sentry |
//!
//! # Running
//!
//! ```text
//! SENTRY_DSN=https://yourkey@sentry.io/123 \
//!   cargo run --example sentry_setup -p nebula-log --features sentry
//! ```
//!
//! Without `SENTRY_DSN` the example runs normally; Sentry is simply not
//! initialised (the guard is `None`).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Sentry is initialised automatically from env vars ──────────────────
    //
    // `nebula_log::init_with` calls `crate::telemetry::sentry::init()` which
    // reads SENTRY_DSN.  No extra builder method is required.
    //
    // To configure Sentry before init you can also call
    // `sentry::init(sentry::ClientOptions { dsn: ..., ..Default::default() })`
    // directly.

    let _guard = nebula_log::auto_init()?;

    tracing::info!("logging and Sentry initialised (DSN from SENTRY_DSN env var)");

    // ── 2. A WARN emits a breadcrumb in Sentry ────────────────────────────────
    tracing::warn!(
        service = "scheduler",
        "task queue depth exceeding threshold — consider scaling"
    );

    // ── 3. An ERROR creates an issue in Sentry ────────────────────────────────
    //
    // In production you would surface the real error here; this call is for
    // demonstration purposes only.
    tracing::error!(
        error.code = "E_QUEUE_FULL",
        service = "scheduler",
        "failed to enqueue workflow execution — queue full"
    );

    // ── 4. Structured fields are forwarded as Sentry extra/tags ──────────────
    tracing::error!(
        workflow_id = "wf-abc123",
        tenant = "acme-corp",
        "workflow execution aborted",
    );

    tracing::info!("example complete — check your Sentry project for the two errors above");

    // The `_guard` drop flushes outstanding Sentry events before the
    // process exits.  Keep it alive until after your last event.
    Ok(())
}
