use nebula_storage_port::dto::{ControlCommand, ControlMsg, JournalEntry};
use nebula_storage_port::{FencingToken, Scope, TransitionBatch, TransitionOutcome};

#[test]
fn builder_requires_core_fields_and_allows_empty_outbox_journal() {
    let b = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(3)
        .fencing(FencingToken::from_generation(7))
        .new_state(serde_json::json!({"s":"running"}))
        .build()
        .expect("all required fields present");
    assert!(b.outbox().is_empty() && b.journal().is_empty());
    assert_eq!(b.expected_version(), 3);
    assert_eq!(b.fencing().generation(), 7);
    assert_eq!(b.execution_id(), "01J");
    assert_eq!(b.scope().workspace_id, "w");
}

#[test]
fn builder_missing_required_field_is_configuration_error() {
    let r = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        // expected_version omitted
        .fencing(FencingToken::from_generation(1))
        .new_state(serde_json::json!({}))
        .build();
    assert!(r.is_err(), "missing expected_version must fail closed");
}

#[test]
fn builder_carries_outbox_and_journal() {
    let msg = ControlMsg {
        id: [1u8; 16],
        execution_id: "01J".into(),
        command: ControlCommand::Cancel,
        scope: Scope::new("w", "o"),
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    let je = JournalEntry {
        seq: None,
        payload: serde_json::json!({"e":"x"}),
    };
    let b = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(0)
        .fencing(FencingToken::from_generation(1))
        .new_state(serde_json::json!({}))
        .outbox(vec![msg])
        .journal(vec![je])
        .build()
        .expect("valid batch");
    assert_eq!(b.outbox().len(), 1);
    assert_eq!(b.journal().len(), 1);
}

#[test]
fn outcome_variants_exist() {
    let _ = TransitionOutcome::Applied { new_version: 4 };
    let _ = TransitionOutcome::VersionConflict { actual: 9 };
    let _ = TransitionOutcome::FencedOut;
}
