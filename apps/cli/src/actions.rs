//! Built-in actions for local CLI execution.
//!
//! These actions are available out-of-the-box when running workflows locally
//! with `nebula run`. They provide basic data manipulation and flow control
//! without requiring any plugins.

use nebula_action::context::Context;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::result::ActionResult;
use nebula_action::{Action, ActionDependencies, StatelessAction};
use nebula_core::action_key;
use nebula_runtime::ActionRegistry;

/// Register all built-in actions with the given registry.
pub fn register_builtins(registry: &ActionRegistry) {
    registry.register_stateless(EchoAction::new());
    registry.register_stateless(SetAction::new());
    registry.register_stateless(NoopAction::new());
    registry.register_stateless(LogAction::new());
    registry.register_stateless(MergeAction::new());
    registry.register_stateless(FilterAction::new());
    registry.register_stateless(FailAction::new());
    registry.register_stateless(DelayAction::new());
}

// ---------------------------------------------------------------------------
// echo — passes input through unchanged
// ---------------------------------------------------------------------------

struct EchoAction {
    meta: ActionMetadata,
}

impl EchoAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "Pass input through unchanged"),
        }
    }
}

impl ActionDependencies for EchoAction {}
impl Action for EchoAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for EchoAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

// ---------------------------------------------------------------------------
// set — outputs a fixed value from parameters
// ---------------------------------------------------------------------------

struct SetAction {
    meta: ActionMetadata,
}

impl SetAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("set"),
                "Set",
                "Output a fixed value from parameters",
            ),
        }
    }
}

impl ActionDependencies for SetAction {}
impl Action for SetAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for SetAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // The "set" action returns whatever is passed as input (which comes
        // from node parameters resolved by the engine).
        Ok(ActionResult::success(input))
    }
}

// ---------------------------------------------------------------------------
// noop — does nothing, always succeeds with empty object
// ---------------------------------------------------------------------------

struct NoopAction {
    meta: ActionMetadata,
}

impl NoopAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(action_key!("noop"), "No-Op", "Does nothing"),
        }
    }
}

impl ActionDependencies for NoopAction {}
impl Action for NoopAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for NoopAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(serde_json::json!({})))
    }
}

// ---------------------------------------------------------------------------
// log — logs the input and passes it through
// ---------------------------------------------------------------------------

struct LogAction {
    meta: ActionMetadata,
}

impl LogAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("log"),
                "Log",
                "Log input data and pass it through",
            ),
        }
    }
}

impl ActionDependencies for LogAction {}
impl Action for LogAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for LogAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let node_id = _ctx.node_id();
        let compact = serde_json::to_string(&input).unwrap_or_else(|_| "???".to_owned());
        eprintln!("[log:{node_id}] {compact}");
        Ok(ActionResult::success(input))
    }
}

// ---------------------------------------------------------------------------
// merge — deep-merges input objects (later keys overwrite earlier)
// ---------------------------------------------------------------------------

struct MergeAction {
    meta: ActionMetadata,
}

impl MergeAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("merge"),
                "Merge",
                "Deep-merge input JSON objects",
            ),
        }
    }
}

impl ActionDependencies for MergeAction {}
impl Action for MergeAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for MergeAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // If input is an array of objects, merge them left-to-right.
        // Otherwise, pass through.
        let Some(arr) = input.as_array() else {
            return Ok(ActionResult::success(input));
        };
        let mut merged = serde_json::Map::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                for (k, v) in obj {
                    merged.insert(k.clone(), v.clone());
                }
            }
        }
        Ok(ActionResult::success(serde_json::Value::Object(merged)))
    }
}

// ---------------------------------------------------------------------------
// filter — keeps only specified keys from the input object
// ---------------------------------------------------------------------------

struct FilterAction {
    meta: ActionMetadata,
}

impl FilterAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("filter"),
                "Filter",
                "Keep only specified keys from input",
            ),
        }
    }
}

impl ActionDependencies for FilterAction {}
impl Action for FilterAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for FilterAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Expects {"data": {...}, "keys": ["a", "b"]}
        // Returns only the specified keys from data.
        let data = input.get("data").cloned().unwrap_or(input.clone());
        let keys: Vec<String> = input
            .get("keys")
            .and_then(|k| serde_json::from_value(k.clone()).ok())
            .unwrap_or_default();

        if keys.is_empty() {
            return Ok(ActionResult::success(data));
        }

        if let Some(obj) = data.as_object() {
            let filtered: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .filter(|(k, _)| keys.contains(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Ok(ActionResult::success(serde_json::Value::Object(filtered)))
        } else {
            Ok(ActionResult::success(data))
        }
    }
}

// ---------------------------------------------------------------------------
// fail — always fails (for testing error handling)
// ---------------------------------------------------------------------------

struct FailAction {
    meta: ActionMetadata,
}

impl FailAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("fail"),
                "Fail",
                "Always fails with a fatal error",
            ),
        }
    }
}

impl ActionDependencies for FailAction {}
impl Action for FailAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for FailAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let msg = input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("intentional failure");
        Err(ActionError::fatal(msg))
    }
}

// ---------------------------------------------------------------------------
// delay — waits for a specified duration then passes input through
// ---------------------------------------------------------------------------

struct DelayAction {
    meta: ActionMetadata,
}

impl DelayAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("delay"),
                "Delay",
                "Wait for a duration then pass input through",
            ),
        }
    }
}

impl ActionDependencies for DelayAction {}
impl Action for DelayAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for DelayAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let ms = input.get("ms").and_then(|v| v.as_u64()).unwrap_or(100);
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        Ok(ActionResult::success(input))
    }
}
