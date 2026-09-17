use std::sync::OnceLock;

use nebula_core::{Dependencies, action_key};

use super::*;
use crate::{
    branch_key,
    port::{OutputPort, default_input_ports, default_output_ports},
    port_key,
    testing::{TestActionContext, TestContextBuilder},
};

fn make_ctx() -> TestActionContext {
    TestContextBuilder::new().build()
}

async fn execute(
    handler: &(impl ControlHandle + ?Sized),
    input: Value,
    context: &dyn ActionContext,
) -> Result<ActionResult<Value>, ActionError> {
    let input = handler.prepare_input(ActionInput::Raw(input))?;
    handler.dispatch(input, context).await
}

// ── ControlOutcome → ActionResult ──────────────────────────────

#[test]
fn outcome_branch_desugars_to_action_result_branch() {
    let outcome = ControlOutcome::Branch {
        selected: branch_key!("true"),
        output: serde_json::json!({"v": 1}),
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::Branch {
            selected,
            output,
            alternatives,
        } => {
            assert_eq!(selected.as_str(), "true");
            assert_eq!(output.as_value(), Some(&serde_json::json!({"v": 1})));
            assert!(alternatives.is_empty());
        },
        _ => panic!("expected Branch"),
    }
}

#[test]
fn outcome_route_desugars_to_multi_output() {
    let outcome = ControlOutcome::Route {
        ports: std::collections::HashMap::from([
            (port_key!("high"), serde_json::json!(1)),
            (port_key!("low"), serde_json::json!(2)),
        ]),
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::MultiOutput {
            outputs,
            main_output,
        } => {
            assert_eq!(outputs.len(), 2);
            assert!(outputs.contains_key("high"));
            assert!(outputs.contains_key("low"));
            assert!(main_output.is_none());
        },
        _ => panic!("expected MultiOutput"),
    }
}

#[test]
fn outcome_pass_desugars_to_success() {
    let outcome = ControlOutcome::Pass {
        output: serde_json::json!({"ok": true}),
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::Success { output } => {
            assert_eq!(output.as_value(), Some(&serde_json::json!({"ok": true})));
        },
        _ => panic!("expected Success"),
    }
}

#[test]
fn outcome_drop_desugars_to_drop() {
    let outcome = ControlOutcome::Drop {
        reason: Some("rate limit".into()),
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::Drop { reason } => {
            assert_eq!(reason.as_deref(), Some("rate limit"));
        },
        _ => panic!("expected Drop"),
    }
}

#[test]
fn outcome_terminate_success_desugars_to_terminate() {
    let outcome = ControlOutcome::Terminate {
        reason: TerminationReason::Success {
            note: Some("done".into()),
        },
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Success { note } => assert_eq!(note.as_deref(), Some("done")),
            TerminationReason::Failure { .. } => panic!("expected Success"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn outcome_terminate_failure_desugars_to_terminate() {
    let outcome = ControlOutcome::Terminate {
        reason: TerminationReason::Failure {
            code: "E_BAD".into(),
            message: "nope".into(),
        },
    };
    let result: ActionResult<Value> = outcome.into();
    match result {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "E_BAD");
                assert_eq!(message, "nope");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
        },
        _ => panic!("expected Terminate"),
    }
}

// ── ControlActionAdapter smoke test ────────────────────────────

/// Minimal control action used for smoke tests.
struct TestIf;

#[derive(serde::Deserialize, serde::Serialize, nebula_schema::Schema)]
struct TestIfInput {
    condition: bool,
    payload: Option<Value>,
}

impl TestIf {
    fn new() -> Self {
        Self
    }
}

impl Action for TestIf {
    type Input = TestIfInput;
    type Output = TestIfInput;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            action_key!("test.if"),
            crate::metadata_name!("TestIf"),
            "Binary branch",
        )
        .with_inputs(default_input_ports())
        .with_outputs(vec![
            OutputPort::flow(port_key!("true")),
            OutputPort::flow(port_key!("false")),
        ])
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for TestIf {
    async fn evaluate(
        &self,
        input: TestIfInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<TestIfInput>, ActionError> {
        let selected = if input.condition {
            branch_key!("true")
        } else {
            branch_key!("false")
        };
        Ok(ControlOutcome::Branch {
            selected,
            output: input,
        })
    }
}

/// Terminal-only action for category-inference smoke tests.
struct TestStop;

impl TestStop {
    fn new() -> Self {
        Self
    }
}

impl Action for TestStop {
    type Input = Value;
    type Output = Value;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            action_key!("test.stop"),
            crate::metadata_name!("TestStop"),
            "Terminate",
        )
        .with_outputs(Vec::new())
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for TestStop {
    async fn evaluate(
        &self,
        _input: Value,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        Ok(ControlOutcome::Terminate {
            reason: TerminationReason::Success {
                note: Some("stopped".into()),
            },
        })
    }
}

#[test]
fn adapter_stamps_control_kind() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let meta = adapter.metadata();
    assert_eq!(meta.kind(), ActionKind::Control);
    assert!(
        !meta.outputs().is_empty(),
        "a routing control node declares output ports"
    );
}

#[test]
fn adapter_terminal_node_keeps_control_kind_with_empty_outputs() {
    // A terminal control node (Stop/Fail) keeps `ActionKind::Control`;
    // terminality is carried structurally by the empty `outputs` set, which
    // is what the workflow validator reads to recognise the graph sink.
    let adapter =
        ControlActionAdapter::new(TestStop::new()).expect("valid test catalog definition");
    let meta = adapter.metadata();
    assert_eq!(meta.kind(), ActionKind::Control);
    assert!(
        meta.outputs().is_empty(),
        "a terminal control node declares no output ports"
    );
}

#[test]
fn adapter_preserves_action_key() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    assert_eq!(
        adapter.metadata().base().key().clone(),
        action_key!("test.if")
    );
}

