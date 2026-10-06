//! SQLite coordinator evidence for canonical, serialized migration setup.

#![cfg(feature = "sqlite")]

use std::str::FromStr;

use nebula_storage::sqlite::init_schema;
use nebula_storage_port::StorageError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[path = "support/canonical_head.rs"]
mod canonical_head;

/// The catalog `init_schema` installs, embedded here to read its head.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

async fn migration_head(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
        .expect("canonical migration ledger must be readable")
}

async fn foreign_keys_enabled(connection: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>) -> bool {
    sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
        .fetch_one(&mut **connection)
        .await
        .expect("foreign-key state must be readable")
        == 1
}

#[tokio::test]
async fn max_one_memory_pool_reaches_canonical_head_with_foreign_keys() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open isolated SQLite memory pool");

    init_schema(&pool)
        .await
        .expect("canonical setup must accept a fresh max-one memory database");
    assert_eq!(migration_head(&pool).await, canonical_head::of(&MIGRATOR));
    let mut connection = pool.acquire().await.expect("acquire admitted connection");
    assert!(foreign_keys_enabled(&mut connection).await);
}

#[tokio::test]
async fn named_shared_memory_pool_proves_second_connection_visibility() {
    let database_name = format!("nebula-setup-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database_name}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("parse named shared-memory URL")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("open named shared-memory pool");

    init_schema(&pool)
        .await
        .expect("setup must prove shared-cache visibility before returning");

    let mut first = pool.acquire().await.expect("hold first connection");
    let mut second = pool
        .acquire()
        .await
        .expect("acquire a distinct second connection while first is held");
    let first_head: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut *first)
            .await
            .expect("first connection sees canonical ledger");
    let second_head: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut *second)
            .await
            .expect("second connection sees canonical ledger");
    assert_eq!(
        (first_head, second_head),
        (canonical_head::of(&MIGRATOR), canonical_head::of(&MIGRATOR))
    );
    assert!(foreign_keys_enabled(&mut first).await);
    assert!(foreign_keys_enabled(&mut second).await);
}

#[tokio::test]
async fn concurrent_setup_on_max_two_shared_pool_does_not_starve_visibility_probe() {
    let database_name = format!("nebula-setup-race-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database_name}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("parse named shared-memory URL")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("open max-two shared-memory pool");

    let first = init_schema(&pool);
    let second = init_schema(&pool);
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(first, second)
    })
    .await
    .expect("concurrent setup must not deadlock on pool-slot starvation");
    first.expect("first setup must succeed");
    second.expect("second setup must succeed");
    assert_eq!(migration_head(&pool).await, canonical_head::of(&MIGRATOR));
}

#[tokio::test]
async fn nonempty_unledgered_database_is_rejected_without_mutation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open rejection fixture");
    sqlx::query("CREATE TABLE unrelated (value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create unrelated relation");
    sqlx::query("INSERT INTO unrelated (value) VALUES ('preserve-me')")
        .execute(&pool)
        .await
        .expect("seed unrelated row");

    let schema_before: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, sql
         FROM sqlite_schema
         WHERE type = 'table'
         ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("snapshot schema before rejection");
    let rows_before: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&pool)
        .await
        .expect("snapshot rows before rejection");

    let error = init_schema(&pool)
        .await
        .expect_err("nonempty unledgered database must fail closed");
    assert!(matches!(error, StorageError::Configuration(_)));

    let schema_after: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, sql
         FROM sqlite_schema
         WHERE type = 'table'
         ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("snapshot schema after rejection");
    let rows_after: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&pool)
        .await
        .expect("snapshot rows after rejection");
    assert_eq!(schema_after, schema_before);
    assert_eq!(rows_after, rows_before);
}
