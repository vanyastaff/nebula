use nebula_storage_port::dto::{ControlMsg, ExecutionRecord, JournalEntry, NodeResultRecord};
use nebula_storage_port::{FencingToken, Scope};

#[test]
fn node_result_record_is_action_result_free_and_roundtrips() {
    let r = NodeResultRecord {
        kind_tag: "Value".into(),
        json: serde_json::json!({"k":1}),
        schema_version: 1,
    };
    let s = serde_json::to_string(&r).expect("serialize");
    let back: NodeResultRecord = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.schema_version, 1);
    assert_eq!(back.kind_tag, "Value");
}

#[test]
fn execution_record_roundtrips() {
    let rec = ExecutionRecord {
        id: "exe_1".into(),
        workflow_id: "wf_1".into(),
        scope: Scope::new("ws_1", "org_1"),
        version: 3,
        status: "Running".into(),
        state: serde_json::json!({"s":"running"}),
        lease_holder: Some("nbl_1".into()),
        fencing: Some(7),
        created_at: "2026-05-15T00:00:00Z".into(),
        updated_at: "2026-05-15T00:00:01Z".into(),
    };
    let s = serde_json::to_string(&rec).expect("serialize");
    let back: ExecutionRecord = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back, rec);
}

#[test]
fn control_msg_roundtrips_with_typed_16_byte_id() {
    use nebula_storage_port::dto::ControlCommand;
    let msg = ControlMsg {
        id: [7u8; 16],
        execution_id: "exe_1".into(),
        command: ControlCommand::Cancel,
        scope: Scope::new("ws_1", "org_1"),
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    let s = serde_json::to_string(&msg).expect("serialize");
    let back: ControlMsg = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.id, [7u8; 16]);
    assert_eq!(back.command, ControlCommand::Cancel);
}

#[test]
fn journal_entry_roundtrips() {
    let je = JournalEntry {
        seq: Some(1),
        payload: serde_json::json!({"event":"started"}),
    };
    let s = serde_json::to_string(&je).expect("serialize");
    let back: JournalEntry = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.seq, Some(1));
}

// Compile-time guard: a fresh FencingToken generation is comparable, proving
// the id seam stays usable from DTO-consuming code.
#[test]
fn fencing_token_seam_visible() {
    assert!(FencingToken::from_generation(0) < FencingToken::from_generation(1));
}
