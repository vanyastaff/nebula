//! Core-flavor worker binary.
//!
//! Boots the first-party [`CorePlugin`], wires it into a [`WorkflowEngine`],
//! and runs the durable claim-loop via [`nebula_worker`].
//!
//! ## Configuration (environment variables)
//!
//! | Variable | Default | Description |
//! |---|---|---|
//! | `NEBULA_WORKER_DB_PATH` | `nebula-worker.db` | SQLite database file path |
//! | `NEBULA_WORKER_PROCESSOR_ID` | hostname-derived | 32 hex chars (16 bytes) |
//! | `NEBULA_WORKER_BATCH_SIZE` | orchestrator default (32) | Jobs per claim batch |
//! | `NEBULA_WORKER_POLL_INTERVAL_MS` | 100 | Idle poll interval (ms) |
//! | `RUST_LOG` | `info` | `tracing` subscriber filter |
//!
//! [`CorePlugin`]: nebula_plugin_core::CorePlugin
//! [`WorkflowEngine`]: nebula_engine::WorkflowEngine

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
