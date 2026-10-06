use chrono::{TimeZone, Utc};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, JournalEntry};
use nebula_storage_port::{
    ExecutionListing, ExecutionListingStatus, ExecutionReferenceTransition, FencingToken, Scope,
    TransitionBatch, TransitionOutcome,
};

#[test]
fn builder_requires_core_fields_and_allows_empty_outbox_journal() {
    let b = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(3)
        .fencing(FencingToken::from_generation(7))
        .state(serde_json::json!({"s":"running"}), ExecutionListing::CREATED)
        .build()
        .expect("all required fields present");
    assert!(b.outbox().is_empty() && b.journal().is_empty());
    assert_eq!(b.expected_version(), 3);
    assert_eq!(b.fencing().generation(), 7);
    assert_eq!(b.execution_id(), "01J");
    assert_eq!(b.scope().workspace_id, "w");
    assert_eq!(b.listing(), ExecutionListing::CREATED);
}

#[test]
fn builder_missing_required_field_is_configuration_error() {
    let r = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        // expected_version omitted
        .fencing(FencingToken::from_generation(1))
        .state(serde_json::json!({}), ExecutionListing::CREATED)
        .build();
    assert!(r.is_err(), "missing expected_version must fail closed");
}

#[test]
fn builder_missing_state_is_configuration_error() {
    let r = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(0)
        .fencing(FencingToken::from_generation(1))
        .build();
    assert!(r.is_err(), "a batch without a snapshot must fail closed");
}

fn control_msg(scope: Scope) -> ControlMsg {
    ControlMsg {
        id: [1u8; 16],
        execution_id: "01J".into(),
        command: ControlCommand::Cancel,
        scope,
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    }
}

#[test]
fn builder_carries_outbox_and_journal() {
    let je = JournalEntry {
        seq: None,
        payload: serde_json::json!({"e":"x"}),
    };
    let b = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(0)
        .fencing(FencingToken::from_generation(1))
        .state(serde_json::json!({}), ExecutionListing::CREATED)
        .outbox(vec![control_msg(Scope::new("w", "o"))])
        .journal(vec![je])
        .build()
        .expect("valid batch");
    assert_eq!(b.outbox().len(), 1);
    assert_eq!(b.journal().len(), 1);
}

/// A decorator rebinding a batch must retarget every scoped row and keep
/// every other field — the hand-written rebuild it replaces dropped
/// `reference_transition`.
#[test]
fn rebound_batch_retargets_every_scope_and_keeps_every_other_field() {
    let finished = Utc.with_ymd_and_hms(2026, 10, 5, 12, 0, 0).unwrap();
    let listing = ExecutionListing::new(
        ExecutionListingStatus::Failed,
        Some(finished),
        Some(finished),
    );
    let original = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(4)
        .fencing(FencingToken::from_generation(2))
        .state(serde_json::json!({"x": 1}), listing)
        .outbox(vec![control_msg(Scope::new("w", "o"))])
        .journal(vec![JournalEntry {
            seq: None,
            payload: serde_json::json!({"e": "x"}),
        }])
        .reference_transition(ExecutionReferenceTransition::ReleaseLive)
        .build()
        .expect("valid batch");

    let bound = Scope::new("w2", "o2");
    let rebound = original.rebound_to(&bound);

    assert_eq!(rebound.scope(), &bound);
    assert!(rebound.outbox().iter().all(|row| row.scope == bound));
    assert_eq!(rebound.execution_id(), original.execution_id());
    assert_eq!(rebound.expected_version(), original.expected_version());
    assert_eq!(rebound.fencing(), original.fencing());
    assert_eq!(rebound.new_state(), original.new_state());
    assert_eq!(rebound.listing(), listing);
    assert_eq!(rebound.journal(), original.journal());
    assert_eq!(
        rebound.reference_transition(),
        Some(ExecutionReferenceTransition::ReleaseLive)
    );
}

#[test]
fn appended_journal_keeps_existing_rows_first() {
    let batch = TransitionBatch::builder()
        .scope(Scope::new("w", "o"))
        .execution_id("01J")
        .expected_version(0)
        .fencing(FencingToken::from_generation(1))
        .state(serde_json::json!({}), ExecutionListing::CREATED)
        .journal(vec![JournalEntry {
            seq: None,
            payload: serde_json::json!({"n": 1}),
        }])
        .build()
        .expect("valid batch");
    let appended = batch.with_appended_journal([JournalEntry {
        seq: None,
        payload: serde_json::json!({"n": 2}),
    }]);
    let order: Vec<_> = appended.journal().iter().map(|e| e.payload["n"].clone()).collect();
    assert_eq!(order, vec![serde_json::json!(1), serde_json::json!(2)]);
    assert_eq!(appended.listing(), batch.listing());
}

#[test]
fn outcome_variants_exist() {
    let _ = TransitionOutcome::Applied { new_version: 4 };
    let _ = TransitionOutcome::VersionConflict { actual: 9 };
    let _ = TransitionOutcome::FencedOut;
}
