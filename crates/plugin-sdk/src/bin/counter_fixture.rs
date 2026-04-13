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

use std::sync::atomic::{AtomicI64, Ordering};

use nebula_plugin_sdk::{PluginCtx, PluginError, PluginHandler, PluginMeta, run_duplex};
use serde_json::{Value, json};

struct CounterPlugin {
    total: AtomicI64,
}

#[async_trait::async_trait]
impl PluginHandler for CounterPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta::new("com.nebula.counter", "0.1.0")
            .with_action(
                "increment",
                "Increment",
                "Add the given amount to the running total",
            )
            .with_action("current", "Current", "Return the current running total")
            .with_action("reset", "Reset", "Reset the running total to zero")
    }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        action_key: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        match action_key {
            "increment" => {
                let amount = input.get("amount").and_then(|v| v.as_i64()).unwrap_or(1);
                let previous = self.total.fetch_add(amount, Ordering::Relaxed);
                let new_total = previous + amount;
                Ok(json!({
                    "total": new_total,
                    "added": amount,
                    "previous": previous,
                }))
            }
            "current" => Ok(json!({
                "total": self.total.load(Ordering::Relaxed),
            })),
            "reset" => {
                self.total.store(0, Ordering::Relaxed);
                Ok(json!({
                    "total": 0,
                    "reset": true,
                }))
            }
            other => Err(PluginError::fatal(
                "UNKNOWN_ACTION",
                format!("counter plugin does not implement action '{other}'"),
            )),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let plugin = CounterPlugin {
        total: AtomicI64::new(0),
    };
    run_duplex(plugin).await
}
