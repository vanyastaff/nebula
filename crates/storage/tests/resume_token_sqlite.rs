//! SQLite per-backend tests for `SqliteResumeTokenStore` (W-S3c).
//!
//! These are RED-first tests mandated by ADR-0099 W-S3c to prevent a
//! "dropped field = silent data loss" regression (the W-S3a lesson).
//! They exercise the production SQL path via a real in-memory SQLite pool.
//!
//! Gated on the `sqlite` feature — skips automatically without it.
//!
//! ## Test list
//! 1. `sqlite_batch_commit_inserts_token_visible_to_consume` — round-trip:
//!    `ExecutionStore::commit` with a `ResumeTokenRow` → `consume` returns row.
//!
//! 2. `sqlite_duplicate_park_is_idempotent_via_on_conflict_do_nothing` —
//!    re-committing a batch with the same `(execution_id, node_key)` must NOT
//!    produce a second live token (idempotent re-park / crash re-drive safety).

#![cfg(feature = "sqlite")]

use std::str::FromStr;
use std::time::Duration;

use nebula_storage::sqlite::{SqliteExecutionStore, SqliteResumeTokenStore, init_schema};
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
use nebula_storage_port::store::{ExecutionStore, ResumeTokenStore};
use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

// ── Pool helper ───────────────────────────────────────────────────────────────

/// Create an isolated, schema-initialised in-memory SQLite pool.
///
/// Uses a random named DB with `?mode=memory&cache=shared` so multiple
/// connections within the pool share the same tables — required for the
/// `RETURNING`-based `consume` query to see rows written by `commit`.
async fn fresh_pool() -> sqlx::SqlitePool {
    let db_name = format!("nebula-rt-test-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
    let opts = SqliteConnectOptions::from_str(&url)
        .expect("parse sqlite memory url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite memory");

    init_schema(&pool)
        .await
        .expect("install port schema including port_resume_tokens");

    pool
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn test_scope() -> Scope {
    Scope::new("ws-sqlite-rt", "org-sqlite-rt")
}

fn fill_hash(byte: u8) -> TokenHash {
    TokenHash::try_from_bytes(vec![byte; 32])
        .expect("32 identical bytes must be a valid 32-byte hash")
}

fn token_row_for(hash: TokenHash, execution_id: &str, node_key: &str) -> ResumeTokenRow {
    ResumeTokenRow::new(
        hash,
        test_scope(),
        execution_id.to_owned(),
        node_key.to_owned(),
        ResumeTokenWaitKind::Webhook,
        "sqlite-cb-label".to_owned(),
        "2026-06-21T00:00:00Z".to_owned(),
        None,
    )
}

/// Create an execution row and commit a batch carrying `token_row`.
///
/// Returns the new CAS version (always 1 for the first commit).
async fn seed_token(
    exec_store: &SqliteExecutionStore,
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
                "wf-sqlite-1",
                serde_json::json!({"s": "created"}),
            )
            .await
            .expect("create must succeed on a fresh row");
    }

    let fencing = exec_store
        .acquire_lease(scope, execution_id, "test-runner", Duration::from_secs(30))
        .await
        .expect("acquire_lease must not error")
        .expect("fresh row must yield a fencing token");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id)
        .expected_version(expected_version)
        .fencing(fencing)
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
        "batch at version {expected_version} must apply; got {outcome:?}"
    );

    // Release the lease so subsequent `acquire_lease` calls on the same row
    // succeed (e.g. the duplicate-park test seeds two batches on one execution).
    // `FencingToken: Copy` so `fencing` is still valid after the batch builder.
    exec_store
        .release_lease(scope, execution_id, fencing)
        .await
        .expect("release_lease must succeed after a successful commit");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1 — SQLite per-backend RED test: `commit` inserts a token that
/// `consume` can retrieve.
///
/// Falsifiability: remove the `port_resume_tokens` INSERT loop from
/// `SqliteExecutionStore::commit` → `consume` returns `None` → RED.
///
/// This is the W-S3a-lesson guard for the SQLite backend: a dropped INSERT
/// loop would be silently invisible if only the InMemory path were tested.
#[tokio::test]
async fn sqlite_batch_commit_inserts_token_visible_to_consume() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let token_store = SqliteResumeTokenStore::new(pool);
    let scope = test_scope();
    let hash = fill_hash(0xAA);
    let row = token_row_for(hash.clone(), "exe-sqlite-1", "node-a");

    seed_token(&exec_store, &scope, "exe-sqlite-1", 0, row).await;

    let consumed = token_store
        .consume(&hash)
        .await
        .expect("consume on an inserted hash must not error");

    let returned_row = consumed.expect("consume must return the row inserted by commit");
    assert_eq!(returned_row.execution_id, "exe-sqlite-1");
    assert_eq!(returned_row.node_key, "node-a");
    assert_eq!(returned_row.wait_kind, ResumeTokenWaitKind::Webhook);
    assert_eq!(returned_row.callback_label, "sqlite-cb-label");
    assert_eq!(returned_row.scope, scope);
}

/// Test 2 — `ON CONFLICT(execution_id, node_key) DO NOTHING` prevents a
/// duplicate live token when the same `(execution_id, node_key)` pair is
/// re-submitted (crash re-drive / idempotent re-park).
///
/// The second `commit` must not error and must not create a second token row;
/// the stored token count for the pair must remain 1.
///
/// Falsifiability: change `ON CONFLICT … DO NOTHING` to `OR REPLACE` → the
/// second commit replaces the token → the test still passes (the count
/// assertion would need a different probe), but a hash-change would mean the
/// first `consume` returns the wrong row.  More critically, if the conflict
/// clause is removed entirely, the second commit errors on a UNIQUE violation
/// → `commit` returns `Err` → the `assert!` on `Applied` fires → RED.
#[tokio::test]
async fn sqlite_duplicate_park_is_idempotent_via_on_conflict_do_nothing() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let token_store = SqliteResumeTokenStore::new(pool.clone());
    let scope = test_scope();
    let hash = fill_hash(0xBB);
    let row_first = token_row_for(hash.clone(), "exe-sqlite-2", "node-b");

    // First park: version 0 → 1, token inserted.
    seed_token(&exec_store, &scope, "exe-sqlite-2", 0, row_first).await;

    // Second park of the same (execution_id, node_key) at version 1 → 2.
    // The INSERT must silently do nothing (no error, no second row).
    let hash_second = fill_hash(0xCC); // different hash to prove no row was replaced
    let row_second = ResumeTokenRow::new(
        hash_second.clone(),
        scope.clone(),
        "exe-sqlite-2".to_owned(),
        "node-b".to_owned(), // same node_key as the first park
        ResumeTokenWaitKind::Webhook,
        "cb-second".to_owned(),
        "2026-06-21T00:00:02Z".to_owned(),
        None,
    );
    seed_token(&exec_store, &scope, "exe-sqlite-2", 1, row_second).await;

    // The original token must still be consumable (the second INSERT did nothing).
    let first_token = token_store
        .consume(&hash)
        .await
        .expect("consume of first hash must not error");
    assert!(
        first_token.is_some(),
        "original token must be intact after a duplicate-park commit"
    );

    // The second hash must NOT be present (the DO NOTHING clause kept it out).
    let second_token = token_store
        .consume(&hash_second)
        .await
        .expect("consume of second hash must not error");
    assert!(
        second_token.is_none(),
        "DO NOTHING conflict resolution must prevent the second hash from being inserted"
    );
}
