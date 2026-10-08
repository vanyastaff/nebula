//! Core-flavor worker binary.
//!
//! Boots the first-party core plugin, wires it into a [`WorkflowEngine`],
//! and runs durable control, recovery, resource fanout, and timer processing
//! via [`nebula_worker`].
//!
//! `--help` and `--version` exit before reading deployment configuration.
//! Unrecognized arguments are rejected before initialization.
//! Credential key configuration is validated before opening deployment storage.
//! Shutdown cancels and joins the owned runtime task. On its twenty-second drain
//! timeout the host aborts and joins that task, then exits unsuccessfully; it does
//! not report a completed drain or detach the task. Blocking/non-yielding code
//! still requires the external process supervisor's termination deadline.
//!
//! ## Configuration (environment variables)
//!
//! | Variable | Default | Description |
//! |---|---|---|
//! | `NEBULA_WORKER_ARTIFACT_SET_DIGEST` | required | 64 lowercase hex digits identifying the worker artifact set; supplied by the trusted release/deployment manifest, not derived from plugin metadata |
//! | `NEBULA_WORKER_DATABASE_URL` | unset | Postgres DSN of the deployment database (executions, tenancy, credentials); when set, uses Postgres backend (requires `--features postgres`). Unset = SQLite default. |
//! | `NEBULA_WORKER_DB_PATH` | `nebula-worker.db` | SQLite deployment database file path, credentials included (ignored when `NEBULA_WORKER_DATABASE_URL` is set) |
//! | `NEBULA_WORKER_PROCESSOR_ID` | random UUID v4 per boot | 32 hex chars (16 bytes); set explicitly for stable fence identity |
//! | `NEBULA_CRED_MASTER_KEY` | required | Base64-encoded 32-byte credential encryption key. |
//! | `NEBULA_CRED_LEGACY_MASTER_KEYS` | unset | Up to eight comma-separated base64 AES-256 keys accepted only for decrypting historical credential envelopes. |
//! | `NEBULA_CRED_LEGACY_EMPTY_ID_MASTER_KEY` | unset | One base64 AES-256 decrypt-only key for credential envelopes written by the historical empty-key-ID format. |
//! | `NEBULA_CRED_DEV_KEY` | unset | Set to `1` only for an insecure fixed development key. |
//! | `RUST_LOG` | `info` | `tracing` subscriber filter |
//!
//! [`WorkflowEngine`]: nebula_engine::WorkflowEngine

#![expect(
    clippy::print_stderr,
    reason = "binary edge: startup/exit errors must reach stderr outside the tracing lifetime"
)]

mod compose_main;

use clap::Parser;
use compose_main::run;

#[derive(Parser)]
#[command(
    name = "nebula-worker",
    version,
    about = "Nebula core-flavor execution worker",
    after_help = "Deployment configuration is read from NEBULA_WORKER_* and NEBULA_CRED_* environment variables."
)]
struct Cli;

#[tokio::main]
async fn main() {
    Cli::parse();
    // `run()` handles all setup, signal handling, and graceful shutdown.
    // Errors are reported to stderr as actionable messages; the process exits
    // non-zero on any hard failure, including a supervised worker task panic.
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
