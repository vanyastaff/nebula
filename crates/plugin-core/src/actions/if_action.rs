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
//! ## Field scoping
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
    ActionContext, ActionError, ActionMetadata,
    control::{ControlAction, ControlInput, ControlOutcome},
    port::{OutputPort, default_input_ports},
};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::condition::{evaluate_condition, normalize_data};

// Re-export shared types so the existing public path
// `nebula_plugin_core::actions::if_action::{Condition, ConditionOp}` stays
// valid for any downstream code that imported them from here.
pub use crate::condition::{Condition, ConditionOp};

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

// Dynamic `data` and a runtime-polymorphic `condition.value` make a closed-
// form schema impossible to express. `ValidSchema::empty()` is the honest
// declaration; the module doc describes the expected structure out-of-band.
impl HasSchema for IfInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
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
/// use nebula_plugin_core::actions::if_action::{Condition, ConditionOp};
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

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.if"),
            "If",
            "Routes execution to 'true' or 'false' port based on a field condition",
        )
        .with_inputs(default_input_ports())
        .with_outputs(vec![OutputPort::flow("true"), OutputPort::flow("false")])
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
        fields(op = ?input.as_value().get("condition").and_then(|c| c.get("op")))
    )]
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome, ActionError> {
        // The GenericControlFactory dispatch path passes the raw JSON value
        // directly — it does NOT deserialize through `CoreIf::Input`. We must
        // manually deserialize to gain typed access to `condition` and `data`.
        let IfInput { data, condition } = serde_json::from_value::<IfInput>(input.into_value())
            .map_err(|deserialization_err| {
                ActionError::fatal(format!(
                    "core.if: invalid input shape — {deserialization_err}"
                ))
            })?;

        let data_object = normalize_data(data).map_err(|err| {
            // Prefix the normalisation error with the action key for context.
            ActionError::fatal(format!("core.if: {err}"))
        })?;
        let branch_taken = evaluate_condition(&data_object, &condition)?;
        let selected_port = if branch_taken { "true" } else { "false" }.to_string();

        Ok(ControlOutcome::Branch {
            selected: selected_port,
            output: data_object,
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_action::{
        control::{ControlInput, ControlOutcome},
        testing::TestContextBuilder,
    };
    use serde_json::json;

    use super::*;

    fn ctx() -> impl ActionContext {
        TestContextBuilder::new().build()
    }

    /// Drive `CoreIf::evaluate` directly from a JSON value that mirrors the
    /// wire shape the engine provides.
    async fn run_if(wire_json: Value) -> Result<ControlOutcome, ActionError> {
        let action = CoreIf;
        action
            .evaluate(ControlInput::from_value(wire_json), &ctx())
            .await
    }

    /// Assert the branch port selected by `ControlOutcome::Branch`.
    fn assert_branch(outcome: ControlOutcome, expected_port: &str) {
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

    // ── Eq ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eq_matching_value_routes_true() {
        let outcome = run_if(json!({
            "data": { "status": "active" },
            "condition": { "field": "status", "op": "eq", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn eq_non_matching_value_routes_false() {
        let outcome = run_if(json!({
            "data": { "status": "inactive" },
            "condition": { "field": "status", "op": "eq", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // RED witness: if missing-field defaulted to true for Eq, this test fails.
    #[tokio::test]
    async fn eq_missing_field_routes_false() {
        let outcome = run_if(json!({
            "data": { "other": 1 },
            "condition": { "field": "status", "op": "eq", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // ── Ne ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ne_different_value_routes_true() {
        let outcome = run_if(json!({
            "data": { "status": "inactive" },
            "condition": { "field": "status", "op": "ne", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn ne_same_value_routes_false() {
        let outcome = run_if(json!({
            "data": { "status": "active" },
            "condition": { "field": "status", "op": "ne", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn ne_missing_field_routes_true() {
        // Missing field is "not equal" to anything — routes true.
        let outcome = run_if(json!({
            "data": {},
            "condition": { "field": "status", "op": "ne", "value": "active" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    // ── Gt (numbers) ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn gt_numbers_true() {
        let outcome = run_if(json!({
            "data": { "score": 10 },
            "condition": { "field": "score", "op": "gt", "value": 5 }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn gt_numbers_false() {
        let outcome = run_if(json!({
            "data": { "score": 3 },
            "condition": { "field": "score", "op": "gt", "value": 5 }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // ── Gt (strings — lexicographic) ─────────────────────────────────────────

    #[tokio::test]
    async fn gt_strings_lexicographic_true() {
        let outcome = run_if(json!({
            "data": { "name": "beta" },
            "condition": { "field": "name", "op": "gt", "value": "alpha" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn gt_strings_lexicographic_false() {
        let outcome = run_if(json!({
            "data": { "name": "alpha" },
            "condition": { "field": "name", "op": "gt", "value": "beta" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // RED witness: if type-mismatch silently returned false instead of Fatal,
    // this would be `Ok("false")`, not `Err(Fatal)`.
    #[tokio::test]
    async fn gt_type_mismatch_returns_fatal() {
        let err = run_if(json!({
            "data": { "score": 10 },
            "condition": { "field": "score", "op": "gt", "value": "five" }
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected ActionError::Fatal for number vs string comparison; got: {err:?}"
        );
    }

    // RED witness: if missing field silently returned false instead of Fatal,
    // this would be `Ok("false")`, not `Err(Fatal)`.
    #[tokio::test]
    async fn gt_missing_field_returns_fatal() {
        let err = run_if(json!({
            "data": {},
            "condition": { "field": "score", "op": "gt", "value": 5 }
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected ActionError::Fatal for ordered comparison on missing field; got: {err:?}"
        );
    }

    // ── Gte / Lt / Lte ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn gte_equal_value_routes_true() {
        let outcome = run_if(json!({
            "data": { "n": 5 },
            "condition": { "field": "n", "op": "gte", "value": 5 }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn lt_smaller_routes_true() {
        let outcome = run_if(json!({
            "data": { "n": 3 },
            "condition": { "field": "n", "op": "lt", "value": 5 }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn lte_equal_routes_true() {
        let outcome = run_if(json!({
            "data": { "n": 5 },
            "condition": { "field": "n", "op": "lte", "value": 5 }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    // ── Exists / NotExists ────────────────────────────────────────────────────

    #[tokio::test]
    async fn exists_present_routes_true() {
        let outcome = run_if(json!({
            "data": { "key": null },
            "condition": { "field": "key", "op": "exists" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn exists_absent_routes_false() {
        let outcome = run_if(json!({
            "data": {},
            "condition": { "field": "missing", "op": "exists" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn not_exists_absent_routes_true() {
        let outcome = run_if(json!({
            "data": {},
            "condition": { "field": "missing", "op": "not_exists" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn not_exists_present_routes_false() {
        let outcome = run_if(json!({
            "data": { "key": "val" },
            "condition": { "field": "key", "op": "not_exists" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // ── Truthy ────────────────────────────────────────────────────────────────

    // RED witness: if `is_truthy` treated `false` as truthy, this would route "true".
    #[tokio::test]
    async fn truthy_false_literal_routes_false() {
        let outcome = run_if(json!({
            "data": { "flag": false },
            "condition": { "field": "flag", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_true_literal_routes_true() {
        let outcome = run_if(json!({
            "data": { "flag": true },
            "condition": { "field": "flag", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn truthy_null_routes_false() {
        let outcome = run_if(json!({
            "data": { "v": null },
            "condition": { "field": "v", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_zero_int_routes_false() {
        let outcome = run_if(json!({
            "data": { "n": 0 },
            "condition": { "field": "n", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_nonzero_int_routes_true() {
        let outcome = run_if(json!({
            "data": { "n": 42 },
            "condition": { "field": "n", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn truthy_empty_string_routes_false() {
        let outcome = run_if(json!({
            "data": { "s": "" },
            "condition": { "field": "s", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_non_empty_string_routes_true() {
        let outcome = run_if(json!({
            "data": { "s": "hello" },
            "condition": { "field": "s", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn truthy_empty_array_routes_false() {
        let outcome = run_if(json!({
            "data": { "arr": [] },
            "condition": { "field": "arr", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_empty_object_routes_false() {
        let outcome = run_if(json!({
            "data": { "obj": {} },
            "condition": { "field": "obj", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    #[tokio::test]
    async fn truthy_non_empty_array_routes_true() {
        let outcome = run_if(json!({
            "data": { "arr": [1] },
            "condition": { "field": "arr", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    #[tokio::test]
    async fn truthy_non_empty_object_routes_true() {
        let outcome = run_if(json!({
            "data": { "obj": { "k": 1 } },
            "condition": { "field": "obj", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "true");
    }

    // RED witness: if missing field were treated as truthy, this would route "true".
    #[tokio::test]
    async fn truthy_missing_field_routes_false() {
        let outcome = run_if(json!({
            "data": {},
            "condition": { "field": "absent", "op": "truthy" }
        }))
        .await
        .unwrap();
        assert_branch(outcome, "false");
    }

    // ── Data passthrough ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn data_passes_through_on_true_branch() {
        let data = json!({ "score": 10, "label": "high" });
        let outcome = run_if(json!({
            "data": data,
            "condition": { "field": "score", "op": "gt", "value": 5 }
        }))
        .await
        .unwrap();
        let output = branch_output(outcome);
        assert_eq!(
            output, data,
            "data must be passed through unchanged on true branch"
        );
    }

    #[tokio::test]
    async fn data_passes_through_on_false_branch() {
        let data = json!({ "score": 2, "label": "low" });
        let outcome = run_if(json!({
            "data": data,
            "condition": { "field": "score", "op": "gt", "value": 5 }
        }))
        .await
        .unwrap();
        let output = branch_output(outcome);
        assert_eq!(
            output, data,
            "data must be passed through unchanged on false branch"
        );
    }

    #[tokio::test]
    async fn null_data_produces_null_output() {
        // null data normalises to {}, so the output is an empty object.
        let outcome = run_if(json!({
            "data": null,
            "condition": { "field": "x", "op": "exists" }
        }))
        .await
        .unwrap();
        let output = branch_output(outcome);
        assert_eq!(output, json!({}));
    }

    #[tokio::test]
    async fn absent_data_produces_empty_object_output() {
        let outcome = run_if(json!({
            "condition": { "field": "x", "op": "not_exists" }
        }))
        .await
        .unwrap();
        // absent data → {}, "x" is not_exists → true; output is {}
        assert_branch(outcome.clone(), "true");
        let output = branch_output(outcome);
        assert_eq!(output, json!({}));
    }

    // ── Non-object data → Fatal (all operators) ───────────────────────────────

    #[tokio::test]
    async fn non_object_data_array_returns_fatal() {
        let err = run_if(json!({
            "data": [1, 2, 3],
            "condition": { "field": "x", "op": "eq", "value": 1 }
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for array data; got: {err:?}"
        );
    }

    // Locks the contract: normalize_data rejects non-objects BEFORE operator
    // dispatch, so even exists (which needs no value) must return Fatal on
    // array data.
    #[tokio::test]
    async fn non_object_data_with_exists_returns_fatal() {
        let err = run_if(json!({
            "data": true,
            "condition": { "field": "x", "op": "exists" }
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for bool data with exists op; got: {err:?}"
        );
    }

    // ── Missing condition.value → Fatal for comparison ops ────────────────────

    // RED witness: with the old `unwrap_or(&Value::Null)` this evaluated as
    // "x == null" and returned Ok("false") instead of Err(Fatal).
    #[tokio::test]
    async fn eq_missing_value_returns_fatal() {
        let err = run_if(json!({
            "data": { "x": "hello" },
            "condition": { "field": "x", "op": "eq" }   // no "value" key
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal when condition.value is absent for Eq; got: {err:?}"
        );
    }

    // RED witness: with the old `unwrap_or(&Value::Null)` this fell through to
    // evaluate_ordered comparing 10 vs null (type mismatch Fatal) — wrong error
    // *source*. Now it errors before even inspecting the field.
    #[tokio::test]
    async fn gt_missing_value_returns_fatal() {
        let err = run_if(json!({
            "data": { "score": 10 },
            "condition": { "field": "score", "op": "gt" }   // no "value" key
        }))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal when condition.value is absent for Gt; got: {err:?}"
        );
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn action_key_is_core_dot_if() {
        use nebula_action::action::Action;
        assert_eq!(CoreIf::metadata().base.key.as_str(), "core.if");
    }

    #[test]
    fn metadata_has_two_output_ports_true_and_false() {
        use nebula_action::action::Action;
        let meta = CoreIf::metadata();
        let port_keys: Vec<&str> = meta.outputs.iter().map(OutputPort::key).collect();
        assert!(
            port_keys.contains(&"true"),
            "outputs must include 'true'; got: {port_keys:?}"
        );
        assert!(
            port_keys.contains(&"false"),
            "outputs must include 'false'; got: {port_keys:?}"
        );
        assert_eq!(
            port_keys.len(),
            2,
            "exactly 2 output ports; got: {port_keys:?}"
        );
    }

    #[test]
    fn action_kind_is_control_after_factory_stamp() {
        use nebula_action::factory::GenericControlFactory;
        let factory = GenericControlFactory::<CoreIf>::new();
        use nebula_action::ActionFactory;
        assert_eq!(
            factory.metadata().kind,
            nebula_action::metadata::ActionKind::Control,
            "GenericControlFactory must stamp ActionKind::Control on CoreIf"
        );
    }

    // ── Serde round-trips ─────────────────────────────────────────────────────

    #[test]
    fn condition_op_serde_roundtrip() {
        for op in [
            ConditionOp::Eq,
            ConditionOp::Ne,
            ConditionOp::Gt,
            ConditionOp::Gte,
            ConditionOp::Lt,
            ConditionOp::Lte,
            ConditionOp::Exists,
            ConditionOp::NotExists,
            ConditionOp::Truthy,
        ] {
            let serialized = serde_json::to_string(&op).unwrap();
            let round_tripped: ConditionOp = serde_json::from_str(&serialized).unwrap();
            assert_eq!(
                round_tripped, op,
                "ConditionOp::{op:?} must survive a serde round-trip"
            );
        }
    }
}
