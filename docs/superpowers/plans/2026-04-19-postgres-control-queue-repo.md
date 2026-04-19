# Postgres `ControlQueueRepo` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land a Postgres-backed `ControlQueueRepo` implementation so the durable control plane (canon §12.2) actually works in multi-process / restart-tolerant deployments. Today only `InMemoryControlQueueRepo` exists; this closes the last structural gap between the API producer and the engine consumer described by ADR-0008 + ADR-0017.

**Architecture:**
- New module `crates/storage/src/pg/control_queue.rs` holds `PgControlQueueRepo`, alongside existing `PgOrgRepo` / `PgWorkspaceRepo`. It follows that file's exact pattern: `sqlx::Pool<Postgres>` field, `map_db_err` for error translation, tuple-decoded `SELECT`, tests gated behind `#[cfg(all(test, feature = "postgres"))]` + `DATABASE_URL` env skip.
- `claim_pending` uses the canonical Postgres `WITH claimed AS (... FOR UPDATE SKIP LOCKED) UPDATE ... FROM claimed RETURNING ...` idiom (ADR-0008 §1).
- `reclaim_stuck` runs two targeted `UPDATE ... WHERE status = 'Processing' AND processed_at < NOW() - make_interval(...) AND reclaim_count <|>= $N RETURNING id` statements inside a single transaction — the `status = 'Processing'` predicate acts as the atomic CAS fence under concurrent sweepers (Postgres READ COMMITTED row-level locking re-evaluates the WHERE after blocking on a concurrent writer). The exhausted-message is built server-side via `COALESCE(encode(processed_by, 'hex'), '<unknown>')` so the lowercase-hex encoding matches `InMemoryControlQueueRepo::hex_encode_bytes` byte-for-byte.
- `reclaim_count: u32` on `ControlQueueEntry` maps to `BIGINT` in the DB; conversions use `i64::from(u32)` outbound and `u32::try_from(v.max(0)).unwrap_or(u32::MAX)` inbound.
- Tests need a real `executions` row to satisfy the `execution_id REFERENCES executions(id)` FK from migration 0013. A shared test helper `seed_execution_parent_chain` inserts the minimal `orgs → workspaces → workflows → workflow_versions → executions` chain with random IDs per test.

**Tech Stack:**
- Rust 2024, edition 1.94, `async_trait`, `sqlx 0.8` (`postgres`, `chrono`, `migrate` features), `hex 0.4`.
- Existing in-repo crates: `nebula-storage` (trait + in-memory impl already in `crates/storage/src/repos/control_queue.rs`).
- Canon: `docs/PRODUCT_CANON.md §12.2`, `docs/adr/0008-execution-control-queue-consumer.md` (decisions 1 + 5), `docs/adr/0017-control-queue-reclaim-policy.md`.

**Pre-read before starting:**
- `crates/storage/src/repos/control_queue.rs` — trait + reference in-memory impl; the test matrix (`claim_pending_stamps_...`, `reclaim_stuck_*`) is what the Postgres impl must mirror.
- `crates/storage/src/pg/org.rs` — canonical style template (tuple types, `query_as`, `map_db_err`, test skip-if-no-DB).
- `crates/storage/migrations/postgres/0013_execution_lifecycle.sql` + `0021_add_control_queue_reclaim_count.sql` — schema you bind against. **Do NOT add migration 0022 — schema is complete.**
- `docs/PRODUCT_CANON.md §12.2` (L2 invariant), §11.6 (doc-truth rule).

**Out of scope (do not sprawl):**
- `LISTEN/NOTIFY` wake-up optimisation (ADR-0008 names it as additive; polling stays authoritative).
- `apps/server` production composition root; the `PgControlQueueRepo` lives waiting for it.
- Counter metric `nebula_engine_control_reclaim_total` (separate chip).
- Cross-runner `processor_id` liveness detection (ADR-0017 explicitly out of scope).
- Changing `execution_id` encoding (still UTF-8 bytes of the ULID string — load-bearing per the `ControlQueueEntry` doc comment).

