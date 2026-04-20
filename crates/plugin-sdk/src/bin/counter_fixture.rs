//! Counter plugin fixture: demonstrates long-lived plugin state.
//!
//! Holds an `AtomicI64` and exposes three actions:
//!
//! - `increment { amount: i64 }` → `{ total, added }`
//! - `current` → `{ total }`
//! - `reset` → `{ total: 0, reset: true }`
//!
//! Paired with `crates/sandbox/examples/sandbox_demo.rs` which drives it
//! over multiple calls. If the plugin were respawned between calls, the
//! counter would reset every time; seeing it accumulate proves slice 1c's
//! long-lived `PluginHandle` is live.

use std::{
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    PluginCtx, PluginError, PluginHandler, protocol::ActionDescriptor, run_duplex,
};
use nebula_schema::Schema;
use semver::Version;
use serde_json::{Value, json};

struct CounterPlugin {
    manifest: PluginManifest,
    actions: Vec<ActionDescriptor>,
    total: AtomicI64,
}

impl CounterPlugin {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.nebula.counter", "Counter")
            .version(Version::new(0, 1, 0))
            .description("Fixture plugin — returns an incrementing counter.")
            .build()
            .unwrap();
        let empty_schema = || Schema::builder().build().unwrap();
        let actions = vec![
            ActionDescriptor {
                key: "increment".into(),
                name: "Increment".into(),
                description: "Add the given amount to the running total".into(),
                schema: empty_schema(),
            },
            ActionDescriptor {
                key: "current".into(),
                name: "Current".into(),
                description: "Return the current running total".into(),
                schema: empty_schema(),
            },
            ActionDescriptor {
                key: "reset".into(),
                name: "Reset".into(),
                description: "Reset the running total to zero".into(),
                schema: empty_schema(),
            },
            ActionDescriptor {
                key: "panic".into(),
                name: "Panic".into(),
                description: "Deliberately panic (probe)".into(),
                schema: empty_schema(),
            },
            ActionDescriptor {
                key: "slow".into(),
                name: "Slow".into(),
                description: "Sleep for `millis` then return (probe timeout handling)".into(),
                schema: empty_schema(),
            },
            ActionDescriptor {
                key: "big".into(),
                name: "Big".into(),
                description: "Return a payload of roughly `kb` kilobytes (probe large IO)".into(),
                schema: empty_schema(),
            },
        ];
        Self {
            manifest,
            actions,
            total: AtomicI64::new(0),
        }
    }
}

#[async_trait]
impl PluginHandler for CounterPlugin {
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
            "increment" => {
                let amount = input.get("amount").and_then(Value::as_i64).unwrap_or(1);
                let previous = self.total.fetch_add(amount, Ordering::Relaxed);
                let new_total = previous + amount;
                Ok(json!({
                    "total": new_total,
                    "added": amount,
                    "previous": previous,
                }))
            },
            "current" => Ok(json!({
                "total": self.total.load(Ordering::Relaxed),
            })),
            "reset" => {
                self.total.store(0, Ordering::Relaxed);
                Ok(json!({
                    "total": 0,
                    "reset": true,
                }))
            },
            // Probe actions for slice 1c validation — exercise edge cases
            // in the long-lived plugin lifecycle.
            "panic" => {
                panic!("boom from counter plugin (probe)");
            },
            "slow" => {
                let millis = input.get("millis").and_then(Value::as_u64).unwrap_or(2000);
                tokio::time::sleep(Duration::from_millis(millis)).await;
                Ok(json!({ "slept_ms": millis }))
            },
            "big" => {
                let kb = input.get("kb").and_then(Value::as_u64).unwrap_or(100);
                let len = (kb as usize) * 1024;
                let data: String = "x".repeat(len);
                Ok(json!({
                    "size_bytes": len,
                    "data": data,
                }))
            },
            other => Err(PluginError::fatal(
                "UNKNOWN_ACTION",
                format!("counter plugin does not implement action '{other}'"),
            )),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    run_duplex(CounterPlugin::new()).await
}
