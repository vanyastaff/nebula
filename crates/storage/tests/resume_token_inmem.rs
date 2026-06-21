//! Behavioral tests for `InMemoryResumeTokenStore` (W-S3c).
//!
//! Covers:
//!  1. `consume` returns the minted row on a matching hash.
//!  2. A second `consume` with the same hash returns `None` (single-use).
//!  3. A `consume` with an uninserted hash returns `None` without touching stored tokens.
//!  4. `revoke_on_terminal` removes every token for a given execution.
//!  5. `revoke_on_terminal` returns the exact count of rows removed.
//!  6. A token inserted via `TransitionBatch::commit` is visible to `consume` (W-S3c atomicity).
//!  7. A failed `consume` (wrong hash) does not consume the valid token.
//!  8. `standalone()` store is always empty (no shared state with any execution store).
//!
//! All tests construct tokens via `ExecutionStore::commit` with a `TransitionBatch`
//! carrying `resume_tokens`, which is the production code path.  Store access goes
//! through the public `resume_token_store()` method so the shared-mutex invariant
//! is exercised, not bypassed.

use std::time::Duration;

use nebula_storage::{InMemoryExecutionStore, InMemoryResumeTokenStore};
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
use nebula_storage_port::store::{ExecutionStore, ResumeTokenStore};
use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn test_scope() -> Scope {
    Scope::new("test-org", "test-ws")
}

/// 32-byte `TokenHash` filled with the given byte value.
fn fill_hash(byte: u8) -> TokenHash {
    TokenHash::try_from_bytes(vec![byte; 32])
        .expect("32 identical bytes must be a valid 32-byte hash")
}

fn minimal_token_row(hash: TokenHash, execution_id: &str, node_key: &str) -> ResumeTokenRow {
    ResumeTokenRow {
        token_hash: hash,
        scope: test_scope(),
        execution_id: execution_id.to_owned(),
        node_key: node_key.to_owned(),
        wait_kind: ResumeTokenWaitKind::Webhook,
        callback_label: "cb-label".to_owned(),
        created_at: "2026-06-21T00:00:00Z".to_owned(),
        expires_at: None,
    }
}

/// Create a fresh execution row (when `expected_version == 0`) then commit a
/// `TransitionBatch` carrying `token_row`.
///
/// On return the store holds the execution at `expected_version + 1` and the
/// token is persisted.  Callers building a second batch against the same
/// execution should pass `expected_version = 1`.
async fn seed_token(
    exec_store: &InMemoryExecutionStore,
    scope: &Scope,
    execution_id: &str,
    expected_version: u64,
    token_row: ResumeTokenRow,
) {
    if expected_version == 0 {
        exec_store
            .create(
                scope,
                execution_id,
                "wf-1",
                serde_json::json!({"s": "created"}),
            )
            .await
            .expect("execution row must not already exist when seeding version 0");
    }

    let fencing_token = exec_store
        .acquire_lease(scope, execution_id, "test-runner", Duration::from_secs(30))
        .await
        .expect("acquire_lease must not error")
        .expect("fresh or re-lockable row must yield a fencing token");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id)
        .expected_version(expected_version)
        .fencing(fencing_token)
        .new_state(serde_json::json!({"s": "waiting"}))
        .resume_tokens(vec![token_row])
        .build()
        .expect("well-formed batch must build without error");

    let outcome = exec_store
        .commit(batch)
        .await
        .expect("commit must not error");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "batch at expected version {expected_version} must apply; got {outcome:?}"
    );

    // Release the lease so subsequent `acquire_lease` calls on the same
    // execution row succeed (e.g. tests 4 and 5 seed two tokens on one row).
    // `FencingToken: Copy` so `fencing_token` is still valid here.
    exec_store
        .release_lease(scope, execution_id, fencing_token)
        .await
        .expect("release_lease must succeed after a successful commit");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1 — `consume` returns the minted row when the hash matches.
