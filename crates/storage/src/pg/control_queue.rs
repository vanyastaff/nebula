//! Postgres implementation of [`ControlQueueRepo`] (ADR-0008 / ADR-0017).
//!
//! Schema: migrations `0013_execution_lifecycle.sql` (base table) +
//! `0021_add_control_queue_reclaim_count.sql` (reclaim column + partial
//! index). No migration changes needed — this module binds the trait
//! against the existing schema.

use async_trait::async_trait;
use sqlx::{Pool, Postgres};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::{ControlCommand, ControlQueueEntry, ControlQueueRepo, ReclaimOutcome},
};

type EntryTuple = (
    Vec<u8>,                               // id
    Vec<u8>,                               // execution_id
    String,                                // command
    Option<Vec<u8>>,                       // issued_by
    chrono::DateTime<chrono::Utc>,         // issued_at
    String,                                // status
    Option<Vec<u8>>,                       // processed_by
    Option<chrono::DateTime<chrono::Utc>>, // processed_at
    Option<String>,                        // error_message
    i64,                                   // reclaim_count
);

const SELECT_COLS: &str = "id, execution_id, command, issued_by, issued_at, status, processed_by, \
     processed_at, error_message, reclaim_count";

fn decode_command(s: &str) -> Result<ControlCommand, StorageError> {
    match s {
        "Start" => Ok(ControlCommand::Start),
        "Cancel" => Ok(ControlCommand::Cancel),
        "Terminate" => Ok(ControlCommand::Terminate),
        "Resume" => Ok(ControlCommand::Resume),
        "Restart" => Ok(ControlCommand::Restart),
        other => Err(StorageError::Serialization(format!(
            "unknown control_queue.command: {other}"
        ))),
    }
}

fn tuple_to_entry(t: EntryTuple) -> Result<ControlQueueEntry, StorageError> {
    Ok(ControlQueueEntry {
        id: t.0,
        execution_id: t.1,
        command: decode_command(&t.2)?,
        issued_by: t.3,
        issued_at: t.4,
        status: t.5,
        processed_by: t.6,
        processed_at: t.7,
        error_message: t.8,
        reclaim_count: u32::try_from(t.9.max(0)).unwrap_or(u32::MAX),
    })
}

/// Postgres-backed durable control queue (canon §12.2).
///
/// Implements the [`ControlQueueRepo`] trait against the
/// `execution_control_queue` table defined by migration 0013 + 0021.
///
/// - `claim_pending` uses `FOR UPDATE SKIP LOCKED` per ADR-0008 §1; two concurrent claimers never
///   double-claim a row.
/// - `reclaim_stuck` runs two `UPDATE ... WHERE status = 'Processing' ... RETURNING id` statements
///   inside one transaction; the `status = 'Processing'` predicate is the CAS that fences
///   concurrent sweepers (ADR-0017). The exhausted-message encodes `processed_by` as lowercase hex
///   to stay byte-identical with `InMemoryControlQueueRepo`.
#[derive(Clone)]
pub struct PgControlQueueRepo {
    pool: Pool<Postgres>,
}

