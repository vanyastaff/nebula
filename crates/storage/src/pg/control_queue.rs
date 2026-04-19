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
    // `reclaim_count` is BIGINT in the DB and u32 on the trait. A negative
    // or overflowing value would be persisted corruption — silently
    // saturating (`max(0).unwrap_or(u32::MAX)`) would hide the bug from
    // reclaim/exhaust decision paths. Surface it instead.
    let reclaim_count = u32::try_from(t.9).map_err(|_| {
        StorageError::Serialization(format!(
            "invalid control_queue.reclaim_count: {} (id={})",
            t.9,
            hex::encode(&t.0)
        ))
    })?;
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
        reclaim_count,
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
        // Fence on `status = 'Processing'`: a stale worker whose row was
        // reclaimed (→ Pending, now re-claimed by someone else) or moved
        // to a terminal state must not be able to overwrite the newer
        // state. A missed update is an idempotent no-op under the
        // at-least-once contract of ADR-0008 §5.
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Completed' \
             WHERE id = $1 AND status = 'Processing'",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;
        Ok(())
    }

    async fn mark_failed(&self, id: &[u8], error: &str) -> Result<(), StorageError> {
        // Same CAS fence as `mark_completed` — a stale worker must not
        // overwrite an already-terminal or re-claimed row. In particular,
        // this prevents flipping a `reclaim_exhausted` Failed row back
        // to a worker-reported Failed with a different error message.
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Failed', error_message = $2 \
             WHERE id = $1 AND status = 'Processing'",
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
        let secs = duration_to_interval_secs(reclaim_after);
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

    async fn cleanup(&self, retention: std::time::Duration) -> Result<u64, StorageError> {
        // Only delete rows in terminal states. Canon §12.2 explicitly
        // treats "removing rows before the engine has acted" as broken,
        // so Pending / Processing rows must never be pruned regardless
        // of `issued_at` age.
        let secs = duration_to_interval_secs(retention);
        let result = sqlx::query(
            "DELETE FROM execution_control_queue \
             WHERE status IN ('Completed', 'Failed') \
               AND issued_at < NOW() - make_interval(secs => $1)",
        )
        .bind(secs)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("control_queue", e))?;
        Ok(result.rows_affected())
    }
}

