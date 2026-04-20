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

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    PluginCtx, PluginError, PluginHandler, protocol::ActionDescriptor, run_duplex,
};
use nebula_schema::Schema;
use semver::Version;
use serde_json::Value;

struct EchoPlugin {
    manifest: PluginManifest,
    actions: Vec<ActionDescriptor>,
}

impl EchoPlugin {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.nebula.echo", "Echo")
            .version(Version::new(0, 1, 0))
            .description("Fixture plugin — echoes its input back.")
            .build()
            .unwrap();
        let actions = vec![ActionDescriptor {
            key: "echo".into(),
            name: "Echo".into(),
            description: "Returns the input as the output".into(),
            schema: Schema::builder().build().unwrap(),
        }];
        Self { manifest, actions }
    }
}

#[async_trait]
impl PluginHandler for EchoPlugin {
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
    run_duplex(EchoPlugin::new()).await
}
