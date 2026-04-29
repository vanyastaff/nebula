//! Postgres integration tests for `PgExecutionRepo` lease lifecycle
//! (ROADMAP §M2.2 — Layer 1 verification).
//!
//! Mirrors the convention used by `refresh_claim_pg_integration.rs`:
//! tests skip silently when `DATABASE_URL` is absent, and panic loudly
//! when it is set but unparsable. Each test scopes its rows to a freshly
//! generated `ExecutionId` so parallel runs do not collide on the
//! `executions.id` PRIMARY KEY.
//!
//! Run locally via:
//!   DATABASE_URL=postgres://... cargo nextest run \
//!     -p nebula-storage --features postgres \
//!     --test execution_lease_pg_integration
//!
//! These tests exercise the real Postgres SQL fence in
//! `pg_execution.rs:144-207`:
//! - acquire: `WHERE id = $1 AND (lease_holder IS NULL OR lease_expires_at < NOW())`
//! - renew:   `WHERE id = $1 AND lease_holder = $2`
//! - release: same as renew
//!
//! The InMemory parity tests in `crates/storage/src/execution_repo.rs:1182-1346`
//! cover the same surface deterministically; this file proves the
//! Postgres backend honors the contract under real `NOW()` semantics.

#![cfg(feature = "postgres")]

use std::time::Duration;

use nebula_core::id::{ExecutionId, WorkflowId};
use nebula_storage::{ExecutionRepo, PgExecutionRepo};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::OnceCell;

// Layer 1 production migrator — the simple `executions` table with
// `(id, workflow_id, version, state)` plus the M2.2 lease columns
// (`lease_holder`, `lease_expires_at`) from migration 7.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL`, apply Layer 1 migrations, or return
/// `None` to instruct the test to skip.
async fn pool() -> Option<PgPool> {
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
            MIGRATOR
                .run(&pool)
                .await
                .expect("apply Layer 1 storage migrations");
        })
        .await;
    Some(pool)
}

/// Build a fresh repo + pre-create a row so lease UPDATE statements have a target.
/// Returns the execution_id used for the row.
async fn fresh_execution(pool: &PgPool) -> (PgExecutionRepo, ExecutionId) {
    let repo = PgExecutionRepo::new(pool.clone());
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    repo.create(execution_id, workflow_id, serde_json::json!({}))
        .await
        .expect("seed executions row");
    (repo, execution_id)
}

