use nebula_action::{ActionFactory, control::ControlOutcome, testing::TestContextBuilder};
use serde_json::json;

use super::*;

fn ctx() -> impl ActionContext {
    TestContextBuilder::new().build()
}

async fn run_switch(wire_json: Value) -> Result<ControlOutcome<Value>, ActionError> {
    let input = serde_json::from_value(wire_json)
        .expect("test input must match the declared SwitchInput wire shape");
    CoreSwitch.evaluate(input, &ctx()).await
}

/// Assert the selected port from `ControlOutcome::Branch`.
fn assert_port(outcome: &ControlOutcome<Value>, expected_port: &str) {
    match outcome {
        ControlOutcome::Branch { selected, .. } => {
            assert_eq!(
                selected.as_str(),
                expected_port,
                "expected port `{expected_port}`, got `{selected}`"
            );
        },
        other => panic!("expected ControlOutcome::Branch, got {other:?}"),
    }
}

/// Extract the output value from `ControlOutcome::Branch`.
fn branch_output(outcome: ControlOutcome<Value>) -> Value {
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
    let factory = nebula_action::GenericControlFactory::<CoreSwitch>::new()
        .expect("switch metadata must admit");
    assert_eq!(
        ActionFactory::metadata(&factory).base().key().as_str(),
        "core.switch"
    );
}

#[test]
fn metadata_has_one_dynamic_output_port_with_source_field_cases() {
    let factory = nebula_action::GenericControlFactory::<CoreSwitch>::new()
        .expect("switch metadata must admit");
    let metadata = ActionFactory::metadata(&factory);
    assert_eq!(
        metadata.outputs().len(),
        1,
        "must have exactly one output port declaration"
    );
    match &metadata.outputs()[0] {
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
    let factory = GenericControlFactory::<CoreSwitch>::new().expect("switch metadata must admit");
    assert_eq!(
        factory.metadata().kind(),
        nebula_action::metadata::ActionKind::Control,
        "GenericControlFactory must stamp ActionKind::Control on CoreSwitch"
    );
}
