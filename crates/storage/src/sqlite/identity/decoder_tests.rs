// A NULL in a NOT-NULL-modeled column must surface as
// `StorageError::Corrupt`, never default to an empty string or zero. Each test
// crafts an in-memory SQLite row with one deliberate defect and asserts the
// decoder reports it as corrupt.

use nebula_storage_port::StorageError;
use sqlx::SqlitePool;

use super::org::decode_org;
use super::resource::decode_resource;
use super::trigger::decode_trigger;
use super::workspace::decode_workspace;

fn is_corrupt<T>(result: &Result<T, StorageError>) -> bool {
    matches!(result, Err(StorageError::Corrupt(_)))
}

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
async fn decode_org_null_required_created_at_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
         created_at TEXT, created_by TEXT NOT NULL, plan TEXT NOT NULL, \
         settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
         billing_email TEXT, deleted_at TEXT)",
        // `created_at` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('org-1', 'my-org', 'My Org', NULL, 'user-1', 'free', \
         '{}', 0, NULL, NULL)",
        decode_org,
    )
    .await;
    assert!(
        is_corrupt(&result),
        "decode_org must return Err when the required `created_at` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn decode_workspace_null_required_slug_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, org_id TEXT NOT NULL, slug TEXT, \
         display_name TEXT NOT NULL, created_at TEXT NOT NULL, \
         created_by TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0, \
         settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
         description TEXT, deleted_at TEXT)",
        // `slug` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('ws-1', 'org-1', NULL, 'My WS', '2024-01-01T00:00:00Z', \
         'user-1', 0, '{}', 0, NULL, NULL)",
        decode_workspace,
    )
    .await;
    assert!(
        is_corrupt(&result),
        "decode_workspace must return Err when the required `slug` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn decode_resource_null_required_kind_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, slug TEXT NOT NULL, \
         display_name TEXT NOT NULL, kind TEXT, config TEXT NOT NULL DEFAULT '{}', \
         credential_bindings TEXT NOT NULL DEFAULT '{}', topology TEXT, \
         resilience_override TEXT, created_at INTEGER NOT NULL, created_by TEXT NOT NULL, \
         version INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER)",
        // `kind` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('res-1', 'ws-1', 'my-res', 'My Resource', NULL, '{}', '{}', \
         NULL, NULL, 1704067200000000, 'user-1', 0, NULL)",
        decode_resource,
    )
    .await;
    assert!(
        is_corrupt(&result),
        "decode_resource must return Err when the required `kind` column is NULL, got Ok"
    );
}

#[tokio::test]
async fn decode_trigger_null_required_workflow_id_is_err() {
    let result = with_null_row(
        "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, \
         workflow_id TEXT, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
         kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', \
         state TEXT NOT NULL, created_at INTEGER NOT NULL, created_by TEXT NOT NULL, \
         version INTEGER NOT NULL DEFAULT 0, run_as TEXT, webhook_path TEXT, deleted_at INTEGER)",
        // `workflow_id` is NOT NULL in the schema but NULL here.
        "INSERT INTO t VALUES ('trg-1', 'ws-1', NULL, 'my-trg', 'My Trigger', \
         'webhook', '{}', 'active', 1704067200000000, 'user-1', 0, NULL, NULL, NULL)",
        decode_trigger,
    )
    .await;
    assert!(
        is_corrupt(&result),
        "decode_trigger must return Err when the required `workflow_id` column is NULL, got Ok"
    );
}
