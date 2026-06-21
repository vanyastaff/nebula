//! Behavioral tests for `InMemoryResumeProducer` (W-S3d, Option B1).
//!
//! Covers the atomic consume+enqueue seam at the storage level:
//!  1. `peek` returns the row WITHOUT burning it (a later consume still wins).
//!  2. `peek` returns `None` for an absent hash.
//!  3. `consume_and_enqueue_resume` burns the token AND enqueues the Resume
//!     under one lock (the control queue observes exactly one Pending row).
//!  4. A second `consume_and_enqueue_resume` on the same hash returns
//!     `Ok(false)` and enqueues nothing (single-use replay gate).
//!  5. The producer, token store, and control queue all share one `SharedState`
//!     (a token minted via `commit` is consumable and lands in the same queue).
//!
//! Tokens are minted through `ExecutionStore::commit` with a `TransitionBatch`
//! (the production path); the producer is obtained via `resume_producer()` so
//! the shared-mutex invariant is exercised, not bypassed.

use std::time::Duration;

use nebula_storage::inmem::InMemoryControlQueue;
use nebula_storage::{InMemoryExecutionStore, InMemoryResumeProducer};
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, ResumeTarget};
use nebula_storage_port::store::{ExecutionStore, ResumeProducer};
use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};

fn test_scope() -> Scope {
    Scope::new("test-org", "test-ws")
}

fn fill_hash(byte: u8) -> TokenHash {
    TokenHash::try_from_bytes(vec![byte; 32])
        .expect("32 identical bytes must be a valid 32-byte hash")
}

fn webhook_token_row(hash: TokenHash, execution_id: &str, callback_label: &str) -> ResumeTokenRow {
    ResumeTokenRow::new(
        hash,
        test_scope(),
        execution_id.to_owned(),
        "node-a".to_owned(),
        ResumeTokenWaitKind::Webhook,
        callback_label.to_owned(),
        "2026-06-21T00:00:00Z".to_owned(),
        None,
    )
}

/// A `Resume` `ControlMsg` for `execution_id`, targeting `callback_label`.
fn resume_msg(execution_id: &str, callback_label: &str) -> ControlMsg {
    ControlMsg {
        id: *uuid::Uuid::new_v4().as_bytes(),
        execution_id: execution_id.to_owned(),
        command: ControlCommand::Resume,
        scope: test_scope(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: Some(ResumeTarget::Webhook {
            callback_id: callback_label.to_owned(),
        }),
    }
}

/// Mint `token_row` into `exec_store` via the production `commit` path.
async fn seed_token(
    exec_store: &InMemoryExecutionStore,
    scope: &Scope,
    execution_id: &str,
    token_row: ResumeTokenRow,
) {
    exec_store
        .create(
            scope,
            execution_id,
            "wf-1",
            serde_json::json!({"s": "created"}),
        )
        .await
        .expect("execution row must not already exist");

    let fencing_token = exec_store
        .acquire_lease(scope, execution_id, "test-runner", Duration::from_secs(30))
        .await
        .expect("acquire_lease must not error")
        .expect("fresh row must yield a fencing token");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id)
        .expected_version(0)
        .fencing(fencing_token)
        .new_state(serde_json::json!({"s": "waiting"}))
        .resume_tokens(vec![token_row])
        .build()
        .expect("well-formed batch must build");

    let outcome = exec_store
        .commit(batch)
        .await
        .expect("commit must not error");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "batch must apply; got {outcome:?}"
    );
    exec_store
        .release_lease(scope, execution_id, fencing_token)
        .await
        .expect("release_lease must succeed");
}

/// Test 1 — `peek` returns the row but does NOT burn it.
#[tokio::test]
async fn peek_returns_row_without_burning() {
    let exec_store = InMemoryExecutionStore::new();
    let producer = exec_store.resume_producer();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let hash = fill_hash(0xA1);
    seed_token(
        &exec_store,
        &scope,
        "exe-peek",
        webhook_token_row(hash.clone(), "exe-peek", "cb-peek"),
    )
    .await;

    let peeked = producer.peek(&hash).await.expect("peek must not error");
    let row = peeked.expect("peek must return the seeded row");
    assert_eq!(row.execution_id, "exe-peek");
    assert_eq!(row.callback_label, "cb-peek");

    // The token survived the peek — a direct consume still finds it.
    use nebula_storage_port::store::ResumeTokenStore;
    let still_there = token_store
        .consume(&hash)
        .await
        .expect("consume must not error");
    assert!(
        still_there.is_some(),
        "peek must NOT burn the token — a consume after peek must still win"
    );
}

/// Test 2 — `peek` returns `None` for a hash that was never inserted.
#[tokio::test]
async fn peek_returns_none_for_absent_hash() {
    let exec_store = InMemoryExecutionStore::new();
    let producer = exec_store.resume_producer();
    let absent = producer
        .peek(&fill_hash(0xB2))
        .await
        .expect("peek must not error");
    assert!(absent.is_none(), "peek of an absent hash must return None");
}

