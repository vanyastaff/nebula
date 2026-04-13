//! Test fixture: a minimal plugin that echoes any input back unchanged.
//!
//! Built by Cargo as a binary target of `nebula-plugin-sdk`. The sibling
//! integration test `tests/broker_smoke.rs` spawns this binary via
//! `env!("CARGO_BIN_EXE_nebula-echo-fixture")` and exercises the duplex
//! protocol end-to-end.
//!
//! Plugin authors in the wild will have their own binary crate depending on
//! `nebula-plugin-sdk` and calling [`nebula_plugin_sdk::run_duplex`]. This
//! fixture is an example of the pattern.

use nebula_plugin_sdk::{PluginCtx, PluginError, PluginHandler, PluginMeta, run_duplex};
use serde_json::Value;

struct EchoPlugin;

#[async_trait::async_trait]
impl PluginHandler for EchoPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta::new("com.nebula.echo", "0.1.0").with_action(
            "echo",
            "Echo",
            "Returns the input as the output",
        )
    }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        action_key: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        match action_key {
            "echo" => Ok(input),
            other => Err(PluginError::fatal(
                "UNKNOWN_ACTION",
                format!("echo plugin does not implement action '{other}'"),
            )),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    run_duplex(EchoPlugin).await
}