---

## File structure

**Create:**
- `crates/storage/src/pg/control_queue.rs` — `PgControlQueueRepo` with all five `ControlQueueRepo` methods + private `decode_command` / `reclaim_after_seconds` helpers + `#[cfg(all(test, feature = "postgres"))]` test module.

**Modify:**
- `crates/storage/src/pg/mod.rs` — add `mod control_queue;` + `pub use control_queue::PgControlQueueRepo;`.
- `crates/storage/src/repos/mod.rs` — update status-table row: drop "only in-memory" qualifier; mention the new `pg::PgControlQueueRepo`.
- `crates/storage/src/lib.rs` — minor doc touch in the `repos` module `//!` so §11.6 stays truthful (strike the exception wording that singles out in-memory only).
- `docs/MATURITY.md` — add a "last targeted revision" line noting this PR landed the Postgres `ControlQueueRepo`; no column change (the row stays `partial`/`stable` — this closes one piece, not the whole storage surface).

**No schema changes.** Migration 0021 already exists on `main` and supplies `reclaim_count` + the partial index on `(processed_at) WHERE status = 'Processing'`.

---

## Task 0: Verify baseline before touching anything

- [ ] **Step 0.1: Confirm in-memory tests + workspace build are green on this branch before any edits.**

Run:
```
cargo nextest run -p nebula-storage --lib
```
Expected: all `control_queue::tests::*` tests pass (baseline for behavioral parity claims later).

- [ ] **Step 0.2: Confirm the `postgres` feature still compiles unchanged.**

Run:
```
cargo check -p nebula-storage --features postgres
```
Expected: clean build, no warnings.

No commit.

---

## Task 1: Skeleton `PgControlQueueRepo` — struct + `enqueue` (TDD)

**Files:**
- Create: `crates/storage/src/pg/control_queue.rs`
- Modify: `crates/storage/src/pg/mod.rs`

- [ ] **Step 1.1: Add module wiring in `pg/mod.rs`.**

Modify `crates/storage/src/pg/mod.rs` — add `control_queue` to the module list and re-export the struct. Full post-edit file:

```rust
//! PostgreSQL implementations of repository traits.
//!
//! Each module in this directory implements exactly one repo trait from
//! `crate::repos`. All implementations share:
//!
//! - a `sqlx::Pool<Postgres>` for connection management
//! - the `map_db_err` helper for translating `sqlx::Error` into `StorageError`
//! - SQLSTATE `23505` (unique violation) → `StorageError::Duplicate`
//!
//! # Testing
//!
//! Tests are gated behind `cfg(all(test, feature = "postgres"))` and
//! are skipped when `DATABASE_URL` is not set in the environment.

use sqlx::Error as SqlxError;

use crate::error::StorageError;

mod control_queue;
mod org;
mod workspace;

pub use control_queue::PgControlQueueRepo;
pub use org::PgOrgRepo;
pub use workspace::PgWorkspaceRepo;

/// Translate an [`sqlx::Error`] into a [`StorageError`].
///
/// Most errors become [`StorageError::Connection`]. Unique-constraint
/// violations (SQLSTATE `23505`) become [`StorageError::Duplicate`]
/// with the constraint detail preserved.
pub(crate) fn map_db_err(entity: &'static str, err: SqlxError) -> StorageError {
    if let SqlxError::Database(db_err) = &err
        && db_err.code().as_deref() == Some("23505")
    {
        return StorageError::Duplicate {
            entity,
            detail: db_err.message().to_string(),
        };
    }
    StorageError::Connection(err.to_string())
}
```

- [ ] **Step 1.2: Write the failing test for `enqueue` + roundtrip-read.**

Create `crates/storage/src/pg/control_queue.rs` with the skeleton (empty `impl` body so compilation fails in a targeted way):