/// Test 3 — `consume_and_enqueue_resume` burns the token AND enqueues the
/// Resume under one lock: the shared control queue sees exactly one Pending row.
#[tokio::test]
async fn consume_and_enqueue_is_atomic_under_one_lock() {
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let producer = exec_store.resume_producer();
    let scope = test_scope();
    let hash = fill_hash(0xC3);
    seed_token(
        &exec_store,
        &scope,
        "exe-atomic",
        webhook_token_row(hash.clone(), "exe-atomic", "cb-atomic"),
    )
    .await;

    let msg = resume_msg("exe-atomic", "cb-atomic");
    let won = producer
        .consume_and_enqueue_resume(&hash, &msg)
        .await
        .expect("consume_and_enqueue must not error");
    assert!(won, "the only consumer must win the atomic delete");

    let queued = control_queue.snapshot();
    assert_eq!(queued.len(), 1, "exactly one Resume must be enqueued");
    let (enqueued, status) = &queued[0];
    assert_eq!(enqueued.command, ControlCommand::Resume);
    assert_eq!(enqueued.execution_id, "exe-atomic");
    assert_eq!(
        enqueued.resume_target,
        Some(ResumeTarget::Webhook {
            callback_id: "cb-atomic".to_owned()
        })
    );
    assert_eq!(status, "Pending");

    // The token is burned — peek now returns None.
    assert!(
        producer
            .peek(&hash)
            .await
            .expect("peek must not error")
            .is_none(),
        "the token must be burned after a winning consume"
    );
}

/// Test 4 — A second `consume_and_enqueue_resume` on the same hash returns
/// `Ok(false)` and enqueues nothing (single-use replay gate).
#[tokio::test]
async fn second_consume_returns_false_and_enqueues_nothing() {
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let producer = exec_store.resume_producer();
    let scope = test_scope();
    let hash = fill_hash(0xD4);
    seed_token(
        &exec_store,
        &scope,
        "exe-replay",
        webhook_token_row(hash.clone(), "exe-replay", "cb-replay"),
    )
    .await;

    let first = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-replay", "cb-replay"))
        .await
        .expect("first consume must not error");
    assert!(first, "first consume wins");

    let second = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-replay", "cb-replay"))
        .await
        .expect("second consume must not error");
    assert!(
        !second,
        "second consume of the same hash must return Ok(false)"
    );

    assert_eq!(
        control_queue.snapshot().len(),
        1,
        "the replay must NOT enqueue a second Resume"
    );
}

/// Test 5 — `standalone()`-style independence: a producer over a fresh exec
/// store sees no tokens minted on a different store.
#[tokio::test]
async fn producer_over_separate_state_sees_no_foreign_tokens() {
    let store_a = InMemoryExecutionStore::new();
    let store_b = InMemoryExecutionStore::new();
    let producer_b: InMemoryResumeProducer = store_b.resume_producer();
    let scope = test_scope();
    seed_token(
        &store_a,
        &scope,
        "exe-iso",
        webhook_token_row(fill_hash(0xE5), "exe-iso", "cb-iso"),
    )
    .await;

    // Producer B is backed by store B's state, which never received the token.
    assert!(
        producer_b
            .peek(&fill_hash(0xE5))
            .await
            .expect("peek must not error")
            .is_none(),
        "a producer must not see tokens minted on a different execution store"
    );
}

/// Test 6 — a non-`Resume` command is rejected at the boundary BEFORE any
/// mutation: the call errors, the token is NOT burned, and nothing is enqueued.
///
/// This is the release-enforced structural guard (replacing the prior debug-only
/// `debug_assert!` that ran *after* the mutation): the resume producer must only
/// ever enqueue `Resume`, and a misuse must never burn a token.
#[tokio::test]
async fn non_resume_command_rejected_without_burning_token() {
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let producer = exec_store.resume_producer();
    let scope = test_scope();
    let hash = fill_hash(0xF6);
    seed_token(
        &exec_store,
        &scope,
        "exe-guard",
        webhook_token_row(hash.clone(), "exe-guard", "cb-guard"),
    )
    .await;

    // A `ControlMsg` carrying a non-`Resume` command.
    let mut wrong = resume_msg("exe-guard", "cb-guard");
    wrong.command = ControlCommand::Cancel;

    let result = producer.consume_and_enqueue_resume(&hash, &wrong).await;
    assert!(
        result.is_err(),
        "a non-Resume command must be rejected, not enqueued; got {result:?}"
    );

    // Boundary check fires BEFORE mutation: the token must survive.
    assert!(
        producer
            .peek(&hash)
            .await
            .expect("peek must not error")
            .is_some(),
        "the token must NOT be burned when the command is rejected"
    );
    assert!(
        control_queue.snapshot().is_empty(),
        "nothing may be enqueued when the command is rejected"
    );
}