#[tokio::test]
async fn acquire_lease_succeeds_when_holder_is_null() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    let acquired = repo
        .acquire_lease(id, "runner-a".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        acquired,
        "fresh row with NULL lease_holder must accept the first acquire"
    );

    repo.release_lease(id, "runner-a")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn acquire_lease_fails_when_held_by_other_within_ttl() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    let a = repo
        .acquire_lease(id, "runner-a".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(a, "A must acquire");

    let b = repo
        .acquire_lease(id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !b,
        "B must NOT acquire while A still holds (within TTL) — fence is `lease_holder IS NULL OR lease_expires_at < NOW()`"
    );

    repo.release_lease(id, "runner-a")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn acquire_lease_succeeds_after_explicit_release() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    repo.acquire_lease(id, "runner-a".into(), Duration::from_secs(30))
        .await
        .unwrap();
    let released = repo.release_lease(id, "runner-a").await.unwrap();
    assert!(released, "release for the holding holder must succeed");

    let b = repo
        .acquire_lease(id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(b, "B must acquire after A's explicit release");

    repo.release_lease(id, "runner-b")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn acquire_lease_succeeds_after_natural_expiry() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    // 1s is the floor enforced by `ttl_seconds()` in pg_execution.rs:65-69.
    repo.acquire_lease(id, "runner-a".into(), Duration::from_secs(1))
        .await
        .unwrap();

    // Sleep past TTL so `lease_expires_at < NOW()` becomes true.
    tokio::time::sleep(Duration::from_millis(1300)).await;

    let b = repo
        .acquire_lease(id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        b,
        "B must acquire after A's lease expired naturally — the fence's `lease_expires_at < NOW()` branch"
    );

    repo.release_lease(id, "runner-b")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn renew_lease_extends_expiry_for_holder() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    // Short TTL so renewal effect is observable.
    repo.acquire_lease(id, "runner-a".into(), Duration::from_secs(1))
        .await
        .unwrap();

    // Renew before expiry — should succeed and push expires_at forward.
    let renewed = repo
        .renew_lease(id, "runner-a", Duration::from_secs(30))
        .await
        .unwrap();
    assert!(renewed, "renew by current holder must succeed");

    // After original 1s would have elapsed, B still cannot acquire
    // because the renew bumped expires_at by 30s.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let b = repo
        .acquire_lease(id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !b,
        "B must NOT acquire after A renewed — original 1s TTL bumped to 30s"
    );

    repo.release_lease(id, "runner-a")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn renew_lease_rejects_wrong_holder() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    repo.acquire_lease(id, "runner-a".into(), Duration::from_secs(30))
        .await
        .unwrap();

    let renewed = repo
        .renew_lease(id, "runner-b", Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !renewed,
        "renew by non-holder must be rejected — fence is `lease_holder = $2`"
    );

    repo.release_lease(id, "runner-a")
        .await
        .expect("cleanup release");
}

/// ROADMAP §M2.2 / T7 — multi-runner takeover under real Postgres
/// `NOW()` semantics. Two `PgExecutionRepo` instances against the
/// same pool stand in for two engine processes; A acquires with a
/// short TTL, "crashes" (we just stop interacting with the row),
/// the wall-clock advances past TTL, and B acquires successfully.
/// The companion in-memory verification lives in
/// `crates/engine/tests/lease_takeover.rs`
/// (`engine_b_takes_over_after_engine_a_runner_dies`).
#[tokio::test]
async fn multi_runner_takeover_after_natural_expiry() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };

    // Two repo handles sharing the same pool — different processes
    // would behave the same way at the SQL layer.
    let repo_a = PgExecutionRepo::new(pool.clone());
    let repo_b = PgExecutionRepo::new(pool.clone());

    // Pre-create the row via repo_a (any handle works — the row is
    // shared state). Use a unique execution_id so parallel test runs
    // do not collide.
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    repo_a
        .create(execution_id, workflow_id, serde_json::json!({}))
        .await
        .expect("seed executions row");

    // Runner A acquires with the floor TTL (1s, see ttl_seconds()).
    let a = repo_a
        .acquire_lease(execution_id, "runner-a".into(), Duration::from_secs(1))
        .await
        .unwrap();
    assert!(a, "runner A must acquire");

    // While the lease is held, runner B is blocked.
    let b_blocked = repo_b
        .acquire_lease(execution_id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !b_blocked,
        "runner B must NOT acquire while runner A's lease is live"
    );

    // Simulate runner A's process death by simply not renewing.
    // Wall-clock advances past the 1s TTL.
    tokio::time::sleep(Duration::from_millis(1300)).await;

    // Runner B now acquires — the SQL fence's
    // `lease_expires_at < NOW()` branch matches.
    let b = repo_b
        .acquire_lease(execution_id, "runner-b".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        b,
        "runner B must acquire after runner A's lease expired naturally — \
         the cross-process takeover path"
    );

    // Runner A's stale renew is rejected — the holder column moved on.
    let a_stale_renew = repo_a
        .renew_lease(execution_id, "runner-a", Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !a_stale_renew,
        "runner A's stale renew must be rejected (lease_holder is now runner-b)"
    );

    // Runner A's stale release is also a no-op — same fence.
    let a_stale_release = repo_a
        .release_lease(execution_id, "runner-a")
        .await
        .unwrap();
    assert!(
        !a_stale_release,
        "runner A's stale release must be a no-op (it no longer holds)"
    );

    repo_b
        .release_lease(execution_id, "runner-b")
        .await
        .expect("cleanup release");
}

#[tokio::test]
async fn release_lease_no_op_for_wrong_holder() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let (repo, id) = fresh_execution(&pool).await;

    repo.acquire_lease(id, "runner-a".into(), Duration::from_secs(30))
        .await
        .unwrap();

    let released = repo.release_lease(id, "runner-b").await.unwrap();
    assert!(
        !released,
        "release by non-holder must be a no-op — fence is `lease_holder = $2`"
    );

    // Verify A still holds: a third party trying to acquire must fail.
    let c = repo
        .acquire_lease(id, "runner-c".into(), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(
        !c,
        "A's lease must still be in place after a wrong-holder release attempt"
    );

    repo.release_lease(id, "runner-a")
        .await
        .expect("cleanup release");
}
