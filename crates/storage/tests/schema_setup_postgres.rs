//! PostgreSQL coordinator evidence for bounded lock acquisition/release and
//! cancellation-safe setup.

#![cfg(feature = "postgres")]

use std::{error::Error, str::FromStr, time::Duration};

use nebula_storage::postgres::init_schema;
use nebula_storage_port::StorageError;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

#[path = "support/canonical_head.rs"]
mod canonical_head;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

struct IsolatedSchema {
    admin: PgPool,
    pool: PgPool,
    schema: String,
}

impl IsolatedSchema {
    async fn connect() -> Option<Self> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
                );
                return None;
            },
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect to PostgreSQL test database");
        let schema = format!("nebula_schema_setup_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create isolated setup schema");
        let options = PgConnectOptions::from_str(&url)
            .expect("parse PostgreSQL URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect_with(options)
            .await
            .expect("connect to isolated setup schema");
        Some(Self {
            admin,
            pool,
            schema,
        })
    }

    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated setup schema");
        self.admin.close().await;
    }
}

async fn setup_lock_key(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT hashtextextended(
             'nebula:credential-schema:' || current_database() || ':' || current_schema(),
             0
         )",
    )
    .fetch_one(pool)
    .await
    .expect("derive established setup advisory key")
}

async fn wait_until_advisory_lock_is_held(pool: &PgPool, lock_key: i64) {
    let mut probe = pool
        .acquire()
        .await
        .expect("acquire dedicated advisory-lock probe session");
    for _ in 0..100 {
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *probe)
            .await
            .expect("probe setup advisory lock");
        if !acquired {
            return;
        }
        let released: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
            .bind(lock_key)
            .fetch_one(&mut *probe)
            .await
            .expect("release probe advisory lock");
        assert!(released, "probe must release the lock it acquired");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("setup did not acquire the advisory lock before the evidence deadline");
}

#[tokio::test]
async fn nonempty_unledgered_database_is_rejected_without_mutation() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    sqlx::query("CREATE TABLE unrelated (value TEXT NOT NULL)")
        .execute(&database.pool)
        .await?;
    sqlx::query("INSERT INTO unrelated (value) VALUES ('preserve-me')")
        .execute(&database.pool)
        .await?;

    let relations_before: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname || ':' || c.relkind::text
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = current_schema()
         ORDER BY c.relname, c.relkind",
    )
    .fetch_all(&database.pool)
    .await?;
    let rows_before: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&database.pool)
        .await?;

    let error = init_schema(&database.pool)
        .await
        .expect_err("nonempty unledgered database must fail closed");
    assert!(matches!(error, StorageError::Configuration(_)));

    let relations_after: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname || ':' || c.relkind::text
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = current_schema()
         ORDER BY c.relname, c.relkind",
    )
    .fetch_all(&database.pool)
    .await?;
    let rows_after: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&database.pool)
        .await?;
    assert_eq!(relations_after, relations_before);
    assert_eq!(rows_after, rows_before);

    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn abort_while_migration_blocked_releases_setup_lock_and_retry_succeeds() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    MIGRATOR.run_to(40, &database.pool).await?;

    let mut blocker = database.pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *blocker).await?;
    sqlx::query("LOCK TABLE port_executions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;

    let lock_key = setup_lock_key(&database.pool).await;
    let starter_pool = database.pool.clone();
    let starter = tokio::spawn(async move { init_schema(&starter_pool).await });
    wait_until_advisory_lock_is_held(&database.pool, lock_key).await;

    starter.abort();
    let cancellation = starter.await.expect_err("starter must be cancelled");
    assert!(cancellation.is_cancelled());

    sqlx::query("ROLLBACK").execute(&mut *blocker).await?;
    drop(blocker);

    tokio::time::timeout(Duration::from_secs(10), init_schema(&database.pool))
        .await
        .expect("replacement setup must complete within the evidence deadline")
        .expect("replacement setup must acquire the released setup lock");
    let head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&database.pool)
        .await?;
    assert_eq!(head, canonical_head::of(&MIGRATOR));

    database.cleanup().await;
    Ok(())
}
