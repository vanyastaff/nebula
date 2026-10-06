//! `TransitionBatch` construction. Every required part is a constructor
//! argument, so a batch missing one does not compile; these tests cover what
//! the constructor and the `with_*` methods carry.

use chrono::{TimeZone, Utc};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, JournalEntry};
use nebula_storage_port::{
    ExecutionListing, ExecutionListingStatus, ExecutionReferenceTransition, FencingToken, Scope,
    TransitionBatch, TransitionOutcome,
};

fn batch(listing: ExecutionListing) -> TransitionBatch {
    TransitionBatch::new(
        Scope::new("w", "o"),
        "01J",
        3,
        FencingToken::from_generation(7),
        serde_json::json!({"s": "running"}),
        listing,
    )
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

fn journal_entry(n: u64) -> JournalEntry {
    JournalEntry {
        seq: None,
        payload: serde_json::json!({ "n": n }),
    }
}

#[test]
fn constructor_carries_the_required_parts_and_starts_with_empty_extras() {
    let b = batch(ExecutionListing::CREATED);
    assert_eq!(b.scope().workspace_id, "w");
    assert_eq!(b.execution_id(), "01J");
    assert_eq!(b.expected_version(), 3);
    assert_eq!(b.fencing().generation(), 7);
    assert_eq!(b.new_state(), &serde_json::json!({"s": "running"}));
    assert_eq!(b.listing(), ExecutionListing::CREATED);
    assert!(b.outbox().is_empty() && b.journal().is_empty() && b.resume_tokens().is_empty());
    assert_eq!(b.reference_transition(), None);
}

#[test]
fn with_methods_carry_outbox_journal_and_reference_transition() {
    let b = batch(ExecutionListing::CREATED)
        .with_outbox(vec![control_msg(Scope::new("w", "o"))])
        .with_journal(vec![journal_entry(1)])
        .with_reference_transition(ExecutionReferenceTransition::ReleaseLive);
    assert_eq!(b.outbox().len(), 1);
    assert_eq!(b.journal().len(), 1);
    assert_eq!(
        b.reference_transition(),
        Some(ExecutionReferenceTransition::ReleaseLive)
    );
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
    let original = batch(listing)
        .with_outbox(vec![control_msg(Scope::new("w", "o"))])
        .with_journal(vec![journal_entry(1)])
        .with_reference_transition(ExecutionReferenceTransition::ReleaseLive);

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
    let original = batch(ExecutionListing::CREATED).with_journal(vec![journal_entry(1)]);
    let appended = original.with_appended_journal([journal_entry(2)]);
    let order: Vec<_> = appended
        .journal()
        .iter()
        .map(|entry| entry.payload["n"].clone())
        .collect();
    assert_eq!(order, vec![serde_json::json!(1), serde_json::json!(2)]);
    assert_eq!(appended.listing(), original.listing());
}

#[test]
fn outcome_variants_exist() {
    let _ = TransitionOutcome::Applied { new_version: 4 };
    let _ = TransitionOutcome::VersionConflict { actual: 9 };
    let _ = TransitionOutcome::FencedOut;
}
