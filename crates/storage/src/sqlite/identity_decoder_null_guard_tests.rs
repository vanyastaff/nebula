// FIX 1 regression: a NULL in a NOT-NULL-modeled column must surface as
// `Err(StorageError::…)`, not silently default to an empty string / zero.
// Each test crafts an in-memory SQLite row with a deliberate NULL in one
// required column and asserts the decoder returns Err.

use nebula_storage_port::StorageError;
use sqlx::SqlitePool;

use super::{
    audit_from_row, blob_from_row, org_from_row, quota_from_row, resource_from_row,
    trigger_from_row, user_from_row, workspace_from_row,
};

// Open a temporary in-memory SQLite pool with the given DDL applied, then
// fetch the sole row and call `decoder` on it. Returns the decoder's Result.
//
// Both `ddl` and `insert` are `'static` string literals sourced from test
// constants — the `sqlx::query` API requires `'static`.
async fn with_null_row<T>(
    ddl: &'static str,
    insert: &'static str,
    decoder: impl Fn(&sqlx::sqlite::SqliteRow) -> Result<T, StorageError>,
) -> Result<T, StorageError> {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query(ddl).execute(&pool).await.unwrap();
    sqlx::query(insert).execute(&pool).await.unwrap();
    let row = sqlx::query("SELECT * FROM t")
        .fetch_one(&pool)
        .await
        .unwrap();
    decoder(&row)
}