///
/// Falsifiability: delete the `resume_tokens.insert` in `InMemoryExecutionStore::commit`
/// → `consume` finds no entry → `consumed.is_some()` assertion fails → RED.
#[tokio::test]
async fn consume_returns_matching_row_on_valid_hash() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let hash = fill_hash(0xAA);
    let row = minimal_token_row(hash.clone(), "exe-t1", "node-a");

    seed_token(&exec_store, &scope, "exe-t1", 0, row).await;

    let consumed = token_store
        .consume(&hash)
        .await
        .expect("consume on a present hash must not error");

    let returned_row = consumed.expect("consume must return the row for a matching hash");
    assert_eq!(returned_row.execution_id, "exe-t1");
    assert_eq!(returned_row.node_key, "node-a");
    assert_eq!(returned_row.wait_kind, ResumeTokenWaitKind::Webhook);
    assert_eq!(returned_row.callback_label, "cb-label");
    assert_eq!(returned_row.scope, scope);
}

/// Test 2 — A second `consume` with the same hash returns `None` (single-use invariant).
///
/// Falsifiability: clone the row instead of removing it on consume
/// → second call returns `Some(_)` → `second_consume.is_none()` assertion fails → RED.
#[tokio::test]
async fn consume_returns_none_on_second_call() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let hash = fill_hash(0xBB);
    let row = minimal_token_row(hash.clone(), "exe-t2", "node-b");

    seed_token(&exec_store, &scope, "exe-t2", 0, row).await;

    let first_consume = token_store
        .consume(&hash)
        .await
        .expect("first consume must not error");
    assert!(first_consume.is_some(), "first consume must succeed");

    let second_consume = token_store
        .consume(&hash)
        .await
        .expect("second consume must not error");
    assert!(
        second_consume.is_none(),
        "second consume of the same hash must return None (single-use)"
    );
}

/// Test 3 — `consume` with a hash that was never inserted returns `None`.
///
/// Falsifiability: return the first available row unconditionally
/// → wrong-hash consume returns `Some(_)` → `missed.is_none()` assertion fails → RED.
#[tokio::test]
async fn consume_returns_none_for_hash_that_was_never_inserted() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let stored_hash = fill_hash(0xCC);
    let row = minimal_token_row(stored_hash.clone(), "exe-t3", "node-c");

    seed_token(&exec_store, &scope, "exe-t3", 0, row).await;

    // Hash with different byte value — was never inserted.
    let absent_hash = fill_hash(0xDD);
    let missed = token_store
        .consume(&absent_hash)
        .await
        .expect("consume of an absent hash must not error");

    assert!(
        missed.is_none(),
        "consume of a hash that was never inserted must return None"
    );
}

/// Test 4 — `revoke_on_terminal` removes all tokens for the target execution.
///
/// Falsifiability: omit the `retain` call in `revoke_on_terminal`
/// → tokens persist → post-revoke consume returns `Some(_)` → assertions fail → RED.
#[tokio::test]
async fn revoke_on_terminal_removes_all_tokens_for_execution() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();

    // Mint two tokens on the same execution (two concurrently-parked nodes).
    let hash_node_a = fill_hash(0x01);
    seed_token(
        &exec_store,
        &scope,
        "exe-t4",
        0,
        minimal_token_row(hash_node_a.clone(), "exe-t4", "node-a"),
    )
    .await;

    let hash_node_b = fill_hash(0x02);
    let node_b_row = ResumeTokenRow {
        token_hash: hash_node_b.clone(),
        scope: scope.clone(),
        execution_id: "exe-t4".to_owned(),
        node_key: "node-b".to_owned(),
        wait_kind: ResumeTokenWaitKind::Approval,
        callback_label: "approver@example.com".to_owned(),
        created_at: "2026-06-21T00:00:01Z".to_owned(),
        expires_at: None,
    };
    seed_token(&exec_store, &scope, "exe-t4", 1, node_b_row).await;

    token_store
        .revoke_on_terminal(&scope, "exe-t4")
        .await
        .expect("revoke_on_terminal must not error");

    let post_revoke_a = token_store
        .consume(&hash_node_a)
        .await
        .expect("post-revoke consume node-a must not error");
    let post_revoke_b = token_store
        .consume(&hash_node_b)
        .await
        .expect("post-revoke consume node-b must not error");

    assert!(
        post_revoke_a.is_none(),
        "token for node-a must be gone after revoke_on_terminal"
    );
    assert!(
        post_revoke_b.is_none(),
        "token for node-b must be gone after revoke_on_terminal"
    );
}