#[tokio::test]
async fn adapter_executes_through_stateless_handler() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let ctx = make_ctx();

    let result = execute(
        &adapter,
        serde_json::json!({ "condition": true, "payload": 42 }),
        &ctx,
    )
    .await
    .unwrap();

    match result {
        ActionResult::Branch {
            selected, output, ..
        } => {
            assert_eq!(selected.as_str(), "true");
            assert_eq!(
                output.as_value(),
                Some(&serde_json::json!({ "condition": true, "payload": 42 }))
            );
        },
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn adapter_evaluates_false_branch() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let ctx = make_ctx();

    let result = execute(&adapter, serde_json::json!({ "condition": false }), &ctx)
        .await
        .unwrap();

    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected.as_str(), "false"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn adapter_propagates_validation_error_on_missing_field() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let ctx = make_ctx();

    let err = execute(&adapter, serde_json::json!({}), &ctx)
        .await
        .unwrap_err();

    assert!(matches!(err, ActionError::Validation { .. }));
}

#[tokio::test]
async fn adapter_stop_action_returns_terminate() {
    let adapter =
        ControlActionAdapter::new(TestStop::new()).expect("valid test catalog definition");
    let ctx = make_ctx();

    let result = execute(&adapter, serde_json::json!({}), &ctx)
        .await
        .unwrap();

    match result {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Success { note } => {
                assert_eq!(note.as_deref(), Some("stopped"));
            },
            TerminationReason::Failure { .. } => panic!("expected Success"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn adapter_is_dyn_compatible() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let _: Arc<dyn ControlHandle> = Arc::new(adapter);
}

#[test]
fn adapter_into_inner_returns_action() {
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    let key = adapter.metadata().base().key().clone();
    let _action = adapter.into_inner();
    assert_eq!(key, action_key!("test.if"));
}

#[test]
fn adapter_preserves_original_outputs_after_stamp() {
    // The adapter only rewrites `category`; it must not touch `outputs`
    // or any other metadata field.
    let adapter = ControlActionAdapter::new(TestIf::new()).expect("valid test catalog definition");
    assert_eq!(
        adapter.metadata().outputs(),
        [
            OutputPort::flow(port_key!("true")),
            OutputPort::flow(port_key!("false")),
        ]
    );
}

// ── default_output_ports parity ────────────────────────────────

#[test]
fn default_control_node_has_non_empty_outputs() {
    // A control node built with default ports (one main output) must not
    // look like a terminal sink: terminality is read from an empty
    // `outputs` set, so the defaults must stay non-empty.
    assert!(!default_output_ports().is_empty());
}
