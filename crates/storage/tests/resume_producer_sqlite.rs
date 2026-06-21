//! SQLite per-backend tests for `SqliteResumeProducer` (W-S3d, Option B1).
//!
//! These exercise the production SQL path via a real in-memory SQLite pool and
//! prove the atomic consume+enqueue seam at the storage level:
//!  1. `peek` returns a committed token WITHOUT burning it.
//!  2. `consume_and_enqueue_resume` burns the token AND inserts the Resume into
//!     `port_control_queue` in one transaction.
//!  3. A replay returns `Ok(false)` and inserts no second Resume.
//!  4. **Atomicity gate**: when the control INSERT fails inside the tx, the token
//!     DELETE is rolled back — the token survives and no Resume is written.
//!
//! Gated on the `sqlite` feature — skips automatically without it.

#![cfg(feature = "sqlite")]

use std::str::FromStr;
use std::time::Duration;

use nebula_storage::sqlite::{SqliteExecutionStore, SqliteResumeProducer, init_schema};
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, ResumeTarget};
use nebula_storage_port::store::{ExecutionStore, ResumeProducer};
use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Create an isolated, schema-initialised in-memory SQLite pool (shared cache so
/// the pool's connections see the same tables).
async fn fresh_pool() -> sqlx::SqlitePool {
    let db_name = format!("nebula-rp-test-{}", uuid::Uuid::new_v4());
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

fn test_scope() -> Scope {
    Scope::new("ws-sqlite-rp", "org-sqlite-rp")
}

fn fill_hash(byte: u8) -> TokenHash {
    TokenHash::try_from_bytes(vec![byte; 32])
        .expect("32 identical bytes must be a valid 32-byte hash")
}

fn token_row_for(hash: TokenHash, execution_id: &str, callback_label: &str) -> ResumeTokenRow {
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

/// Mint `token_row` into the SQLite execution store via the production `commit`.
async fn seed_token(
    exec_store: &SqliteExecutionStore,
    scope: &Scope,
    execution_id: &str,
    token_row: ResumeTokenRow,
) {
    exec_store
        .create(
            scope,
            execution_id,
            "wf-sqlite-1",
            serde_json::json!({"s": "created"}),
        )
        .await
        .expect("create must succeed on a fresh row");
    let fencing = exec_store
        .acquire_lease(scope, execution_id, "test-runner", Duration::from_secs(30))
        .await
        .expect("acquire_lease must not error")
        .expect("fresh row must yield a fencing token");
    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id)
        .expected_version(0)
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
        "batch must apply; got {outcome:?}"
    );
    exec_store
        .release_lease(scope, execution_id, fencing)
        .await
        .expect("release_lease must succeed");
}

async fn resume_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query("SELECT COUNT(*) AS n FROM port_control_queue WHERE command = 'Resume'")
        .fetch_one(pool)
        .await
        .expect("count query must succeed")
        .try_get::<i64, _>("n")
        .expect("count column must decode")
}

/// Test 1 — `peek` returns the committed token without burning it.
#[tokio::test]
async fn sqlite_peek_returns_row_without_burning() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let producer = SqliteResumeProducer::new(pool.clone());
    let scope = test_scope();
    let hash = fill_hash(0xA1);
    seed_token(
        &exec_store,
        &scope,
        "exe-peek",
        token_row_for(hash.clone(), "exe-peek", "cb-peek"),
    )
    .await;

    let peeked = producer.peek(&hash).await.expect("peek must not error");
    assert_eq!(
        peeked.expect("peek must return the row").callback_label,
        "cb-peek"
    );
    // Survived the peek — a winning consume still follows.
    let won = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-peek", "cb-peek"))
        .await
        .expect("consume must not error");
    assert!(
        won,
        "peek must not burn the token — the consume after peek wins"
    );
}

