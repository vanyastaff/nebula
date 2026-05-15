//! No-resend regression fixture.
//!
//! Two actions, each appending one byte to a `resend-counter` file placed
//! next to this fixture's own executable **before** doing anything else,
//! so the host-observable side-effect count is exactly "number of times
//! the plugin received this invocation". The counter path is derived from
//! `current_exe()` (no env var, no shared global state) so concurrent
//! test binaries in distinct temp dirs never collide:
//!
//! - `crash_after_recv` — record the receipt, then `std::process::exit(0)`
//!   **without** writing a response. The host's send succeeded but its
//!   recv hits EOF (`sent == true`), which maps to
//!   `SandboxError::PluginClosedAfterSend` → fatal → the engine MUST NOT
//!   re-dispatch. A correct engine leaves exactly one byte in the file
//!   even under a multi-attempt retry policy.
//! - `fail_retryable` — record the receipt, then return a *retryable*
//!   `PluginError` (no crash). This maps to `ActionError::Retryable`,
//!   which is NOT fatal, so the engine's retry policy still applies — the
//!   deterministic stand-in for the pre-send `PluginClosed → Retryable`
//!   class (a real pre-send stale is not deterministically reproducible
//!   from a child process; the classification + retry-decision path is
//!   identical). A correct engine retries up to `max_attempts`, leaving
//!   that many bytes.
//!
//! Built as a `nebula-plugin-`-prefixed binary so `discover_directory`
//! accepts it.

use std::io::Write;

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    PluginCtx, PluginError, PluginHandler, protocol::ActionDescriptor, run_duplex,
};
use nebula_schema::ValidSchema;
use semver::Version;
use serde_json::Value;

/// Counter file path: `resend-counter` alongside this fixture's own
/// executable. The host copies the binary into a per-test temp dir, so
/// the path is unique per test run with no env var or shared state.
fn counter_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.with_file_name("resend-counter"))
}

fn record_receipt() {
    let Some(path) = counter_path() else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(b"x");
        let _ = f.flush();
    }
}

struct ResendFixture {
    manifest: PluginManifest,
    actions: Vec<ActionDescriptor>,
}

impl ResendFixture {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.nebula.resend", "Resend Fixture")
            .version(Version::new(1, 0, 0))
            .description("No-resend regression fixture.")
            .build()
            .expect("manifest builds");
        let actions = vec![
            ActionDescriptor {
                key: "crash_after_recv".into(),
                name: "Crash after recv".into(),
                description: "Record receipt then exit before responding".into(),
                schema: ValidSchema::empty(),
            },
            ActionDescriptor {
                key: "fail_retryable".into(),
                name: "Fail retryable".into(),
                description: "Record receipt then return a retryable error".into(),
                schema: ValidSchema::empty(),
            },
        ];
        Self { manifest, actions }
    }
}

#[async_trait]
impl PluginHandler for ResendFixture {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> &[ActionDescriptor] {
        &self.actions
    }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        action_key: &str,
        _input: Value,
    ) -> Result<Value, PluginError> {
        match action_key {
            "crash_after_recv" => {
                record_receipt();
                // Die AFTER the host's send succeeded but BEFORE we write a
                // response: the host observes EOF with sent == true.
                std::process::exit(0);
            },
            "fail_retryable" => {
                record_receipt();
                Err(PluginError::retryable(
                    "TRANSIENT",
                    "transient failure — safe to retry",
                ))
            },
            other => Err(PluginError::fatal(
                "UNKNOWN_ACTION",
                format!("resend fixture does not implement '{other}'"),
            )),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    run_duplex(ResendFixture::new()).await
}
