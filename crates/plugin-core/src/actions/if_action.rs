//! `core.if` — binary control-flow branch on a top-level field condition.
//!
//! Evaluates a single [`Condition`] against the `data` object and routes
//! execution to either the `"true"` or `"false"` output port. The `data`
//! value is passed through unchanged on the selected port — downstream nodes
//! receive the original data, not the `IfInput` envelope.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data":      { /* optional object to evaluate; defaults to {} */ },
//!   "condition": { "field": "status", "op": "eq", "value": "active" }
//! }
//! ```
//!
//! ## Output ports
//!
//! | Port    | Activated when |
//! |---------|----------------|
//! | `true`  | condition evaluates to `true` |
//! | `false` | condition evaluates to `false` |
//!
//! ## Condition operators
//!
//! ### `eq` / `ne`
//! - `eq`: `obj.get(field)` == `cond.value` (deep JSON equality). Missing
//!   field → `false` (no value cannot equal anything). `ne` is the logical
//!   negation of `eq`: missing field → `true`.
//! - `cond.value` of `null` compares against JSON `null` literally.
//!
//! ### `gt` / `gte` / `lt` / `lte`
//! - Missing field → **Fatal** (ordered comparison requires a value).
//! - Both numbers → integers compared **exactly** (large 64-bit IDs do not lose
//!   precision); only genuine floats compare via `f64`.
//! - Both strings → lexicographic byte-order comparison.
//! - Type mismatch (e.g. number vs string) → **Fatal** with a message naming
//!   both types.
//!
//! ### `exists` / `not_exists`
//! - `exists`: `obj.get(field).is_some()`. The `value` field is ignored.
//! - `not_exists`: `obj.get(field).is_none()`. The `value` field is ignored.
//!
//! ### `truthy`
//! Exact truthiness table (all other values are truthy):
//!
//! | Value | Truthy? |
//! |-------|---------|
//! | `true` | yes |
//! | `false` | no |
//! | `null` | no |
//! | `0` / `0.0` | no |
//! | `""` (empty string) | no |
//! | `[]` (empty array) | no |
//! | `{}` (empty object) | no |
//! | missing field | no |
//! | non-zero number | yes |
//! | non-empty string | yes |
//! | non-empty array | yes |
//! | non-empty object | yes |
//!
//! The `value` field is ignored for `truthy`.
//!
//! ## Property scoping
//!
//! `condition.field` is a **top-level key** in `data`, not a JSON pointer.
//! A dot character in the field name is literal — `"a.b"` refers to a
//! single key named `"a.b"`, not a nested path.
//!
//! ## Non-object `data`
//!
//! If `data` is a JSON array, boolean, number, or string (anything other than
//! a JSON object or null), **every** operator returns a **Fatal** error naming
//! the actual type. The check happens before operator dispatch, so there is no
//! operator that silently accepts a non-object `data`. `null` and absent `data`
//! are treated as `{}`.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{
    ActionContext, ActionError, branch_key,
    control::{ControlAction, ControlOutcome},
    port::{OutputPort, default_input_ports},
    port_key,
};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Schema, ValidSchema, ValidationReport, field_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::condition::{Condition, evaluate_condition, normalize_data};

// ── Wire types ────────────────────────────────────────────────────────────────

/// Resolved input for the `If` action.
///
/// `data` defaults to an empty object when absent or `null`.
/// `condition` is always required on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IfInput {
    /// Object to evaluate the condition against. `null` / absent → `{}`.
    #[serde(default)]
    pub data: Option<Value>,
    /// The predicate to evaluate.
    pub condition: Condition,
}

impl HasSchema for IfInput {
    #[instrument(name = "core.if.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .property(super::input_schema::nullable_object_data())
                    .property(super::input_schema::condition(field_key!("condition")))
                    .build()
            })
            .clone()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Binary control-flow branch on a field condition.
///
/// Keyed `core.if`. Routes to port `"true"` or `"false"` and passes the
/// original `data` value through on the selected port.
///
/// # Example
///
/// `Condition` serializes to a plain JSON object. The wire shape is what the
/// engine resolves from `NodeDefinition::parameters` before dispatch:
///
/// ```rust
/// use nebula_plugin_core::condition::{Condition, ConditionOp};
/// use serde_json::json;
///
/// let condition = Condition::Leaf {
///     field: "status".into(),
///     op: ConditionOp::Eq,
///     value: Some(json!("active")),
/// };
///
/// // Wire shape: a Leaf serializes to the flat object form — no wrapper key.
/// let wire = serde_json::to_value(&condition).unwrap();
/// assert_eq!(wire, json!({ "field": "status", "op": "eq", "value": "active" }));
///
/// // Round-trip: deserialize back to the same condition.
/// let restored: Condition = serde_json::from_value(wire).unwrap();
/// assert_eq!(restored, condition);
/// ```
///
/// Wire the action into the engine via [`CorePlugin`](crate::CorePlugin) and
/// `WorkflowEngine::with_plugin` — see the crate-level docs for a complete
/// wiring example.
#[derive(Debug)]
pub struct CoreIf;

impl nebula_action::action::Action for CoreIf {
    type Input = IfInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.if"),
            nebula_action::metadata_name!("If"),
            "Routes execution to 'true' or 'false' port based on a field condition",
        )
        .with_inputs(default_input_ports())
        .with_outputs(vec![
            OutputPort::flow(port_key!("true")),
            OutputPort::flow(port_key!("false")),
        ])
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for CoreIf {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CoreIf)
    }
}

impl ControlAction for CoreIf {
    #[instrument(
        name = "core.if",
        skip_all,
        fields(condition_kind = condition_kind(&input.condition))
    )]
    async fn evaluate(
        &self,
        input: IfInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        let IfInput { data, condition } = input;

        let data_object = normalize_data(data).map_err(|err| {
            // Prefix the normalisation error with the action key for context.
            ActionError::fatal(format!("core.if: {err}"))
        })?;
        let branch_taken = evaluate_condition(&data_object, &condition)?;
        let selected = if branch_taken {
            branch_key!("true")
        } else {
            branch_key!("false")
        };

        Ok(ControlOutcome::Branch {
            selected,
            output: data_object,
        })
    }
}

fn condition_kind(condition: &Condition) -> &'static str {
    match condition {
        Condition::Leaf { .. } => "leaf",
        Condition::All(_) => "all",
        Condition::Any(_) => "any",
        Condition::Not(_) => "not",
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "if_action_tests.rs"]
mod tests;