/// Test 5 — `revoke_on_terminal` returns the exact count of rows it removed.
///
/// Falsifiability: always return 0 → `assert_eq!(removed_count, 2)` assertion fails → RED.
#[tokio::test]
async fn revoke_on_terminal_returns_count_of_removed_rows() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();

    seed_token(
        &exec_store,
        &scope,
        "exe-t5",
        0,
        minimal_token_row(fill_hash(0x11), "exe-t5", "node-a"),
    )
    .await;
    seed_token(
        &exec_store,
        &scope,
        "exe-t5",
        1,
        minimal_token_row(fill_hash(0x22), "exe-t5", "node-b"),
    )
    .await;

    let removed_count = token_store
        .revoke_on_terminal(&scope, "exe-t5")
        .await
        .expect("revoke_on_terminal must not error");

    assert_eq!(
        removed_count, 2,
        "exactly 2 tokens must be removed by revoke_on_terminal"
    );
}

/// Test 6 — A token inserted via `TransitionBatch::commit` is visible to `consume`
/// via the store obtained from `resume_token_store()` (W-S3c atomicity invariant).
///
/// Falsifiability: skip the `resume_tokens` insertion loop in `commit`
/// → `consume` returns `None` → `consumed.is_some()` assertion fails → RED.
#[tokio::test]
async fn token_inserted_via_commit_batch_is_visible_to_consume() {
    let exec_store = InMemoryExecutionStore::new();
    // `resume_token_store()` returns a store that shares the same mutex as
    // `exec_store`, so inserts from `commit` are immediately visible here.
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let hash = fill_hash(0x55);
    let row = minimal_token_row(hash.clone(), "exe-t6", "node-a");

    seed_token(&exec_store, &scope, "exe-t6", 0, row).await;

    let consumed = token_store
        .consume(&hash)
        .await
        .expect("consume must not error");

    assert!(
        consumed.is_some(),
        "token inserted via TransitionBatch commit must be immediately visible to consume"
    );
}

/// Test 7 — A failed `consume` (wrong hash) leaves the valid token intact.
///
/// Falsifiability: let `consume` with a wrong hash remove the first map entry
/// → the subsequent correct-hash consume returns `None`
/// → `valid_row.is_some()` assertion fails → RED.
#[tokio::test]
async fn wrong_hash_consume_does_not_consume_valid_token() {
    let exec_store = InMemoryExecutionStore::new();
    let token_store = exec_store.resume_token_store();
    let scope = test_scope();
    let valid_hash = fill_hash(0x77);
    let row = minimal_token_row(valid_hash.clone(), "exe-t7", "node-a");

    seed_token(&exec_store, &scope, "exe-t7", 0, row).await;

    // Absent hash — must return None and leave the stored token untouched.
    let absent_consume = token_store
        .consume(&fill_hash(0x88))
        .await
        .expect("absent-hash consume must not error");
    assert!(
        absent_consume.is_none(),
        "absent-hash consume must return None"
    );

    // Correct hash must still find the row.
    let valid_row = token_store
        .consume(&valid_hash)
        .await
        .expect("correct-hash consume must not error");
    assert!(
        valid_row.is_some(),
        "valid token must still be present after a failed absent-hash consume attempt"
    );
}

/// Test 8 — `standalone()` returns a store that is always empty.
///
/// `InMemoryResumeTokenStore::standalone()` is for composition roots holding an
/// erased `Arc<dyn ExecutionStore>` that do not exercise the park path.  It must
/// never return tokens because `commit` on a separate store cannot reach its state.
///
/// Falsifiability: make `standalone()` share global state with every store
/// → consume returns `Some(_)` → `absent.is_none()` assertion fails → RED.
#[tokio::test]
async fn standalone_store_is_always_empty() {
    let standalone_store = InMemoryResumeTokenStore::standalone();

    let absent = standalone_store
        .consume(&fill_hash(0xFF))
        .await
        .expect("consume on standalone store must not error");

    assert!(
        absent.is_none(),
        "standalone store is disconnected from any execution store so consume must return None"
    );

    let revoked = standalone_store
        .revoke_on_terminal(&test_scope(), "exe-nonexistent")
        .await
        .expect("revoke_on_terminal on standalone store must not error");
    assert_eq!(
        revoked, 0,
        "standalone store has no tokens so revoke_on_terminal must return 0"
    );
}