impl PgControlQueueRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ControlQueueRepo for PgControlQueueRepo {
    async fn enqueue(&self, entry: &ControlQueueEntry) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO execution_control_queue \
             (id, execution_id, command, issued_by, issued_at, status, \
              processed_at, processed_by, error_message, reclaim_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&entry.id)
        .bind(&entry.execution_id)
        .bind(entry.command.as_str())
        .bind(entry.issued_by.as_deref())
        .bind(entry.issued_at)
        .bind(&entry.status)
        .bind(entry.processed_at)
        .bind(entry.processed_by.as_deref())
        .bind(entry.error_message.as_deref())
        .bind(i64::from(entry.reclaim_count))
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;
        Ok(())
    }

    async fn claim_pending(
        &self,
        processor: &[u8],
        batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError> {
        // Canonical Postgres SKIP LOCKED claim (ADR-0008 §1).
        // The CTE's SELECT ... FOR UPDATE SKIP LOCKED skips rows another
        // runner has already locked; the outer UPDATE stamps the survivors
        // atomically and returns them via RETURNING.
        let sql = format!(
            "WITH claimed AS ( \
                 SELECT id FROM execution_control_queue \
                 WHERE status = 'Pending' \
                 ORDER BY issued_at \
                 LIMIT $1 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE execution_control_queue e \
             SET status = 'Processing', processed_at = NOW(), processed_by = $2 \
             FROM claimed \
             WHERE e.id = claimed.id \
             RETURNING {SELECT_COLS}"
        );
        let rows = sqlx::query_as::<_, EntryTuple>(&sql)
            .bind(i64::from(batch_size))
            .bind(processor)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_db_err("control_queue", e))?;
        rows.into_iter().map(tuple_to_entry).collect()
    }

    async fn mark_completed(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query("UPDATE execution_control_queue SET status = 'Completed' WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| map_db_err("control_queue", e))?;
        Ok(())
    }

    async fn mark_failed(&self, id: &[u8], error: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Failed', error_message = $2 \
             WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: std::time::Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        let secs = reclaim_after_seconds(reclaim_after);
        let max_count = i64::from(max_reclaim_count);

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| map_db_err("control_queue", e))?;

        // Reclaim branch: Processing → Pending, bump reclaim_count, clear
        // processed_at / processed_by. CAS fence: status = 'Processing'
        // (another sweeper's commit flipping status out from under us
        // makes our UPDATE return zero rows for that row).
        let reclaimed = sqlx::query_scalar::<_, Vec<u8>>(
            "UPDATE execution_control_queue \
             SET status = 'Pending', \
                 reclaim_count = reclaim_count + 1, \
                 processed_at = NULL, \
                 processed_by = NULL \
             WHERE status = 'Processing' \
               AND processed_at < NOW() - make_interval(secs => $1) \
               AND reclaim_count < $2 \
             RETURNING id",
        )
        .bind(secs)
        .bind(max_count)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;

        // Exhaust branch: Processing → Failed with canonical message.
        // `encode(processed_by, 'hex')` produces lowercase hex, matching
        // the in-memory `hex_encode_bytes` helper byte-for-byte.
        // `reclaim_count` in the message is the pre-transition value —
        // consistent with the in-memory impl which bumps *after* the
        // decision in the reclaim branch and never bumps in the exhaust
        // branch.
        let exhausted = sqlx::query_scalar::<_, Vec<u8>>(
            "UPDATE execution_control_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: processor ' || \
                                 COALESCE(encode(processed_by, 'hex'), '<unknown>') || \
                                 ' presumed dead after ' || reclaim_count || ' reclaims' \
             WHERE status = 'Processing' \
               AND processed_at < NOW() - make_interval(secs => $1) \
               AND reclaim_count >= $2 \
             RETURNING id",
        )
        .bind(secs)
        .bind(max_count)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;

        tx.commit()
            .await
            .map_err(|e| map_db_err("control_queue", e))?;

        Ok(ReclaimOutcome {
            reclaimed: reclaimed.len() as u64,
            exhausted: exhausted.len() as u64,
        })
    }

    async fn cleanup(&self, _retention: std::time::Duration) -> Result<u64, StorageError> {
        unimplemented!()
    }
}

