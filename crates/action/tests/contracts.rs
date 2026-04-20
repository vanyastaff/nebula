//! Contract tests for nebula-action (Phase 1: Compatibility Contracts).
//!
//! These tests freeze the JSON serialization of boundary types used by
//! engine/runtime/API. Changing the output format is a breaking change —
//! update expected values intentionally and document in MIGRATION.md.

use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use nebula_action::{
    ActionCategory, ActionOutput, ActionResult, BreakReason, DynamicPort, FlowKind, InputPort,
    OutputPort, SupportPort, TerminationReason, WaitCondition,
};
use nebula_core::id::ExecutionId;

#[test]
fn action_output_value_serialization_contract() {
    let out = ActionOutput::<i32>::Value(42);
    let json = serde_json::to_string(&out).unwrap();
    assert_eq!(json, r#"{"type":"Value","data":42}"#);
    let back: ActionOutput<i32> = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ActionOutput::Value(42)));
}

#[test]
fn action_output_empty_serialization_contract() {
    let out: ActionOutput<serde_json::Value> = ActionOutput::Empty;
    let json = serde_json::to_string(&out).unwrap();
    assert_eq!(json, r#"{"type":"Empty"}"#);
    let back: ActionOutput<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ActionOutput::Empty));
}

#[test]
fn flow_kind_serialization_contract() {
    let cases = [
        (FlowKind::Main, r#""main""#),
        (FlowKind::Error, r#""error""#),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "FlowKind serialization changed for {value:?}"
        );
    }
}

#[test]
fn support_port_serialization_contract() {
    let port = InputPort::Support(SupportPort {
        key: "model".to_string(),
        name: "AI Model".to_string(),
        description: "Primary model input".to_string(),
        required: true,
        multi: false,
        filter: Default::default(),
    });
    let json = serde_json::to_string(&port).unwrap();
    assert_eq!(
        json,
        r#"{"type":"support","key":"model","name":"AI Model","description":"Primary model input","required":true,"multi":false,"filter":{}}"#
    );
    let back: InputPort = serde_json::from_str(&json).unwrap();
    assert_eq!(back, port);
}

#[test]
fn dynamic_port_serialization_contract() {
    let port = OutputPort::Dynamic(DynamicPort {
        key: "rule".to_string(),
        source_field: "rules".to_string(),
        label_field: Some("label".to_string()),
        include_fallback: true,
    });
    let json = serde_json::to_string(&port).unwrap();
    assert_eq!(
        json,
        r#"{"type":"dynamic","key":"rule","source_field":"rules","label_field":"label","include_fallback":true}"#
    );
    let back: OutputPort = serde_json::from_str(&json).unwrap();
    assert_eq!(back, port);
}

fn assert_result_roundtrip(case: ActionResult<serde_json::Value>) {
    let json = serde_json::to_string(&case).unwrap();
    let back: ActionResult<serde_json::Value> = serde_json::from_str(&json).unwrap();
    let v1: serde_json::Value = serde_json::from_str(&json).unwrap();
    let v2: serde_json::Value = serde_json::to_value(&back).unwrap();
    assert_eq!(v2, v1, "ActionResult serialization changed");
}

#[test]
fn action_result_serialization_contract_all_variants_roundtrip() {
    assert_result_roundtrip(ActionResult::Success {
        output: ActionOutput::Value(serde_json::json!({"ok": true})),
    });

    assert_result_roundtrip(ActionResult::Skip {
        reason: "filtered".to_string(),
        output: Some(ActionOutput::Value(serde_json::json!({"reason": "test"}))),
    });

    assert_result_roundtrip(ActionResult::Continue {
        output: ActionOutput::Value(serde_json::json!({"step": 1})),
        progress: Some(0.5),
        delay: Some(Duration::from_millis(250)),
    });

    assert_result_roundtrip(ActionResult::Break {
        output: ActionOutput::Value(serde_json::json!({"done": true})),
        reason: BreakReason::ConditionMet,
    });

    let mut alternatives = HashMap::new();
    alternatives.insert(
        "true".to_string(),
        ActionOutput::Value(serde_json::json!({"v": 1})),
    );
    alternatives.insert(
        "false".to_string(),
        ActionOutput::Value(serde_json::json!({"v": 0})),
    );
    assert_result_roundtrip(ActionResult::Branch {
        selected: "true".to_string(),
        output: ActionOutput::Value(serde_json::json!({"selected": "true"})),
        alternatives,
    });

    assert_result_roundtrip(ActionResult::Route {
        port: "main".to_string(),
        data: ActionOutput::Value(serde_json::json!({"routed": true})),
    });

    let mut outputs = HashMap::new();
    outputs.insert(
        "main".to_string(),
        ActionOutput::Value(serde_json::json!({"main": true})),
    );
    outputs.insert(
        "audit".to_string(),
        ActionOutput::Value(serde_json::json!({"audit": "ok"})),
    );
    assert_result_roundtrip(ActionResult::MultiOutput {
        outputs,
        main_output: Some(ActionOutput::Value(serde_json::json!({"main": true}))),
    });

    assert_result_roundtrip(ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_millis(1500),
        },
        timeout: Some(Duration::from_secs(30)),
        partial_output: Some(ActionOutput::Value(serde_json::json!({"partial": true}))),
    });

    #[cfg(feature = "unstable-retry-scheduler")]
    assert_result_roundtrip(ActionResult::Retry {
        after: Duration::from_millis(5000),
        reason: "backoff".to_string(),
    });
}