/// Test 2 — `consume_and_enqueue_resume` burns the token AND writes the Resume.
#[tokio::test]
async fn sqlite_consume_and_enqueue_writes_one_resume() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let producer = SqliteResumeProducer::new(pool.clone());
    let scope = test_scope();
    let hash = fill_hash(0xB2);
    seed_token(
        &exec_store,
        &scope,
        "exe-atomic",
        token_row_for(hash.clone(), "exe-atomic", "cb-atomic"),
    )
    .await;

    let won = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-atomic", "cb-atomic"))
        .await
        .expect("consume must not error");
    assert!(won, "the only consumer wins");
    assert_eq!(
        resume_count(&pool).await,
        1,
        "exactly one Resume must be written"
    );
    assert!(
        producer
            .peek(&hash)
            .await
            .expect("peek must not error")
            .is_none(),
        "token must be burned after a winning consume"
    );
}

/// Test 3 — A replay returns `Ok(false)` and writes no second Resume.
#[tokio::test]
async fn sqlite_replay_returns_false_and_writes_nothing() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let producer = SqliteResumeProducer::new(pool.clone());
    let scope = test_scope();
    let hash = fill_hash(0xC3);
    seed_token(
        &exec_store,
        &scope,
        "exe-replay",
        token_row_for(hash.clone(), "exe-replay", "cb-replay"),
    )
    .await;

    assert!(
        producer
            .consume_and_enqueue_resume(&hash, &resume_msg("exe-replay", "cb-replay"))
            .await
            .expect("first consume must not error")
    );
    assert!(
        !producer
            .consume_and_enqueue_resume(&hash, &resume_msg("exe-replay", "cb-replay"))
            .await
            .expect("replay must not error"),
        "replay of a burned token must return Ok(false)"
    );
    assert_eq!(
        resume_count(&pool).await,
        1,
        "replay must not write a second Resume"
    );
}

/// Test 4 — **Atomicity gate (real-tx rollback).** When the control INSERT fails
/// inside the producer's transaction, the token DELETE rolls back with it: the
/// token survives and no Resume is written.
///
/// We force the INSERT to fail by dropping `port_control_queue` before the call.
/// Falsifiability: a non-atomic `commit`-the-delete-then-insert producer would
/// burn the token and return `Err` with the token gone → `peek` returns `None`
/// → the `is_some()` assertion fails → this is exactly the P1 bug.
#[tokio::test]
async fn sqlite_failed_enqueue_rolls_back_the_burn() {
    let pool = fresh_pool().await;
    let exec_store = SqliteExecutionStore::new(pool.clone());
    let producer = SqliteResumeProducer::new(pool.clone());
    let scope = test_scope();
    let hash = fill_hash(0xD4);
    seed_token(
        &exec_store,
        &scope,
        "exe-rollback",
        token_row_for(hash.clone(), "exe-rollback", "cb-rollback"),
    )
    .await;

    // Make the in-tx control INSERT fail.
    sqlx::query("DROP TABLE port_control_queue")
        .execute(&pool)
        .await
        .expect("drop must succeed");

    let result = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-rollback", "cb-rollback"))
        .await;
    assert!(
        result.is_err(),
        "a failed control INSERT must surface as Err (tx rolled back)"
    );

    // The token survived the rolled-back transaction.
    assert!(
        producer
            .peek(&hash)
            .await
            .expect("peek must not error")
            .is_some(),
        "the token MUST survive a rolled-back enqueue — burning it here is the P1 bug"
    );

    // Restore the table and prove the retry now succeeds + writes exactly one Resume.
    init_schema(&pool)
        .await
        .expect("re-init must restore the table");
    let won = producer
        .consume_and_enqueue_resume(&hash, &resume_msg("exe-rollback", "cb-rollback"))
        .await
        .expect("retry after fault clears must not error");
    assert!(won, "the retry must win (token was still live)");
    assert_eq!(
        resume_count(&pool).await,
        1,
        "the retry writes exactly one Resume"
    );
}
