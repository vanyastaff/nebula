//! Contract tests for nebula-action (Phase 1: Compatibility Contracts).
//!
//! These tests freeze the JSON serialization of boundary types used by
//! engine/runtime/API. Changing the output format is a breaking change —
//! update expected values intentionally and document in MIGRATION.md.

use nebula_action::{ActionOutput, FlowKind};

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
            "FlowKind serialization changed for {:?}",
            value
        );
    }
}

// BreakReason and WaitCondition do not implement Serialize yet; when added,
// add contract tests here for their JSON shape.