#[test]
fn action_result_duration_millis_wire_format_contract() {
    let wait = ActionResult::<serde_json::Value>::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_millis(250),
        },
        timeout: Some(Duration::from_secs(5)),
        partial_output: None,
    };
    let json = serde_json::to_string(&wait).unwrap();
    assert_eq!(
        json,
        r#"{"type":"Wait","condition":{"type":"Duration","duration":250},"timeout":5000,"partial_output":null}"#
    );
}

#[cfg(feature = "unstable-retry-scheduler")]
#[test]
fn action_result_retry_wire_format_contract() {
    let retry = ActionResult::<serde_json::Value>::Retry {
        after: Duration::from_millis(1234),
        reason: "retry".to_string(),
    };
    let json = serde_json::to_string(&retry).unwrap();
    assert_eq!(json, r#"{"type":"Retry","after":1234,"reason":"retry"}"#);
}

#[test]
fn wait_condition_serialization_contract() {
    let cases = [
        WaitCondition::Webhook {
            callback_id: "cb-1".to_string(),
        },
        WaitCondition::Until {
            datetime: Utc::now(),
        },
        WaitCondition::Duration {
            duration: Duration::from_secs(2),
        },
        WaitCondition::Approval {
            approver: "ops".to_string(),
            message: "approve".to_string(),
        },
        WaitCondition::Execution {
            execution_id: ExecutionId::new(),
        },
    ];
    for value in cases {
        let json = serde_json::to_string(&value).unwrap();
        let back: WaitCondition = serde_json::from_str(&json).unwrap();
        let json_back = serde_json::to_string(&back).unwrap();
        assert_eq!(json_back, json, "WaitCondition serialization changed");
    }
}

#[test]
fn break_reason_serialization_contract() {
    let cases = [
        BreakReason::Completed,
        BreakReason::MaxIterations,
        BreakReason::ConditionMet,
        BreakReason::Custom("custom".to_string()),
    ];
    for value in cases {
        let json = serde_json::to_string(&value).unwrap();
        let back: BreakReason = serde_json::from_str(&json).unwrap();
        let json_back = serde_json::to_string(&back).unwrap();
        assert_eq!(json_back, json, "BreakReason serialization changed");
    }
}

// ── Drop / Terminate / TerminationReason contracts ──────────────────────────
//
// These freeze the JSON shape for the new control-flow variants added in
// Phase 0 of the ControlAction work. Changing any of these values without
// a documented migration is a breaking change — the engine, audit log, and
// downstream persistence layers rely on the shape staying stable.

#[test]
fn action_result_drop_serialization_contract() {
    let drop_with_reason: ActionResult<i32> = ActionResult::drop_with_reason("filtered");
    let json = serde_json::to_string(&drop_with_reason).unwrap();
    assert_eq!(json, r#"{"type":"Drop","reason":"filtered"}"#);
    let back: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    assert!(back.is_drop());

    let drop_no_reason: ActionResult<i32> = ActionResult::drop_item();
    let json = serde_json::to_string(&drop_no_reason).unwrap();
    assert_eq!(json, r#"{"type":"Drop","reason":null}"#);
}

#[test]
fn action_result_terminate_success_serialization_contract() {
    let terminate: ActionResult<i32> =
        ActionResult::terminate_success(Some("done early".to_string()));
    let json = serde_json::to_string(&terminate).unwrap();
    assert_eq!(
        json,
        r#"{"type":"Terminate","reason":{"type":"Success","note":"done early"}}"#
    );
    let back: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    assert!(back.is_terminate());
}

#[test]
fn action_result_terminate_success_no_note_serialization_contract() {
    let terminate: ActionResult<i32> = ActionResult::terminate_success(None);
    let json = serde_json::to_string(&terminate).unwrap();
    assert_eq!(
        json,
        r#"{"type":"Terminate","reason":{"type":"Success","note":null}}"#
    );
}

#[test]
fn action_result_terminate_failure_serialization_contract() {
    let terminate: ActionResult<i32> =
        ActionResult::terminate_failure("E_VALIDATION", "field missing");
    let json = serde_json::to_string(&terminate).unwrap();
    assert_eq!(
        json,
        r#"{"type":"Terminate","reason":{"type":"Failure","code":"E_VALIDATION","message":"field missing"}}"#
    );
    let back: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    match back {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "E_VALIDATION");
                assert_eq!(message, "field missing");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
            _ => panic!("unexpected TerminationReason variant"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn termination_reason_round_trip_contract() {
    let cases: Vec<TerminationReason> = vec![
        TerminationReason::Success { note: None },
        TerminationReason::Success {
            note: Some("early stop".to_string()),
        },
        TerminationReason::Failure {
            code: "E_FAIL".into(),
            message: "boom".to_string(),
        },
    ];
    for original in cases {
        let json = serde_json::to_string(&original).unwrap();
        let back: TerminationReason = serde_json::from_str(&json).unwrap();
        let json_back = serde_json::to_string(&back).unwrap();
        assert_eq!(json_back, json, "TerminationReason serialization changed");
    }
}

// ── ActionCategory contract ────────────────────────────────────────────────
//
// Metadata discriminator added in Phase 0. Serialized as snake_case tag;
// changing any variant name is a breaking change for UI editor / audit log.

#[test]
fn action_category_serialization_contract() {
    let cases = [
        (ActionCategory::Data, r#""data""#),
        (ActionCategory::Control, r#""control""#),
        (ActionCategory::Trigger, r#""trigger""#),
        (ActionCategory::Resource, r#""resource""#),
        (ActionCategory::Agent, r#""agent""#),
        (ActionCategory::Terminal, r#""terminal""#),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "ActionCategory serialization changed for {value:?}"
        );
        let back: ActionCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value);
    }
}
