//! Core-flavor worker binary.
//!
//! Boots the first-party [`CorePlugin`], wires it into a [`WorkflowEngine`],
//! and runs durable control, recovery, resource fanout, and timer processing
//! via [`nebula_worker`].
//!
//! ## Configuration (environment variables)
//!
//! | Variable | Default | Description |
//! |---|---|---|
//! | `NEBULA_WORKER_ARTIFACT_SET_DIGEST` | required | 64 lowercase hex digits identifying the worker artifact set; supplied by the trusted release/deployment manifest, not derived from plugin metadata |
//! | `NEBULA_WORKER_DATABASE_URL` | unset | Postgres DSN; when set, uses Postgres backend (requires `--features postgres`). Unset = SQLite default. |
//! | `NEBULA_WORKER_DB_PATH` | `nebula-worker.db` | SQLite database file path (ignored when `NEBULA_WORKER_DATABASE_URL` is set) |
//! | `NEBULA_WORKER_PROCESSOR_ID` | random UUID v4 per boot | 32 hex chars (16 bytes); set explicitly for stable fence identity |
//! | `RUST_LOG` | `info` | `tracing` subscriber filter |
//!
//! [`CorePlugin`]: nebula_plugin_core::CorePlugin
//! [`WorkflowEngine`]: nebula_engine::WorkflowEngine

#![expect(
    clippy::print_stderr,
    reason = "binary edge: startup/exit errors must reach stderr outside the tracing lifetime"
)]

mod compose_main;

use compose_main::run;

#[tokio::main]
async fn main() {
    // `run()` handles all setup, signal handling, and graceful shutdown.
    // Errors are reported to stderr as actionable messages; the process exits
    // non-zero on any hard failure. Panics inside the worker task propagate
    // through the JoinHandle and cause a non-zero exit via `expect`.
    if let Err(e) = run().await {
        // Display chain (not Debug) gives the user an actionable message.
        eprintln!("error: {e}");
        // Walk the source chain for additional context.
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = std::error::Error::source(cause);
        }
        std::process::exit(1);
    }
}
