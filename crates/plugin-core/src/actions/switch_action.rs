//! `core.switch` — N-way control-flow branch on an ordered list of conditions.
//!
//! Evaluates a list of [`SwitchCase`]s **in order** against the `data` object.
//! The first case whose [`Condition`] matches routes execution to that case's
//! `port`. If no case matches (including an empty case list), execution routes
//! to the `"default"` port. The `data` value is passed through unchanged on the
//! selected port.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": { /* optional object; defaults to {} */ },
//!   "cases": [
//!     { "condition": { "field": "status", "op": "eq", "value": "active" },   "port": "a" },
//!     { "condition": { "field": "score",  "op": "gt", "value": 90 },         "port": "b" }
//!   ]
//! }
//! ```
//!
//! ## Output ports
//!
//! | Port       | Activated when |
//! |------------|----------------|
//! | `<port>`   | First matching case selects this port name |
//! | `"default"` | No case matched (or cases list is empty) |
//!
//! Ports are declared as a single `Dynamic` output port template keyed
//! `"case"` with `source_field = "cases"`, `label_field = "port"`, and
//! `include_fallback = true` (which auto-generates the `"default"` port).
//!
//! ## Evaluation semantics
//!
//! - **First-match-wins**: cases are tested in declaration order. Once a
//!   matching case is found, remaining cases are **not evaluated** — a later
//!   case whose condition would cause a Fatal error is never reached.
//! - **Case-Fatal propagates**: if an evaluated case's condition returns a
//!   Fatal error (e.g. ordered comparison on a missing field), the error
//!   propagates immediately; the switch does not skip to the next case.
//! - **Duplicate port names** are allowed: the first matching case wins
//!   regardless of whether later cases share the same port name.
//! - **Data passthrough**: the normalized `data` object is emitted unchanged
//!   on the selected port (matched case or `"default"`).
//!
//! ## Non-object `data`
//!
//! If `data` is anything other than a JSON object or `null`, every case
//! evaluation returns a Fatal error. `null` and absent `data` normalize to `{}`.
//!
//! ## Property scoping
//!
//! `case.condition.field` is a top-level key in `data`, not a JSON pointer.
//! A dot in the field name is literal.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{
    ActionContext, ActionError, branch_key,
    control::{ControlAction, ControlOutcome},
    port::{DynamicPort, OutputPort, default_input_ports},
    port_key,
};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Property, Schema, ValidSchema, ValidationReport, field_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::condition::{Condition, evaluate_condition, normalize_data};

// ── Wire types ────────────────────────────────────────────────────────────────

/// A single (condition → port) branch in a Switch node.
///
/// `condition` is evaluated; on a match, execution routes to `port`.
/// `port` is the string name of the output port to activate — it does not
/// need to be unique across cases (first-match-wins applies).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchCase {
    /// The predicate to evaluate against `data`.
    pub condition: Condition,
    /// Output port to select when this case matches.
    pub port: String,
}

/// Resolved input for the `Switch` action.
///
/// `data` defaults to an empty object when absent or `null`.
/// `cases` defaults to an empty list when absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchInput {
    /// Object to evaluate case conditions against. `null` / absent → `{}`.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of (condition → port) cases. Evaluated first-to-last.
    #[serde(default)]
    pub cases: Vec<SwitchCase>,
}

impl HasSchema for SwitchInput {
    #[instrument(name = "core.switch.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .property(super::input_schema::nullable_object_data())
                    .property(
                        Property::list(field_key!("cases")).item(
                            Property::object(field_key!("item"))
                                .property(super::input_schema::condition(field_key!("condition")))
                                .property(Property::string(field_key!("port")).required()),
                        ),
                    )
                    .build()
            })
            .clone()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// N-way control-flow branch on an ordered list of field conditions.
///
/// Keyed `core.switch`. Evaluates cases in order and routes to the first
/// matching port, or `"default"` if no case matches.
#[derive(Debug)]
pub struct CoreSwitch;

impl nebula_action::action::Action for CoreSwitch {
    type Input = SwitchInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.switch"),
            nebula_action::metadata_name!("Switch"),
            "Routes execution to the first matching case port, or 'default' if none match",
        )
        .with_inputs(default_input_ports())
        .with_outputs(vec![OutputPort::Dynamic(DynamicPort {
            key: port_key!("case"),
            source_field: "cases".into(),
            label_field: Some("port".into()),
            include_fallback: true,
        })])
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for CoreSwitch {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CoreSwitch)
    }
}

impl ControlAction for CoreSwitch {
    #[instrument(
        name = "core.switch",
        skip_all,
        fields(case_count = input.cases.len())
    )]
    async fn evaluate(
        &self,
        input: SwitchInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        let SwitchInput { data, cases } = input;

        let data_object = normalize_data(data)
            .map_err(|err| ActionError::fatal(format!("core.switch: {err}")))?;

        // Evaluate cases in order. First-match-wins; Fatal propagates immediately.
        for case in &cases {
            if evaluate_condition(&data_object, &case.condition)? {
                let selected =
                    nebula_action::BranchKey::new(case.port.clone()).map_err(|validation_err| {
                        ActionError::fatal(format!(
                            "core.switch: case port name is invalid — {validation_err}"
                        ))
                    })?;
                return Ok(ControlOutcome::Branch {
                    selected,
                    output: data_object,
                });
            }
        }

        // No case matched (or cases list is empty) → route to "default".
        Ok(ControlOutcome::Branch {
            selected: branch_key!("default"),
            output: data_object,
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "switch_action_tests.rs"]
mod tests;