/// Normalize `reclaim_after` into positive seconds bounded by a sane
/// upper limit. Matches the in-memory impl's intent: a huge
/// `reclaim_after` means "never reclaim anything under realistic
/// processing ages", so we clamp to ~10 years. A negative / NaN /
/// infinite value collapses to 0 so the caller still sees a deterministic
/// result (nothing younger than `now` is reclaimable, but everything
/// older than `now` is — same as the in-memory no-fallback path).
fn reclaim_after_seconds(d: std::time::Duration) -> f64 {
    const TEN_YEARS_SECS: f64 = 86_400.0 * 365.0 * 10.0;
    let secs = d.as_secs_f64();
    if !secs.is_finite() {
        return 0.0;
    }
    secs.clamp(0.0, TEN_YEARS_SECS)
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{DateTime, Utc};
    use sqlx::{Pool, Postgres};

    use super::*;
    use crate::backend::PostgresStorage;

    /// Connect to `DATABASE_URL` and run migrations, or return `None` to skip.
    async fn pool() -> Option<Pool<Postgres>> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("migrations");
        Some(storage.pool().clone())
    }

    /// Module-level lock that serialises tests hitting the shared
    /// `execution_control_queue` table. Nextest would otherwise interleave
    /// `claim_pending` / `reclaim_stuck` calls across tests — each of which
    /// mutates the same global queue state — producing flaky assertions.
    /// The lock scopes only to THIS module (this test binary), so parallel
    /// crates still need DB isolation from the CI harness.
    static TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    /// Wipe the control queue before a test so it sees a deterministic
    /// empty state. Parent `executions` / `workflows` / ... rows from
    /// prior tests are left in place (they use random IDs and do not
    /// conflict); we only reset the table under test.
    async fn clean_control_queue(pool: &Pool<Postgres>) {
        sqlx::query("DELETE FROM execution_control_queue")
            .execute(pool)
            .await
            .expect("clean control queue");
    }

    /// Generate a pseudo-unique 16-byte ID (nanosecond timestamp + counter).
    /// Mirrors `test_support::random_id` without pulling it through a new
    /// cfg path.
    fn random_id() -> Vec<u8> {
        use std::{
            sync::atomic::{AtomicU64, Ordering},
            time::{SystemTime, UNIX_EPOCH},
        };
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&nanos.to_le_bytes()[..8]);
        bytes[8..16].copy_from_slice(&seq.to_le_bytes());
        bytes.to_vec()
    }

    /// Seed a minimal `orgs → workspaces → workflows → workflow_versions →
    /// executions` chain so the `execution_id` FK on
    /// `execution_control_queue` is satisfied. Returns the `execution.id`
    /// to reference from subsequent enqueue calls.
    async fn seed_execution_parent_chain(pool: &Pool<Postgres>) -> Vec<u8> {
        let now = Utc::now();
        let org_id = random_id();
        let ws_id = random_id();
        let wf_id = random_id();
        let wfv_id = random_id();
        let exec_id = random_id();
        let creator = random_id();

        // org
        sqlx::query(
            "INSERT INTO orgs \
             (id, slug, display_name, created_at, created_by, plan) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&org_id)
        .bind(format!("org-{}", hex::encode(&org_id[..4])))
        .bind("Test Org")
        .bind(now)
        .bind(&creator)
        .bind("self_host")
        .execute(pool)
        .await
        .expect("insert org");

        // workspace
        sqlx::query(
            "INSERT INTO workspaces \
             (id, org_id, slug, display_name, created_at, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&ws_id)
        .bind(&org_id)
        .bind(format!("ws-{}", hex::encode(&ws_id[..4])))
        .bind("Test Workspace")
        .bind(now)
        .bind(&creator)
        .execute(pool)
        .await
        .expect("insert workspace");

        // workflow (current_version_id FK is deferred until workflow_versions row exists)
        sqlx::query(
            "INSERT INTO workflows \
             (id, workspace_id, slug, display_name, current_version_id, state, \
              created_at, created_by, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&wf_id)
        .bind(&ws_id)
        .bind(format!("wf-{}", hex::encode(&wf_id[..4])))
        .bind("Test Workflow")
        .bind(&wfv_id)
        .bind("Active")
        .bind(now)
        .bind(&creator)
        .bind(now)
        .execute(pool)
        .await
        .expect("insert workflow");

        // workflow_version
        sqlx::query(
            "INSERT INTO workflow_versions \
             (id, workflow_id, version_number, definition, schema_version, \
              state, created_at, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&wfv_id)
        .bind(&wf_id)
        .bind(1_i32)
        .bind(sqlx::types::Json(serde_json::json!({"nodes": []})))
        .bind(1_i32)
        .bind("Published")
        .bind(now)
        .bind(&creator)
        .execute(pool)
        .await
        .expect("insert workflow_version");

        // execution
        sqlx::query(
            "INSERT INTO executions \
             (id, workspace_id, org_id, workflow_version_id, status, source, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&exec_id)
        .bind(&ws_id)
        .bind(&org_id)
        .bind(&wfv_id)
        .bind("Pending")
        .bind(sqlx::types::Json(serde_json::json!({"kind": "Manual"})))
        .bind(now)
        .execute(pool)
        .await
        .expect("insert execution");

        exec_id
    }

    fn pending_entry(exec_id: &[u8]) -> ControlQueueEntry {
        ControlQueueEntry {
            id: random_id(),
            execution_id: exec_id.to_vec(),
            command: ControlCommand::Cancel,
            issued_by: None,
            issued_at: Utc::now(),
            status: "Pending".to_string(),
            processed_by: None,
            processed_at: None,
            error_message: None,
            reclaim_count: 0,
        }
    }

    #[tokio::test]
    async fn enqueue_then_read_back_row() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();

        repo.enqueue(&entry).await.expect("enqueue");

        type Row = (
            Vec<u8>,
            Vec<u8>,
            String,
            Option<Vec<u8>>,
            DateTime<Utc>,
            String,
            Option<DateTime<Utc>>,
            Option<Vec<u8>>,
            Option<String>,
            i64,
        );
        let row: Row = sqlx::query_as(
            "SELECT id, execution_id, command, issued_by, issued_at, status, \
                    processed_at, processed_by, error_message, reclaim_count \
             FROM execution_control_queue WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .expect("select");

        assert_eq!(row.0, row_id);
        assert_eq!(row.1, exec_id);
        assert_eq!(row.2, "Cancel");
        assert!(row.3.is_none());
        assert_eq!(row.5, "Pending");
        assert!(row.6.is_none());
        assert!(row.7.is_none());
        assert!(row.8.is_none());
        assert_eq!(row.9, 0);
    }

    #[tokio::test]
    async fn claim_pending_stamps_processed_at_and_processed_by() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();

        let before = Utc::now();
        let claimed = repo.claim_pending(b"runner-a", 16).await.unwrap();
        let after = Utc::now();

        assert!(
            claimed.iter().any(|e| e.id == row_id),
            "our enqueued row should be in the claim batch"
        );

        type Row = (String, Option<Vec<u8>>, Option<DateTime<Utc>>);
        let row: Row = sqlx::query_as(
            "SELECT status, processed_by, processed_at FROM execution_control_queue \
             WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .expect("select");

        assert_eq!(row.0, "Processing");
        assert_eq!(row.1.as_deref(), Some(b"runner-a".as_slice()));
        let ts = row.2.expect("processed_at stamped");
        assert!(
            ts >= before && ts <= after,
            "processed_at inside the claim window"
        );
    }

    #[tokio::test]
    async fn mark_completed_transitions_status() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let _ = repo.claim_pending(b"runner-a", 1).await.unwrap();

        repo.mark_completed(&row_id).await.unwrap();

        let status: String =
            sqlx::query_scalar("SELECT status FROM execution_control_queue WHERE id = $1")
                .bind(&row_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "Completed");
    }

    #[tokio::test]
    async fn mark_failed_records_error_message() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let _ = repo.claim_pending(b"runner-a", 1).await.unwrap();

        repo.mark_failed(&row_id, "dispatch boom").await.unwrap();

        type Row = (String, Option<String>);
        let row: Row = sqlx::query_as(
            "SELECT status, error_message FROM execution_control_queue WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Failed");
        assert_eq!(row.1.as_deref(), Some("dispatch boom"));
    }

    #[tokio::test]
    async fn reclaim_stuck_moves_expired_processing_to_pending() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        // Enqueue a row then force it into Processing with an ancient
        // processed_at and processed_by so reclaim_stuck picks it up.
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let stale_at = Utc::now() - chrono::Duration::seconds(600);
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Processing', processed_at = $2, processed_by = $3, \
                 reclaim_count = 0 \
             WHERE id = $1",
        )
        .bind(&row_id)
        .bind(stale_at)
        .bind(b"dead-runner".as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 1);
        assert_eq!(outcome.exhausted, 0);

        type Row = (String, i64, Option<DateTime<Utc>>, Option<Vec<u8>>);
        let row: Row = sqlx::query_as(
            "SELECT status, reclaim_count, processed_at, processed_by \
             FROM execution_control_queue WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Pending");
        assert_eq!(row.1, 1);
        assert!(row.2.is_none(), "processed_at cleared on reclaim");
        assert!(row.3.is_none(), "processed_by cleared on reclaim");
    }

    #[tokio::test]
    async fn reclaim_stuck_leaves_fresh_processing_alone() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let fresh_at = Utc::now() - chrono::Duration::seconds(10);
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Processing', processed_at = $2, \
                 processed_by = $3, reclaim_count = 0 \
             WHERE id = $1",
        )
        .bind(&row_id)
        .bind(fresh_at)
        .bind(b"runner-a".as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 0);
        assert_eq!(outcome.exhausted, 0);

        let status: String =
            sqlx::query_scalar("SELECT status FROM execution_control_queue WHERE id = $1")
                .bind(&row_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "Processing", "fresh row untouched");
    }

    #[tokio::test]
    async fn reclaim_stuck_leaves_non_processing_rows_alone() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        // Three rows: Completed, Failed, Pending — none in Processing.
        for status in ["Completed", "Failed", "Pending"] {
            let mut entry = pending_entry(&exec_id);
            entry.status = status.to_string();
            repo.enqueue(&entry).await.unwrap();
        }

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 0);
        assert_eq!(outcome.exhausted, 0);
    }
}
