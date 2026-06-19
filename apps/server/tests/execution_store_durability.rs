//! Restart-survival test for the SQLite execution-store backend (M7 §5).
//!
//! Two-phase SQLite FILE proof:
//!
//! - Phase 1: build a pool against a *persisted* temp file, apply `init_schema`,
//!   enqueue a control command via `SqliteControlQueue`, then `pool.close().await`
//!   (not a bare drop — WAL must checkpoint before the file is re-opened).
//! - Phase 2: re-open the same file with a fresh pool, apply `init_schema`
//!   (idempotent — `CREATE TABLE IF NOT EXISTS`), then `claim_pending` and assert
//!   the command enqueued in Phase 1 is still present.
//!
//! The red-side companion (`in_memory_control_queue_does_not_survive_recreation`)
//! proves that an `InMemoryControlQueue` returns empty after recreation, which
//! makes the SQLite assertion load-bearing: if both backends "survived" the
//! test above would be a tautology.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::inmem::{InMemoryControlQueue, InMemoryExecutionStore};
use nebula_storage::sqlite::{SqliteControlQueue, init_schema};
use nebula_storage_port::Scope;
use nebula_storage_port::dto::{ControlCommand, ControlMsg};
use nebula_storage_port::store::ControlQueue;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tempfile::NamedTempFile;

/// A fixed 16-byte ULID used as the control-message primary key in the test.
/// Chosen as all-ones so it is obviously synthetic and non-colliding.
const TEST_MSG_ID: [u8; 16] = [0xAB; 16];

/// Deterministic test scope — avoids string-formatting overhead in the hot path.
fn test_scope() -> Scope {
    Scope::new("test-workspace", "test-org")
}

/// Build a WAL-mode SQLite pool for `db_path`.
///
/// `create_if_missing(true)` is safe here: a test that gets a fresh temp-file
/// path always needs the file created; a test that reopens an existing file has
/// the file already present.
async fn open_sqlite_pool_for_test(db_path: &str) -> sqlx::SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("SQLite pool must open for the durability test")
}

/// Phase 1: open `db_path`, initialise schema, enqueue a Cancel command, flush WAL.
///
/// Returns the `execution_id` string that was used so Phase 2 can query the same row.
async fn enqueue_and_close(db_path: &str) -> String {
    let pool = open_sqlite_pool_for_test(db_path).await;
    init_schema(&pool)
        .await
        .expect("init_schema must succeed in Phase 1");

    let control_queue = SqliteControlQueue::new(pool.clone());
    let execution_id = "test-execution-durability-proof".to_string();

    let msg = ControlMsg {
        id: TEST_MSG_ID,
        execution_id: execution_id.clone(),
        command: ControlCommand::Cancel,
        scope: test_scope(),
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    control_queue
        .enqueue(&msg)
        .await
        .expect("enqueue must succeed in Phase 1");

    // Explicit pool close: flushes the WAL checkpoint to the main db file so
    // a second pool opened against the same path sees all written pages.
    // Do NOT rely on Drop here — `Drop` is best-effort and may not checkpoint
    // before the test OS process reuses the file descriptor.
    pool.close().await;

    execution_id
}

/// Phase 2: reopen `db_path`, run `claim_pending`, assert the Cancel command survives.
async fn claim_and_assert_recovered(db_path: &str, expected_execution_id: &str) {
    let pool = open_sqlite_pool_for_test(db_path).await;
    // init_schema is idempotent (CREATE TABLE IF NOT EXISTS) — safe to call again.
    init_schema(&pool)
        .await
        .expect("init_schema must succeed in Phase 2");

    let control_queue = SqliteControlQueue::new(pool.clone());

    // Use a zero processor id (the conformance test convention).
    let processor_id = [0u8; 16];
    let recovered_commands = control_queue
        .claim_pending(&processor_id, 10)
        .await
        .expect("claim_pending must succeed in Phase 2");

    let survived = recovered_commands.iter().any(|msg| {
        msg.execution_id == expected_execution_id && matches!(msg.command, ControlCommand::Cancel)
    });
    assert!(
        survived,
        "the Cancel command for execution '{expected_execution_id}' \
         enqueued in Phase 1 must survive WAL close + pool reopen; \
         recovered commands: {recovered_commands:?}"
    );

    pool.close().await;
}

/// SQLite control queue persists across pool close + reopen (WAL-flush proof).
///
/// This is the durability acceptance test for M7: proves that `SqliteControlQueue`
/// survives a simulated server restart (close the pool, open a new one against
/// the same file) while `InMemoryControlQueue` does not (see companion test below).
#[tokio::test]
async fn sqlite_control_queue_survives_restart() {
    // `keep()` prevents NamedTempFile from deleting the file on drop so Phase 2
    // can re-open it. We clean up manually at the end.
    let tmp = NamedTempFile::new().expect("temp file must be created for durability test");
    let (persisted_file, db_path) = tmp.keep().expect("must persist temp file for Phase 2");
    let db_path_str = db_path
        .to_str()
        .expect("temp file path must be valid UTF-8")
        .to_string();

    // Phase 1 — write.
    let written_execution_id = enqueue_and_close(&db_path_str).await;

    // Phase 2 — recover.
    claim_and_assert_recovered(&db_path_str, &written_execution_id).await;

    // Cleanup — ignore errors (the OS will reclaim on process exit anyway).
    drop(persisted_file);
    let _ = std::fs::remove_file(&db_path);
}

/// Red-side proof: in-memory control queue does NOT survive recreation.
///
/// If this test ever fails it means `InMemoryControlQueue` somehow shared
/// state across instances (a bug) — and the SQLite durability test above
/// would lose its meaning. This test must stay green for the suite to be
/// non-vacuous.
#[tokio::test]
async fn in_memory_control_queue_does_not_survive_recreation() {
    // Build the first pair of stores (same shared-core pattern as AppState::in_memory).
    let exec_store_1 = InMemoryExecutionStore::new();
    let queue_1 = Arc::new(InMemoryControlQueue::new(&exec_store_1));

    let execution_id = "test-inmem-non-durability".to_string();
    let msg = ControlMsg {
        id: TEST_MSG_ID,
        execution_id: execution_id.clone(),
        command: ControlCommand::Cancel,
        scope: test_scope(),
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    queue_1
        .enqueue(&msg)
        .await
        .expect("enqueue must succeed on in-memory queue");

    // Simulate restart: drop all references to the first instance and build fresh.
    drop(queue_1);
    drop(exec_store_1);

    let exec_store_2 = InMemoryExecutionStore::new();
    let queue_2 = InMemoryControlQueue::new(&exec_store_2);

    let processor_id = [0u8; 16];
    let recovered = queue_2
        .claim_pending(&processor_id, 10)
        .await
        .expect("claim_pending must succeed on fresh in-memory queue");

    assert!(
        recovered.is_empty(),
        "in-memory queue must NOT recover state across recreation — \
         this is the red-side proof that the SQLite test is non-vacuous; \
         unexpectedly found: {recovered:?}"
    );
}