#[tokio::test]
async fn user_from_row_null_required_id_is_err() {
    // The `id` column is NOT NULL in the schema; a NULL decode must be Err.
    let result = with_null_row(
        "CREATE TABLE t (id TEXT, email TEXT NOT NULL, display_name TEXT NOT NULL, \
         created_at TEXT NOT NULL, failed_login_count INTEGER NOT NULL DEFAULT 0, \
         mfa_enabled INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 0, \
         email_verified_at TEXT, avatar_url TEXT, password_hash TEXT, \
         last_login_at TEXT, locked_until TEXT, mfa_secret BLOB, deleted_at TEXT)",
        // Insert a row where `id` is explicitly NULL.
        "INSERT INTO t VALUES (NULL, 'a@b.com', 'Alice', '2024-01-01T00:00:00Z', \
         0, 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
        user_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "user_from_row must return Err when the required `id` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn org_from_row_null_required_created_at_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
         created_at TEXT, created_by TEXT NOT NULL, plan TEXT NOT NULL, \
         settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
         billing_email TEXT, deleted_at TEXT)",
        // `created_at` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('org-1', 'my-org', 'My Org', NULL, 'user-1', 'free', \
         '{}', 0, NULL, NULL)",
        org_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "org_from_row must return Err when the required `created_at` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn workspace_from_row_null_required_slug_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, org_id TEXT NOT NULL, slug TEXT, \
         display_name TEXT NOT NULL, created_at TEXT NOT NULL, \
         created_by TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0, \
         settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
         description TEXT, deleted_at TEXT)",
        // `slug` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('ws-1', 'org-1', NULL, 'My WS', '2024-01-01T00:00:00Z', \
         'user-1', 0, '{}', 0, NULL, NULL)",
        workspace_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "workspace_from_row must return Err when the required `slug` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn quota_from_row_null_required_plan_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (org_id TEXT NOT NULL, plan TEXT, \
         concurrent_executions_limit INTEGER NOT NULL DEFAULT 0, \
         concurrent_executions INTEGER NOT NULL DEFAULT 0, \
         executions_this_month INTEGER NOT NULL DEFAULT 0, \
         month_reset_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
         executions_per_month_limit INTEGER, active_workflows_limit INTEGER)",
        // `plan` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('org-1', NULL, 10, 0, 0, '2024-01-01T00:00:00Z', \
         '2024-01-01T00:00:00Z', NULL, NULL)",
        quota_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "quota_from_row must return Err when the required `plan` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn audit_from_row_null_required_action_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, org_id TEXT NOT NULL, \
         actor_kind TEXT NOT NULL, action TEXT, emitted_at TEXT NOT NULL, \
         workspace_id TEXT, actor_id TEXT, target_kind TEXT, target_id TEXT, \
         details TEXT, ip_address TEXT, user_agent TEXT)",
        // `action` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('evt-1', 'org-1', 'user', NULL, '2024-01-01T00:00:00Z', \
         NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
        audit_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "audit_from_row must return Err when the required `action` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn resource_from_row_null_required_kind_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, slug TEXT NOT NULL, \
         display_name TEXT NOT NULL, kind TEXT, config TEXT NOT NULL DEFAULT '{}', \
         created_at TEXT NOT NULL, created_by TEXT NOT NULL, \
         version INTEGER NOT NULL DEFAULT 0, deleted_at TEXT)",
        // `kind` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('res-1', 'ws-1', 'my-res', 'My Resource', NULL, '{}', \
         '2024-01-01T00:00:00Z', 'user-1', 0, NULL)",
        resource_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "resource_from_row must return Err when the required `kind` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn trigger_from_row_null_required_workflow_id_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, \
         workflow_id TEXT, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
         kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', \
         state TEXT NOT NULL, created_at TEXT NOT NULL, created_by TEXT NOT NULL, \
         version INTEGER NOT NULL DEFAULT 0, run_as TEXT, webhook_path TEXT, deleted_at TEXT)",
        // `workflow_id` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('trg-1', 'ws-1', NULL, 'my-trg', 'My Trigger', \
         'webhook', '{}', 'active', '2024-01-01T00:00:00Z', 'user-1', 0, NULL, NULL, NULL)",
        trigger_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "trigger_from_row must return Err when the required `workflow_id` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn blob_from_row_null_required_storage_mode_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, \
         kind TEXT NOT NULL, size_bytes INTEGER NOT NULL DEFAULT 0, \
         storage_mode TEXT, created_at TEXT NOT NULL, \
         execution_id TEXT, content_type TEXT, checksum BLOB, \
         data BLOB, external_ref TEXT, metadata TEXT, expires_at TEXT)",
        // `storage_mode` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('blob-1', 'ws-1', 'output', 42, NULL, \
         '2024-01-01T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
        blob_from_row,
    )
    .await;
    assert!(
        result.is_err(),
        "blob_from_row must return Err when the required `storage_mode` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn quota_from_row_i64_outside_i32_range_is_err() {
    // concurrent_executions_limit is stored as i64 in SQLite but mapped to
    // i32 in QuotaRow. A value beyond i32::MAX must produce Err, not a
    // silently wrapped integer.
    let overflow = (i32::MAX as i64) + 1; // 2_147_483_648 — one past i32::MAX
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE t (org_id TEXT NOT NULL, plan TEXT NOT NULL, \
         concurrent_executions_limit INTEGER NOT NULL, \
         concurrent_executions INTEGER NOT NULL DEFAULT 0, \
         executions_this_month INTEGER NOT NULL DEFAULT 0, \
         month_reset_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
         executions_per_month_limit INTEGER, active_workflows_limit INTEGER)",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Bind the overflow value as a parameter so the query string stays static.
    sqlx::query(
        "INSERT INTO t VALUES ('org-1', 'pro', ?, 0, 0, \
         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', NULL, NULL)",
    )
    .bind(overflow)
    .execute(&pool)
    .await
    .unwrap();
    let row = sqlx::query("SELECT * FROM t")
        .fetch_one(&pool)
        .await
        .unwrap();
    let result = quota_from_row(&row);
    assert!(
        result.is_err(),
        "quota_from_row must return Err when concurrent_executions_limit exceeds i32::MAX, got Ok"
    );
}

#[tokio::test]
async fn user_from_row_failed_login_count_outside_i32_range_is_err() {
    // failed_login_count is stored as i64 in SQLite but mapped to i32 in
    // UserRow. A value beyond i32::MAX must produce Err, not wrap.
    let overflow = (i32::MAX as i64) + 1;
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE t (id TEXT NOT NULL, email TEXT NOT NULL, \
         display_name TEXT NOT NULL, created_at TEXT NOT NULL, \
         failed_login_count INTEGER NOT NULL, \
         mfa_enabled INTEGER NOT NULL DEFAULT 0, \
         version INTEGER NOT NULL DEFAULT 0, \
         email_verified_at TEXT, avatar_url TEXT, password_hash TEXT, \
         last_login_at TEXT, locked_until TEXT, mfa_secret BLOB, deleted_at TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO t VALUES ('u-1', 'a@b.com', 'Alice', '2024-01-01T00:00:00Z', \
         ?, 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
    )
    .bind(overflow)
    .execute(&pool)
    .await
    .unwrap();
    let row = sqlx::query("SELECT * FROM t")
        .fetch_one(&pool)
        .await
        .unwrap();
    let result = user_from_row(&row);
    assert!(
        result.is_err(),
        "user_from_row must return Err when failed_login_count exceeds i32::MAX, got Ok"
    );
}