```rust
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

/// Postgres-backed durable control queue (canon §12.2).
///
/// Implements the [`ControlQueueRepo`] trait against the
/// `execution_control_queue` table defined by migration 0013 + 0021.
///
/// - `claim_pending` uses `FOR UPDATE SKIP LOCKED` per ADR-0008 §1; two
///   concurrent claimers never double-claim a row.
/// - `reclaim_stuck` runs two `UPDATE ... WHERE status = 'Processing' ...
///   RETURNING id` statements inside one transaction; the
///   `status = 'Processing'` predicate is the CAS that fences concurrent
///   sweepers (ADR-0017). The exhausted-message encodes `processed_by` as
///   lowercase hex to stay byte-identical with `InMemoryControlQueueRepo`.
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
    async fn enqueue(&self, _entry: &ControlQueueEntry) -> Result<(), StorageError> {
        unimplemented!()
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
    use super::*;
    use crate::backend::postgres::PostgresStorage;
    use chrono::{DateTime, Utc};
    use sqlx::{Pool, Postgres};

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
```

- [ ] **Step 1.3: Run to confirm failure.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::enqueue_then_read_back_row
```
Expected: If `DATABASE_URL` is unset the test returns early (no-op). If set, `unimplemented!()` panics. Either confirms the wiring is live.

- [ ] **Step 1.4: Implement `enqueue`.**

Replace the `enqueue` body in `crates/storage/src/pg/control_queue.rs`:

```rust
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
```

- [ ] **Step 1.5: Run test to verify passing (or skipped if no DB).**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::enqueue_then_read_back_row
```
Expected: PASS when `DATABASE_URL` is set; skip (empty run) otherwise.

- [ ] **Step 1.6: Commit.**

```
git add crates/storage/src/pg/mod.rs crates/storage/src/pg/control_queue.rs
git commit -m "feat(storage): PgControlQueueRepo skeleton + enqueue (ADR-0008)"
```

---

## Task 2: `claim_pending` with `FOR UPDATE SKIP LOCKED` (TDD)

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 2.1: Write the failing test — stamp roundtrip.**

Append inside the `tests` module:

```rust
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
```

- [ ] **Step 2.2: Run to confirm failure (panics on `unimplemented!()`).**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::claim_pending_stamps_processed_at_and_processed_by
```
Expected: FAIL or skip.

- [ ] **Step 2.3: Implement `claim_pending` + a private `decode_command` helper.**

Add near the top of `control_queue.rs` (inside the module, above `impl ControlQueueRepo`):

```rust
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

const SELECT_COLS: &str =
    "id, execution_id, command, issued_by, issued_at, status, processed_by, \
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
```

Replace the `claim_pending` body with:

```rust
    async fn claim_pending(
        &self,
        processor: &[u8],
        batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError> {
        // Canonical Postgres SKIP LOCKED claim (ADR-0008 §1).
        // The CTE's SELECT ... FOR UPDATE SKIP LOCKED skips rows another
        // runner has already locked; the outer UPDATE stamps the survivors
        // atomically and RETURNs them.
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
```

- [ ] **Step 2.4: Run test — verify passing (or skipped).**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::claim_pending_stamps_processed_at_and_processed_by
```
Expected: PASS when `DATABASE_URL` is set.

- [ ] **Step 2.5: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "feat(storage): claim_pending with FOR UPDATE SKIP LOCKED (ADR-0008 §1)"
```

---

## Task 3: `mark_completed` + `mark_failed` (TDD)

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 3.1: Write the failing test.**

Append inside the `tests` module:

```rust
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

        let status: String = sqlx::query_scalar(
            "SELECT status FROM execution_control_queue WHERE id = $1",
        )
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
```

- [ ] **Step 3.2: Run to confirm failure.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::mark_completed_transitions_status pg::control_queue::tests::mark_failed_records_error_message
```
Expected: FAIL or skip.

- [ ] **Step 3.3: Implement both methods.**

Replace `mark_completed` + `mark_failed` bodies:

```rust
    async fn mark_completed(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE execution_control_queue SET status = 'Completed' WHERE id = $1",
        )
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
```

- [ ] **Step 3.4: Run tests — verify passing.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::mark_completed_transitions_status pg::control_queue::tests::mark_failed_records_error_message
```
Expected: PASS (or skip).

- [ ] **Step 3.5: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "feat(storage): mark_completed + mark_failed for PgControlQueueRepo"
```

---

## Task 4: `reclaim_stuck` reclaim branch (TDD)

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 4.1: Write the failing test — mirror `reclaim_stuck_moves_expired_processing_to_pending` from in-memory.**

Append inside `tests` module:

```rust
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

        let status: String = sqlx::query_scalar(
            "SELECT status FROM execution_control_queue WHERE id = $1",
        )
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
```

- [ ] **Step 4.2: Run to confirm failure.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::reclaim_stuck_moves_expired_processing_to_pending
```
Expected: FAIL or skip.

- [ ] **Step 4.3: Implement `reclaim_stuck` (both branches — reclaim + exhaust — in one transaction).**

Add a helper at the bottom of the file (before `#[cfg(test)]`):

```rust
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
```

Replace the `reclaim_stuck` body:

```rust
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
```

- [ ] **Step 4.4: Run tests — verify passing.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::reclaim_stuck_moves_expired_processing_to_pending pg::control_queue::tests::reclaim_stuck_leaves_fresh_processing_alone pg::control_queue::tests::reclaim_stuck_leaves_non_processing_rows_alone
```
Expected: PASS (or skip).

- [ ] **Step 4.5: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "feat(storage): reclaim_stuck reclaim + exhaust branches (ADR-0017)"
```

---

## Task 5: `reclaim_stuck` exhaust message parity (TDD)

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

The exhaust branch is implemented in Task 4 — this task **adds the behavioral-parity test** that locks the message byte-for-byte to the in-memory impl (canonical format + hex encoding).

- [ ] **Step 5.1: Write the test.**

Append inside the `tests` module:

```rust
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
```

- [ ] **Step 5.2: Run to confirm passing (implementation already in place from Task 4).**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::reclaim_stuck_exhausts_after_max_count
```
Expected: PASS.

If failure: compare the generated message against the in-memory format string and tune the SQL `|| ...` concatenation. Do not adjust the assertion — the assertion mirrors the in-memory test exactly; any divergence is a Postgres bug.

- [ ] **Step 5.3: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "test(storage): lock exhaust message parity with in-memory impl"
```

---

## Task 6: `cleanup` restricted to terminal statuses (TDD)

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 6.1: Write the failing test.**

Append inside `tests` module:

```rust
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
            let rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM execution_control_queue WHERE id = $1",
            )
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
        let fresh_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM execution_control_queue WHERE id = $1",
        )
        .bind(&fresh_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(fresh_rows, 1, "fresh Completed row survives cleanup");
    }
```

- [ ] **Step 6.2: Run to confirm failure.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::cleanup_deletes_old_terminal_rows_only
```
Expected: FAIL (panics on `unimplemented!()`).

- [ ] **Step 6.3: Implement `cleanup`.**

Replace `cleanup` body:

```rust
    async fn cleanup(&self, retention: std::time::Duration) -> Result<u64, StorageError> {
        // Only delete rows in terminal states. Canon §12.2 explicitly
        // treats "removing rows before the engine has acted" as broken,
        // so Pending / Processing rows must never be pruned regardless
        // of `issued_at` age.
        let secs = reclaim_after_seconds(retention);
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
```

- [ ] **Step 6.4: Run test — verify passing.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::cleanup_deletes_old_terminal_rows_only
```
Expected: PASS.

- [ ] **Step 6.5: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "feat(storage): cleanup deletes only terminal-status rows past retention"
```

---

## Task 7: Concurrency regression — two claimers never double-claim

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 7.1: Write the regression test for `claim_pending` under concurrency.**

Append inside `tests` module:

```rust
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
        let h_a = tokio::spawn(async move {
            repo_a.claim_pending(b"runner-a", 20).await.unwrap()
        });
        let h_b = tokio::spawn(async move {
            repo_b.claim_pending(b"runner-b", 20).await.unwrap()
        });
        let claimed_a = h_a.await.unwrap();
        let claimed_b = h_b.await.unwrap();

        let ids_a: std::collections::HashSet<_> =
            claimed_a.iter().map(|e| e.id.clone()).collect();
        let ids_b: std::collections::HashSet<_> =
            claimed_b.iter().map(|e| e.id.clone()).collect();
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
```

- [ ] **Step 7.2: Run — verify passing (the SKIP LOCKED implementation in Task 2 already handles this).**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::claim_pending_skip_locked_prevents_double_claim
```
Expected: PASS.

If it fails with some overlap, the `FOR UPDATE SKIP LOCKED` clause has been silently dropped — re-check the SQL in Task 2.

- [ ] **Step 7.3: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "test(storage): concurrent claim_pending never double-claims (SKIP LOCKED regression)"
```

---

## Task 8: Concurrency regression — parallel `reclaim_stuck` sweep is deterministic

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 8.1: Write the regression test.**

Append inside `tests` module:

```rust
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
```

- [ ] **Step 8.2: Run — verify passing.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::reclaim_stuck_safe_under_concurrent_sweep
```
Expected: PASS.

If reclaim_count ends at 2, the two UPDATE statements are not being fenced by `status = 'Processing'` — verify the SQL includes that predicate and that the transaction isolation level is at least READ COMMITTED (sqlx's default).

- [ ] **Step 8.3: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "test(storage): concurrent reclaim_stuck bumps reclaim_count exactly once (ADR-0017)"
```

---

## Task 9: Roundtrip all `ControlCommand` variants

**Files:**
- Modify: `crates/storage/src/pg/control_queue.rs`

- [ ] **Step 9.1: Write the test.**

Append inside `tests` module:

```rust
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
```

- [ ] **Step 9.2: Run — verify passing.**

Run:
```
cargo nextest run -p nebula-storage --features postgres pg::control_queue::tests::enqueue_roundtrip_preserves_every_command_variant
```
Expected: PASS.

- [ ] **Step 9.3: Commit.**

```
git add crates/storage/src/pg/control_queue.rs
git commit -m "test(storage): roundtrip every ControlCommand variant through Postgres"
```

---

## Task 10: Docs sync — truth the new capability (§11.6 canon)

**Files:**
- Modify: `crates/storage/src/repos/mod.rs`
- Modify: `crates/storage/src/lib.rs`
- Modify: `docs/MATURITY.md`

- [ ] **Step 10.1: Update the status table in `repos/mod.rs`.**

Edit `crates/storage/src/repos/mod.rs` — replace the `ControlQueueRepo` row in the `//!` status table. Full new row:

```rust
//! | `ControlQueueRepo` + `InMemoryControlQueueRepo` + `pg::PgControlQueueRepo` | **implemented** | Produced by the API start / cancel handlers; consumed by `nebula_engine::ControlConsumer`. All five commands — `Start` / `Resume` / `Restart` / `Cancel` / `Terminate` — dispatched via `nebula_engine::EngineControlDispatch` (ADR-0008 A2 + A3). Crashed-runner reclaim sweep wired via `reclaim_stuck` (ADR-0008 B1 / ADR-0017). Durable backing exists in two shapes: `InMemoryControlQueueRepo` (tests / local) and `pg::PgControlQueueRepo` (multi-process / restart-tolerant; `FOR UPDATE SKIP LOCKED` per ADR-0008 §1). Safe to depend on as a storage port. |
```

- [ ] **Step 10.2: Update the `repos` module doc in `lib.rs`.**

Edit the `pub mod repos;` doc comment in `crates/storage/src/lib.rs`. Replace:

```
/// Only [`repos::ControlQueueRepo`] + [`repos::InMemoryControlQueueRepo`]
/// are implemented and actually consumed today (by the API cancel path).
```

with:

```
/// Only [`repos::ControlQueueRepo`] is production-wired today —
/// backed by [`repos::InMemoryControlQueueRepo`] (tests / local) and
/// [`pg::PgControlQueueRepo`] (multi-process / restart-tolerant). Both
/// are consumed by `nebula_engine::ControlConsumer`.
```

Also amend the §12.2 comment block in the same file (`## Canon` section — look for `repos::InMemoryControlQueueRepo`-style wording inside the `//!` header) if it singles out in-memory-only.

- [ ] **Step 10.3: Update `docs/MATURITY.md`.**

Locate the "Last targeted revision" line near line 53 and replace it with:

```
Last targeted revision: 2026-04-19 (ADR-0008 B1 / ADR-0017 follow-up: `pg::PgControlQueueRepo` landed — Postgres now honors the durable control plane via `FOR UPDATE SKIP LOCKED` + concurrent-safe `reclaim_stuck`; in-memory + Postgres share one test suite for behavioral parity).
```

(Preserve prior revisions by stacking — if the file already has a later targeted revision, add a new line above it rather than overwriting.)

- [ ] **Step 10.4: Confirm no further §11.6 capability-truthfulness drift.**

Run:
```
grep -rn 'only in-memory' crates/storage/ docs/
```
Expected: zero matches. If any survive, update them.

- [ ] **Step 10.5: Commit.**

```
git add crates/storage/src/repos/mod.rs crates/storage/src/lib.rs docs/MATURITY.md
git commit -m "docs(storage): truth PgControlQueueRepo capability (§11.6)"
```

---

## Task 11: Full gate — canonical verification before PR

- [ ] **Step 11.1: Format.**

Run:
```
cargo +nightly fmt --all
```
Expected: no diff. If any, commit it as `chore: rustfmt`.

- [ ] **Step 11.2: Clippy workspace-wide.**

Run:
```
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: zero warnings.

- [ ] **Step 11.3: Clippy with `postgres` feature on.**

Run:
```
cargo clippy -p nebula-storage --all-targets --features postgres -- -D warnings
```
Expected: zero warnings.

- [ ] **Step 11.4: Nextest, default features.**

Run:
```
cargo nextest run --workspace
```
Expected: all pass. The in-memory `control_queue::tests::*` must still be green (behavioral parity baseline).

- [ ] **Step 11.5: Nextest with `postgres` feature (local DB optional).**

If a Postgres is available:
```
DATABASE_URL=postgres://nebula:nebula@localhost:5432/nebula \
    cargo nextest run -p nebula-storage --features postgres
```
Expected: all tests pass.

If no local Postgres:
```
cargo nextest run -p nebula-storage --features postgres
```
Expected: tests compile + return (the `pool()` helper early-returns on missing `DATABASE_URL`). CI will run the full path.

- [ ] **Step 11.6: Doctest.**

Run:
```
cargo test --workspace --doc
```
Expected: pass.

- [ ] **Step 11.7: Cargo deny.**

Run:
```
cargo deny check
```
Expected: clean — no new advisories or disallowed deps.

- [ ] **Step 11.8: Lefthook pre-push (CI mirror).**

Run:
```
lefthook run pre-push
```
Expected: all jobs pass (fmt, clippy, tests, doctests, taplo, MSRV 1.94, `--all-features`, `--no-default-features`).

- [ ] **Step 11.9: If any gate fails, fix root cause and re-run that gate.**

Do not paper over with `#[allow(...)]` or feature gates. The `postgres` feature path must be clean under both `--features postgres` and default builds.

- [ ] **Step 11.10: Announce evidence via `verify-evidence` skill.**

Before claiming done, invoke `verify-evidence` and paste the output of the four canonical gates (clippy, nextest, doctest, deny) into the summary. This is an Iron Law gate — no "looks good" without fresh command output from this turn.

No commit at this task — the prior tasks already produced the shippable tree.

---

## Task 12: Red-flag self-scan before PR

These are the "are we *actually* done" checks from the original brief. Run each one and confirm before creating the PR.

- [ ] **Step 12.1: `repos/mod.rs` re-exports the new Postgres impl? No silently-private production path.**

Run:
```
grep -n PgControlQueueRepo crates/storage/src/pg/mod.rs crates/storage/src/lib.rs crates/storage/src/repos/mod.rs
```
Expected: `pg/mod.rs` has `pub use control_queue::PgControlQueueRepo`; `repos/mod.rs` docstring references it; `lib.rs` mentions it in the `repos` module description.

- [ ] **Step 12.2: In-memory impl still compiles + tests.**

Run:
```
cargo nextest run -p nebula-storage --lib repos::control_queue
```
Expected: all five existing in-memory tests still pass unchanged.

- [ ] **Step 12.3: No upstream call-site change required.**

Run:
```
grep -rn "InMemoryControlQueueRepo\|ControlQueueRepo" crates/api crates/engine
```
Expected: existing `crates/api` and `crates/engine` code compiles unchanged. The composition root for swapping in `PgControlQueueRepo` is a separate chip — `apps/server` — which this PR does not touch. Confirm no incidental churn crept in.

- [ ] **Step 12.4: Reclaim message format byte-for-byte equal to in-memory?**

Already covered by Task 5. As a belt-and-suspenders check, diff the two format sources visually:

- In-memory: [crates/storage/src/repos/control_queue.rs:261-264](crates/storage/src/repos/control_queue.rs:261) — `format!("reclaim exhausted: processor {processor} presumed dead after {} reclaims", row.reclaim_count)`.
- Postgres: the SQL `||` concatenation in Task 4's `reclaim_stuck` — must emit the same string for the same `(processed_by, reclaim_count)` pair.

If a future refactor changes either side, Task 5's regression test catches the drift.

- [ ] **Step 12.5: ADR-0008 does not need a §5 amendment.**

Read ADR-0008's "Follow-up" list (bottom of `docs/adr/0008-execution-control-queue-consumer.md`). The B1 reclaim path is already marked implemented; the `apps/server` single-production-composition-root bullet is intentionally out of scope for this PR. No amendment required. If wording in the ADR implies that Postgres is still outstanding, tighten it in a follow-up chip — not this PR.

- [ ] **Step 12.6: ADR-0017 does not need a §5 amendment either.**

ADR-0017's "Seam / verification" section references `crates/storage/src/repos/control_queue.rs` — with Postgres landed the seam gets a second backing, but the trait contract is unchanged and the ADR's policy is unchanged. Leave the ADR as-is.

---

## Task 13: Create PR

- [ ] **Step 13.1: Push branch.**

Run:
```
git push -u origin claude/naughty-benz-091944
```

- [ ] **Step 13.2: Open PR with canonical title + body.**

PR title (required by brief): `feat(storage): Postgres ControlQueueRepo (ADR-0008)`.

PR body template:

```
## Summary

- Lands `crates/storage/src/pg/control_queue.rs::PgControlQueueRepo`, the first production-grade backing for `ControlQueueRepo` (canon §12.2). The in-memory impl remains for tests / local.
- `claim_pending` uses the canonical `WITH ... FOR UPDATE SKIP LOCKED` idiom per ADR-0008 §1; two concurrent claimers never double-claim (new regression test).
- `reclaim_stuck` runs reclaim + exhaust branches inside a single transaction; the `status = 'Processing'` predicate acts as the CAS fence under concurrent sweepers (ADR-0017). The exhaust message uses `COALESCE(encode(processed_by, 'hex'), '<unknown>')` so the lowercase-hex encoding matches `InMemoryControlQueueRepo::hex_encode_bytes` byte-for-byte — regression-locked in tests.
- Docs synced: `repos/mod.rs` status table drops the "only in-memory" qualifier; `lib.rs` repos-module docstring is truthful; `MATURITY.md` has a revision note.

## Scope

No schema changes (migration 0021 on `main` supplies `reclaim_count` + partial index). No breaking trait changes. No `crates/api` / `crates/engine` edits — wiring into a single production binary is a separate chip (`apps/server`), tracked as follow-up in ADR-0008.

## Test plan

- [x] `cargo nextest run --workspace` — in-memory control_queue tests unchanged, green.
- [x] `cargo nextest run -p nebula-storage --features postgres` with `DATABASE_URL` set — all Postgres tests pass, including: `claim_pending_stamps_...`, `reclaim_stuck_{moves_expired,leaves_fresh,leaves_non_processing,exhausts_after_max_count}`, `claim_pending_skip_locked_prevents_double_claim`, `reclaim_stuck_safe_under_concurrent_sweep`, `enqueue_roundtrip_preserves_every_command_variant`, `cleanup_deletes_old_terminal_rows_only`.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [x] `cargo clippy -p nebula-storage --all-targets --features postgres -- -D warnings` clean.
- [x] `cargo test --workspace --doc` clean.
- [x] `cargo deny check` clean.
- [x] `lefthook run pre-push` clean.

Refs: ADR-0008, ADR-0017, canon §12.2.
```

Run:
```
gh pr create --title "feat(storage): Postgres ControlQueueRepo (ADR-0008)" --body "$(cat <<'EOF'
<paste body above>
EOF
)"
```

---

## Self-review checklist (run after writing the plan, before executing)

1. **Spec coverage vs. brief's "Done when":**
   - `PgControlQueueRepo` passes the same behavioral tests as `InMemoryControlQueueRepo` → Tasks 2 / 4 / 5 / 6 + concurrency tests in Tasks 7 / 8 + variants in Task 9. ✓
   - `claim_pending` uses `FOR UPDATE SKIP LOCKED` + concurrent-claim regression → Tasks 2 + 7. ✓
   - `reclaim_stuck` safe under concurrent sweep → Task 8. ✓
   - Canonical gates clean → Task 11. ✓
   - `repos/mod.rs` status row updated; `MATURITY.md` aligned → Task 10. ✓
   - PR title `feat(storage): Postgres ControlQueueRepo (ADR-0008)` → Task 13. ✓

2. **Placeholder scan:** every code block is complete; no "TODO" / "as above" / "fill in" / "appropriate error handling" verbiage.

3. **Type consistency:**
   - `EntryTuple` defined in Task 2 and used by `claim_pending` / `reclaim_stuck` RETURNING paths. ✓
   - `SELECT_COLS` used only by `claim_pending` RETURNING; other methods use explicit column lists inline. ✓
   - `reclaim_after_seconds` defined once (Task 4) + reused (Task 6). ✓
   - `i64::from(u32)` outbound / `u32::try_from(v.max(0)).unwrap_or(u32::MAX)` inbound — consistent in both directions. ✓
   - Canonical exhaust message format matches between Task 4 (SQL) and Task 5 (assertion). ✓

4. **Execution order:** Tasks 1 → 9 are TDD-ordered (failing test first); Task 10 is pure docs; Task 11 is the gate; Task 12 is self-scan; Task 13 is PR. No task depends on a later task's output.

If you spot gaps during execution that this plan missed, update this document in place — the plan is the source of truth, not the PR body.