/// Convert a `std::time::Duration` into `make_interval(secs => ...)`-safe
/// seconds. Clamps to `[0, i32::MAX]` seconds (~68 years) so non-finite or
/// absurdly large inputs never overflow Postgres's interval arithmetic.
///
/// For `reclaim_stuck`, a clamp of ~68 years mirrors the in-memory impl's
/// intent — `reclaim_after` values that dwarf real processing ages make
/// the staleness cutoff effectively unreachable, so no row is reclaimed.
/// For `cleanup`, the same clamp lets `retention = Duration::MAX` act as
/// "prune rows older than 68 years" — again, a practical never.
fn duration_to_interval_secs(d: std::time::Duration) -> f64 {
    const MAX_INTERVAL_SECS: f64 = i32::MAX as f64;
    let secs = d.as_secs_f64();
    if !secs.is_finite() {
        return 0.0;
    }
    secs.clamp(0.0, MAX_INTERVAL_SECS)
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{DateTime, Utc};
    use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

    use super::*;
    use crate::test_support::random_id;

    /// Spec-16 Postgres migrator. Distinct from `PostgresStorage::run_migrations`,
    /// which runs the layer-1 (storage_kv / simple workflow) schema. Our tests
    /// bind against the `orgs / workspaces / workflows / workflow_versions /
    /// executions / execution_control_queue` family defined by migrations
    /// 0001-0021 under `crates/storage/migrations/postgres/`.
    static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

    /// Runs spec-16 migrations at most once per test-binary run. `sqlx::Migrator`
    /// itself is idempotent (tracked via `_sqlx_migrations`), but serialising
    /// the call across tests avoids wasted round-trips and the migrator's
    /// internal advisory-lock contention.
    static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    /// Connect to `DATABASE_URL`, apply spec-16 migrations (once), or return
    /// `None` to skip this test. `DATABASE_URL` absent → skip; `DATABASE_URL`
    /// present but invalid Unicode → fail with context so Postgres coverage
    /// is never silently disabled by a misconfigured environment.
    async fn pool() -> Option<Pool<Postgres>> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => return None,
            Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
        };
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect");
        SCHEMA_READY
            .get_or_init(|| async {
                SPEC16_MIGRATOR
                    .run(&pool)
                    .await
                    .expect("spec-16 postgres migrations");
                // 0007 adds `workflows.current_version_id` as a NOT NULL FK
                // to `workflow_versions(id)` with the default NOT DEFERRABLE
                // semantics, creating a chicken-and-egg cycle at INSERT time
                // (workflow needs a real version_id; version_id needs a real
                // workflow_id). We can't resolve this via a normal seed.
                // Relax the constraint for the test binary so the seed
                // helper can wrap both inserts in a transaction and commit
                // them atomically. The production composition root would
                // either use the repo-root `migrations/` schema (which
                // already marks this FK DEFERRABLE INITIALLY DEFERRED) or
                // land a targeted migration for the spec-16 schema; this is
                // a test-only workaround, not a canonical fix.
                sqlx::query(
                    "ALTER TABLE workflows \
                     ALTER CONSTRAINT fk_workflows_current_version \
                     DEFERRABLE INITIALLY DEFERRED",
                )
                .execute(&pool)
                .await
                .expect("relax workflows FK for tests");
            })
            .await;
        Some(pool)
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

    /// Seed a minimal `orgs → workspaces → workflows → workflow_versions →
    /// executions` chain so the `execution_id` FK on
    /// `execution_control_queue` is satisfied. Returns the `execution.id`
    /// to reference from subsequent enqueue calls.
    ///
    /// The insert order is `orgs → workspaces → workflows →
    /// workflow_versions → executions`, which crosses the
    /// `workflows.current_version_id ↔ workflow_versions.id` FK cycle.
    /// This is only resolvable when the FK is DEFERRABLE — the `pool()`
    /// helper promotes the constraint to `DEFERRABLE INITIALLY DEFERRED`
    /// exactly once per test binary, so here we wrap the inserts in a
    /// transaction with `SET CONSTRAINTS ALL DEFERRED` and the cycle is
    /// checked at COMMIT rather than at each INSERT.
    async fn seed_execution_parent_chain(pool: &Pool<Postgres>) -> Vec<u8> {
        let now = Utc::now();
        let org_id = random_id();
        let ws_id = random_id();
        let wf_id = random_id();
        let wfv_id = random_id();
        let exec_id = random_id();
        let creator = random_id();

        let mut tx = pool.begin().await.expect("begin seed tx");
        sqlx::query("SET CONSTRAINTS ALL DEFERRED")
            .execute(&mut *tx)
            .await
            .expect("defer constraints");

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
        .execute(&mut *tx)
        .await
        .expect("insert org");

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
        .execute(&mut *tx)
        .await
        .expect("insert workspace");

        // workflow references workflow_versions.id which does not exist yet —
        // FK deferred to COMMIT.
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
        .execute(&mut *tx)
        .await
        .expect("insert workflow");

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
        .execute(&mut *tx)
        .await
        .expect("insert workflow_version");

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
        .execute(&mut *tx)
        .await
        .expect("insert execution");

        tx.commit().await.expect("commit seed tx");

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
    async fn mark_completed_is_noop_when_row_not_processing() {
        // Stale-worker safety: an ack arriving after the row has been
        // reclaimed (→ Pending) or already terminalised must not
        // overwrite the newer status. The guarded UPDATE returns zero
        // rows; the repo method still returns Ok under at-least-once
        // semantics (ADR-0008 §5).
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        // Case 1: row is Pending (never claimed). mark_completed is a no-op.
        let pending = pending_entry(&exec_id);
        let pending_id = pending.id.clone();
        repo.enqueue(&pending).await.unwrap();
        repo.mark_completed(&pending_id).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM execution_control_queue WHERE id = $1")
                .bind(&pending_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            status, "Pending",
            "mark_completed must not flip a Pending row"
        );

        // Case 2: row was Failed by a prior reclaim exhaustion.
        // mark_failed must not overwrite the exhaust error_message.
        let mut exhausted = pending_entry(&exec_id);
        exhausted.status = "Failed".to_string();
        exhausted.error_message = Some(
            "reclaim exhausted: processor deadbeef presumed dead after 3 reclaims".to_string(),
        );
        let exhausted_id = exhausted.id.clone();
        repo.enqueue(&exhausted).await.unwrap();
        repo.mark_failed(&exhausted_id, "late worker error")
            .await
            .unwrap();
        type Row = (String, Option<String>);
        let row: Row = sqlx::query_as(
            "SELECT status, error_message FROM execution_control_queue WHERE id = $1",
        )
        .bind(&exhausted_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Failed");
        assert!(
            row.1
                .as_deref()
                .is_some_and(|m| m.starts_with("reclaim exhausted:")),
            "mark_failed must not overwrite an exhaust message, got: {:?}",
            row.1
        );
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

    #[tokio::test]
    async fn reclaim_stuck_exhausts_after_max_count() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let stale_at = Utc::now() - chrono::Duration::seconds(600);
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Processing', processed_at = $2, \
                 processed_by = $3, reclaim_count = 3 \
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
        assert_eq!(outcome.reclaimed, 0, "past budget — not requeued");
        assert_eq!(outcome.exhausted, 1, "moved to Failed");

        type Row = (String, Option<String>);
        let row: Row = sqlx::query_as(
            "SELECT status, error_message FROM execution_control_queue \
             WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Failed");
        let msg = row.1.as_deref().expect("error_message set");
        // Byte-for-byte parity with InMemoryControlQueueRepo.
        assert!(
            msg.starts_with("reclaim exhausted: processor "),
            "canonical prefix, got: {msg}"
        );
        assert!(
            msg.contains("presumed dead after 3 reclaims"),
            "includes reclaim count, got: {msg}"
        );
        assert!(
            msg.contains("646561642d72756e6e6572"),
            "processor_id encoded as lowercase hex, got: {msg}"
        );
    }

    #[tokio::test]
    async fn cleanup_deletes_old_terminal_rows_only() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        // Insert three old rows: Completed, Failed, Pending; one fresh
        // Completed row that must survive because of age.
        let old = Utc::now() - chrono::Duration::seconds(3600);
        let fresh = Utc::now();
        let mut ids_old = Vec::new();
        for status in ["Completed", "Failed", "Pending"] {
            let mut entry = pending_entry(&exec_id);
            entry.status = status.to_string();
            entry.issued_at = old;
            ids_old.push((entry.id.clone(), status));
            repo.enqueue(&entry).await.unwrap();
        }
        let mut fresh_entry = pending_entry(&exec_id);
        fresh_entry.status = "Completed".to_string();
        fresh_entry.issued_at = fresh;
        let fresh_id = fresh_entry.id.clone();
        repo.enqueue(&fresh_entry).await.unwrap();

        // retention = 10 minutes — old rows are past, fresh row is under.
        let deleted = repo
            .cleanup(std::time::Duration::from_secs(600))
            .await
            .unwrap();
        assert_eq!(deleted, 2, "only old Completed + old Failed removed");

        // Verify survivors.
        for (id, status) in ids_old {
            let rows: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM execution_control_queue WHERE id = $1")
                    .bind(&id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            let expected_rows = if status == "Pending" { 1 } else { 0 };
            assert_eq!(
                rows, expected_rows,
                "row with status {status} expected {expected_rows} row(s)"
            );
        }
        let fresh_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM execution_control_queue WHERE id = $1")
                .bind(&fresh_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(fresh_rows, 1, "fresh Completed row survives cleanup");
    }

    #[tokio::test]
    async fn claim_pending_skip_locked_prevents_double_claim() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = std::sync::Arc::new(PgControlQueueRepo::new(pool.clone()));
        let exec_id = seed_execution_parent_chain(&pool).await;

        // Enqueue a batch of 20 pending rows.
        let mut enqueued_ids = std::collections::HashSet::new();
        for _ in 0..20 {
            let entry = pending_entry(&exec_id);
            enqueued_ids.insert(entry.id.clone());
            repo.enqueue(&entry).await.unwrap();
        }

        // Fire two concurrent claimers; together they should cover
        // exactly 20 rows with zero overlap.
        let repo_a = repo.clone();
        let repo_b = repo.clone();
        let h_a = tokio::spawn(async move { repo_a.claim_pending(b"runner-a", 20).await.unwrap() });
        let h_b = tokio::spawn(async move { repo_b.claim_pending(b"runner-b", 20).await.unwrap() });
        let claimed_a = h_a.await.unwrap();
        let claimed_b = h_b.await.unwrap();

        let ids_a: std::collections::HashSet<_> = claimed_a.iter().map(|e| e.id.clone()).collect();
        let ids_b: std::collections::HashSet<_> = claimed_b.iter().map(|e| e.id.clone()).collect();
        let overlap: Vec<_> = ids_a.intersection(&ids_b).collect();
        assert!(
            overlap.is_empty(),
            "runners claimed the same row twice: {:?}",
            overlap
        );
        // The 20 rows we enqueued here must all be among the claimed set
        // (union). Other test runs may have left rows; we don't assert
        // the total, just our slice.
        let union: std::collections::HashSet<_> = ids_a.union(&ids_b).cloned().collect();
        for id in &enqueued_ids {
            assert!(
                union.contains(id),
                "our enqueued row missing from claim union: {id:?}"
            );
        }
    }

    #[tokio::test]
    async fn reclaim_stuck_safe_under_concurrent_sweep() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = std::sync::Arc::new(PgControlQueueRepo::new(pool.clone()));
        let exec_id = seed_execution_parent_chain(&pool).await;

        // One stuck row. Two sweepers race to reclaim it.
        let entry = pending_entry(&exec_id);
        let row_id = entry.id.clone();
        repo.enqueue(&entry).await.unwrap();
        let stale_at = Utc::now() - chrono::Duration::seconds(600);
        sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Processing', processed_at = $2, \
                 processed_by = $3, reclaim_count = 0 \
             WHERE id = $1",
        )
        .bind(&row_id)
        .bind(stale_at)
        .bind(b"dead-runner".as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let repo_a = repo.clone();
        let repo_b = repo.clone();
        let h_a = tokio::spawn(async move {
            repo_a
                .reclaim_stuck(std::time::Duration::from_secs(150), 3)
                .await
                .unwrap()
        });
        let h_b = tokio::spawn(async move {
            repo_b
                .reclaim_stuck(std::time::Duration::from_secs(150), 3)
                .await
                .unwrap()
        });
        let out_a = h_a.await.unwrap();
        let out_b = h_b.await.unwrap();

        // Exactly one sweeper reclaimed exactly one row; the other
        // sweeper observed zero-rows-affected. No sweeper exhausted
        // anything (reclaim_count was 0).
        assert_eq!(out_a.reclaimed + out_b.reclaimed, 1);
        assert_eq!(out_a.exhausted + out_b.exhausted, 0);

        // Row is now Pending with reclaim_count == 1 — exactly once,
        // not twice.
        type Row = (String, i64);
        let row: Row = sqlx::query_as(
            "SELECT status, reclaim_count FROM execution_control_queue \
             WHERE id = $1",
        )
        .bind(&row_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Pending");
        assert_eq!(row.1, 1, "reclaim_count bumped exactly once");
    }

    #[tokio::test]
    async fn enqueue_roundtrip_preserves_every_command_variant() {
        let Some(pool) = pool().await else { return };
        let _guard = TEST_LOCK.lock().await;
        clean_control_queue(&pool).await;
        let repo = PgControlQueueRepo::new(pool.clone());
        let exec_id = seed_execution_parent_chain(&pool).await;

        let variants = [
            ControlCommand::Start,
            ControlCommand::Cancel,
            ControlCommand::Terminate,
            ControlCommand::Resume,
            ControlCommand::Restart,
        ];
        let mut enqueued_ids = Vec::new();
        for cmd in variants {
            let mut entry = pending_entry(&exec_id);
            entry.command = cmd;
            enqueued_ids.push((entry.id.clone(), cmd));
            repo.enqueue(&entry).await.unwrap();
        }

        let claimed = repo.claim_pending(b"variant-runner", 64).await.unwrap();
        // Build a map of id → decoded command so ordering doesn't matter.
        let decoded: std::collections::HashMap<_, _> =
            claimed.iter().map(|e| (e.id.clone(), e.command)).collect();
        for (id, expected) in enqueued_ids {
            let got = decoded
                .get(&id)
                .copied()
                .unwrap_or_else(|| panic!("row {id:?} missing from claim batch"));
            assert_eq!(got, expected, "command roundtrip mismatch");
        }
    }
}
