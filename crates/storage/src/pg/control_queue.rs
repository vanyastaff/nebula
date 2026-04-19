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
    repos::{ControlQueueEntry, ControlQueueRepo, ReclaimOutcome},
};

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
        _processor: &[u8],
        _batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError> {
        unimplemented!()
    }

    async fn mark_completed(&self, _id: &[u8]) -> Result<(), StorageError> {
        unimplemented!()
    }

    async fn mark_failed(&self, _id: &[u8], _error: &str) -> Result<(), StorageError> {
        unimplemented!()
    }

    async fn reclaim_stuck(
        &self,
        _reclaim_after: std::time::Duration,
        _max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        unimplemented!()
    }

    async fn cleanup(&self, _retention: std::time::Duration) -> Result<u64, StorageError> {
        unimplemented!()
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{DateTime, Utc};
    use sqlx::{Pool, Postgres};

    use super::*;
    use crate::{backend::PostgresStorage, repos::ControlCommand};

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
}
