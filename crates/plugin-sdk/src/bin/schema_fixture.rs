//! Schema-bearing fixture — used by the discovery schema round-trip test.
//!
//! Declares one action (`describe`) with a two-field schema:
//!   - `name`: required string
//!   - `age`: optional number
//!
//! The `discovery_schema_roundtrip` integration test in `nebula-sandbox`
//! spawns this binary through `discover_directory` and asserts that both
//! schema fields survive the broker envelope round-trip into the host-side
//! `ActionMetadata`.

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    PluginCtx, PluginError, PluginHandler, protocol::ActionDescriptor, run_duplex,
};
use nebula_schema::{Field, Schema, field_key};
use semver::Version;
use serde_json::{Value, json};

struct SchemaFixture {
    manifest: PluginManifest,
    actions: Vec<ActionDescriptor>,
}

impl SchemaFixture {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.author.schema", "Schema Fixture")
            .version(Version::new(1, 0, 0))
            .description("Fixture declaring one action with a two-field schema.")
            .build()
            .unwrap();

        let schema = Schema::builder()
            .add(Field::string(field_key!("name")).required())
            .add(Field::number(field_key!("age")))
            .build()
            .unwrap();

        let actions = vec![ActionDescriptor {
            key: "describe".into(),
            name: "Describe".into(),
            description: "Round-trip schema probe.".into(),
            schema,
        }];

        Self { manifest, actions }
    }
}

#[async_trait]
impl PluginHandler for SchemaFixture {
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
            "describe" => Ok(json!({ "received": input })),
            other => Err(PluginError::fatal(
                "UNKNOWN_ACTION",
                format!("unknown action: {other}"),
            )),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    run_duplex(SchemaFixture::new()).await
}
