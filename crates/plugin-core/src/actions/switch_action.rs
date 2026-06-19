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
//! ## Field scoping
//!
//! `case.condition.field` is a top-level key in `data`, not a JSON pointer.
//! A dot in the field name is literal.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{
    ActionContext, ActionError, ActionMetadata,
    control::{ControlAction, ControlInput, ControlOutcome},
    port::{DynamicPort, OutputPort, default_input_ports},
};
use nebula_core::action_key;
use nebula_schema::HasSchema;
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

// Dynamic `data` and runtime-polymorphic `condition.value` make a closed-form
// schema impossible. `ValidSchema::empty()` is the honest declaration; the
// module doc describes the expected structure out-of-band.
impl HasSchema for SwitchInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
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

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.switch"),
            "Switch",
            "Routes execution to the first matching case port, or 'default' if none match",
        )
        .with_inputs(default_input_ports())
        .with_outputs(vec![OutputPort::Dynamic(DynamicPort {
            key: "case".into(),
            source_field: "cases".into(),
            label_field: Some("port".into()),
            include_fallback: true,
        })])
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
        fields(case_count = input.as_value()
            .get("cases")
            .and_then(|c| c.as_array())
            .map(Vec::len)
            .unwrap_or(0))
    )]
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome, ActionError> {
        // The GenericControlFactory dispatch path passes the raw JSON value
        // directly — it does NOT go through `CoreSwitch::Input`. Manual
        // deserialization gives us typed access to `data` and `cases`.
        let SwitchInput { data, cases } = serde_json::from_value::<SwitchInput>(input.into_value())
            .map_err(|deserialization_err| {
                ActionError::fatal(format!(
                    "core.switch: invalid input shape — {deserialization_err}"
                ))
            })?;

        let data_object = normalize_data(data)
            .map_err(|err| ActionError::fatal(format!("core.switch: {err}")))?;

        // Evaluate cases in order. First-match-wins; Fatal propagates immediately.
        for case in &cases {
            if evaluate_condition(&data_object, &case.condition)? {
                return Ok(ControlOutcome::Branch {
                    selected: case.port.clone(),
                    output: data_object,
                });
            }
        }

        // No case matched (or cases list is empty) → route to "default".
        Ok(ControlOutcome::Branch {
            selected: "default".to_string(),
            output: data_object,
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_action::{
        ActionFactory,
        control::{ControlInput, ControlOutcome},
        testing::TestContextBuilder,
    };
    use serde_json::json;

    use super::*;

    fn ctx() -> impl ActionContext {
        TestContextBuilder::new().build()
    }

    async fn run_switch(wire_json: Value) -> Result<ControlOutcome, ActionError> {
        CoreSwitch
            .evaluate(ControlInput::from_value(wire_json), &ctx())
            .await
    }

    /// Assert the selected port from `ControlOutcome::Branch`.
    fn assert_port(outcome: &ControlOutcome, expected_port: &str) {
        match outcome {
            ControlOutcome::Branch { selected, .. } => {
                assert_eq!(
                    selected, expected_port,
                    "expected port `{expected_port}`, got `{selected}`"
                );
            },
            other => panic!("expected ControlOutcome::Branch, got {other:?}"),
        }
    }

    /// Extract the output value from `ControlOutcome::Branch`.
    fn branch_output(outcome: ControlOutcome) -> Value {
        match outcome {
            ControlOutcome::Branch { output, .. } => output,
            other => panic!("expected ControlOutcome::Branch, got {other:?}"),
        }
    }

    // ── First-match-wins / short-circuit ─────────────────────────────────────

    /// RED witness for short-circuit: case[1] has a `gt` on a MISSING field —
    /// if the engine evaluates it, it fires a Fatal (ordered comparison on
    /// missing field). The test proves case[1] is never reached because case[0]
    /// matched first.
    #[tokio::test]
    async fn first_case_matches_selects_port_a_and_short_circuits() {
        let outcome = run_switch(json!({
            "data": { "status": "active" },
            "cases": [
                { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" },
                // case[1]: gt on "missing_field" — Fatal if evaluated.
                { "condition": { "field": "missing_field", "op": "gt", "value": 0 }, "port": "b" }
            ]
        }))
        .await
        // If both cases were evaluated, case[1] would produce Fatal and this .unwrap() would fail.
        .unwrap();

        assert_port(&outcome, "a");
    }

    /// RED witness: swap expected port — confirms the assertion distinguishes "a" from "b".
    #[tokio::test]
    async fn second_case_wins_when_first_does_not_match() {
        let outcome = run_switch(json!({
            "data": { "status": "inactive", "score": 95 },
            "cases": [
                { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" },
                { "condition": { "field": "score",  "op": "gt", "value": 90 },       "port": "b" }
            ]
        }))
        .await
        .unwrap();

        assert_port(&outcome, "b");
    }

    // ── Default fallback ──────────────────────────────────────────────────────

    /// RED witness: if "default" were never returned (e.g. an error or wrong
    /// port), the assert_port would catch the mismatch.
    #[tokio::test]
    async fn no_case_matches_selects_default() {
        let outcome = run_switch(json!({
            "data": { "status": "pending" },
            "cases": [
                { "condition": { "field": "status", "op": "eq", "value": "active" },   "port": "a" },
                { "condition": { "field": "status", "op": "eq", "value": "inactive" }, "port": "b" }
            ]
        }))
        .await
        .unwrap();

        assert_port(&outcome, "default");
    }

    #[tokio::test]
    async fn empty_cases_selects_default() {
        let outcome = run_switch(json!({
            "data": { "x": 1 },
            "cases": []
        }))
        .await
        .unwrap();

        assert_port(&outcome, "default");
    }

    // ── Fatal propagation ─────────────────────────────────────────────────────

    /// A case condition that errors (Gt on missing field) propagates immediately
    /// as Fatal — the switch does not skip to the next case.
    #[tokio::test]
    async fn case_condition_fatal_propagates() {
        let err = run_switch(json!({
            "data": { "other": 1 },
            "cases": [
                // "score" is missing → Fatal for ordered comparison.
                { "condition": { "field": "score", "op": "gt", "value": 0 }, "port": "a" }
            ]
        }))
        .await
        .unwrap_err();

        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected ActionError::Fatal when a case condition is Fatal; got: {err:?}"
        );
    }

    // ── Non-object data → Fatal ───────────────────────────────────────────────

    #[tokio::test]
    async fn non_object_data_returns_fatal() {
        let err = run_switch(json!({
            "data": [1, 2, 3],
            "cases": [
                { "condition": { "field": "x", "op": "exists" }, "port": "a" }
            ]
        }))
        .await
        .unwrap_err();

        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for array data; got: {err:?}"
        );
    }

    // ── Null / absent data ────────────────────────────────────────────────────

    /// Null data normalises to {} — empty cases → "default", output is {}.
    ///
    /// RED witness: if null data produced Fatal instead of normalising to {},
    /// this would Err rather than returning default.
    #[tokio::test]
    async fn null_data_with_no_cases_selects_default_with_empty_output() {
        let outcome = run_switch(json!({
            "data": null,
            "cases": []
        }))
        .await
        .unwrap();

        assert_port(&outcome, "default");
        let output = branch_output(outcome);
        assert_eq!(
            output,
            json!({}),
            "normalized null data must produce {{}} output"
        );
    }

    // ── Duplicate port names ──────────────────────────────────────────────────

    /// Two cases sharing the same port name route to that port without error.
    ///
    /// This test proves that duplicate port names are accepted (no panic or
    /// Fatal) and that the selected port is the shared name. It does NOT
    /// distinguish which case fired — both conditions match `"tier"` and the
    /// data passthrough is identical either way. First-match-wins behaviour is
    /// separately proven by `first_case_matches_selects_port_a_and_short_circuits`.
    #[tokio::test]
    async fn duplicate_port_names_handled_without_error() {
        let outcome = run_switch(json!({
            "data": { "level": 5 },
            "cases": [
                { "condition": { "field": "level", "op": "gte", "value": 1 }, "port": "tier" },
                { "condition": { "field": "level", "op": "gte", "value": 3 }, "port": "tier" }
            ]
        }))
        .await
        .unwrap();

        assert_port(&outcome, "tier");
    }

    // ── Data passthrough ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn data_passes_through_on_matching_case() {
        let data = json!({ "status": "active", "score": 42 });
        let outcome = run_switch(json!({
            "data": data,
            "cases": [
                { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" }
            ]
        }))
        .await
        .unwrap();

        assert_port(&outcome, "a");
        let output = branch_output(outcome);
        assert_eq!(
            output, data,
            "data must pass through unchanged on a matched case"
        );
    }

    #[tokio::test]
    async fn data_passes_through_on_default() {
        let data = json!({ "status": "unknown" });
        let outcome = run_switch(json!({
            "data": data,
            "cases": [
                { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" }
            ]
        }))
        .await
        .unwrap();

        assert_port(&outcome, "default");
        let output = branch_output(outcome);
        assert_eq!(
            output, data,
            "data must pass through unchanged when routing to default"
        );
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn action_key_is_core_switch() {
        use nebula_action::action::Action;
        assert_eq!(CoreSwitch::metadata().base.key.as_str(), "core.switch");
    }

    #[test]
    fn metadata_has_one_dynamic_output_port_with_source_field_cases() {
        use nebula_action::action::Action;
        let meta = CoreSwitch::metadata();
        assert_eq!(
            meta.outputs.len(),
            1,
            "must have exactly one output port declaration"
        );
        match &meta.outputs[0] {
            OutputPort::Dynamic(dynamic_port) => {
                assert_eq!(
                    dynamic_port.source_field, "cases",
                    "dynamic port source_field must be 'cases'"
                );
                assert!(
                    dynamic_port.include_fallback,
                    "include_fallback must be true to generate the 'default' port"
                );
                assert_eq!(
                    dynamic_port.label_field.as_deref(),
                    Some("port"),
                    "label_field must be 'port'"
                );
            },
            other => panic!("expected OutputPort::Dynamic, got {other:?}"),
        }
    }

    #[test]
    fn action_kind_is_control_after_factory_stamp() {
        use nebula_action::factory::GenericControlFactory;
        let factory = GenericControlFactory::<CoreSwitch>::new();
        assert_eq!(
            factory.metadata().kind,
            nebula_action::metadata::ActionKind::Control,
            "GenericControlFactory must stamp ActionKind::Control on CoreSwitch"
        );
    }
}
