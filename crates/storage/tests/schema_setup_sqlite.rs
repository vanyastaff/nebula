//! SQLite coordinator evidence for canonical, serialized migration setup.

#![cfg(feature = "sqlite")]

use std::str::FromStr;

use nebula_storage::sqlite::init_schema;
use nebula_storage_port::StorageError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[path = "support/canonical_head.rs"]
mod canonical_head;

/// The catalog `init_schema` installs, embedded here to read its head and to
/// build adoption fixtures that really are what they claim to be.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// The version an operator adopts through in these tests. Matches
/// `GENERAL_CATALOG_SUPPORTED_FLOOR`: the lowest catalog ordinary setup
/// admits without aggregate-owner validation.
const ADOPTION_BASELINE: i64 = 40;

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

async fn insert_semantically_invalid_credential(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, material_epoch, created_at, updated_at, expires_at,
             reauth_required, metadata, record_state, tombstoned_at,
             refresh_retry_mode, refresh_retry_not_before, refresh_retry_phase,
             refresh_retry_kind, refresh_retry_diagnostic_code
         ) VALUES (
             'not-a-credential-id', NULL, 'owner-shared-memory',
             'provider.shared-memory', 'ready', 0, zeroblob(0), 1, 1,
             1700000000000, 1700000000001, NULL, 0, '{}', 'live', NULL,
             NULL, NULL, NULL, NULL, NULL
         )",
    )
    .execute(pool)
    .await
    .expect("physical schema must permit the semantic-corruption fixture");
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
async fn named_shared_memory_catalog_setup_does_not_read_credential_rows() {
    let database_name = format!("nebula-setup-catalog-only-{}", uuid::Uuid::new_v4());
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
        .expect("setup must install the canonical catalog");
    insert_semantically_invalid_credential(&pool).await;
    init_schema(&pool)
        .await
        .expect("general setup must inspect catalog facts on every connection");
    let preserved: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM credentials WHERE id = 'not-a-credential-id'")
            .fetch_one(&pool)
            .await
            .expect("semantic-corruption fixture must remain readable");
    assert_eq!(preserved, 1);
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

/// Red→green: an unledgered database has a supported way back to service.
///
/// Every database provisioned by the previous idempotent `init_schema` carries
/// the `port_*` schema with no `_sqlx_migrations` ledger, so setup now fails
/// closed on it and the owning process cannot start. Without an adoption path
/// that is a permanent outage for any deployment holding real data. Adoption
/// stamps the ledger the operator asserts is already satisfied, after which
/// ordinary setup admits the database and brings it to head.
#[tokio::test]
async fn adopting_an_unledgered_database_lets_setup_admit_it() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open adoption fixture");

    // Stand in for a database the old init_schema provisioned: real relations,
    // no ledger.
    //
    // Build the relations by actually migrating to the adoption baseline and
    // then discarding the ledger, rather than by creating a token table. That
    // is what adoption *asserts* about a database it stamps, and a fixture
    // that only pretends silently invalidates everything after the stamp: a
    // later `ALTER TABLE` migration would fail against tables the fixture
    // never created, and the failure would look like a migration defect.
    MIGRATOR
        .run_to(ADOPTION_BASELINE, &pool)
        .await
        .expect("build the schema the operator will assert this database has");
    sqlx::query("DROP TABLE _sqlx_migrations")
        .execute(&pool)
        .await
        .expect("discard the ledger to model a pre-ledger database");

    let rejection = init_schema(&pool)
        .await
        .expect_err("an unledgered database must fail closed before adoption");
    assert!(matches!(rejection, StorageError::Configuration(_)));

    let outcome = nebula_storage::sqlite::adopt_ledger(&pool, ADOPTION_BASELINE)
        .await
        .expect("adoption must succeed for a pre-ledger database");
    assert_eq!(
        outcome,
        nebula_storage::LedgerAdoptionOutcome::Adopted {
            through_version: ADOPTION_BASELINE
        }
    );

    init_schema(&pool)
        .await
        .expect("an adopted database must be admitted and migrated to head");
    assert_eq!(migration_head(&pool).await, canonical_head::of(&MIGRATOR));

    // Re-adoption is refused rather than duplicating ledger rows.
    assert_eq!(
        nebula_storage::sqlite::adopt_ledger(&pool, ADOPTION_BASELINE)
            .await
            .expect("re-adoption must not error"),
        nebula_storage::LedgerAdoptionOutcome::AlreadyLedgered
    );
}

/// Adoption never invents a ledger for a database that has no schema.
#[tokio::test]
async fn adoption_refuses_an_empty_database() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open empty fixture");

    assert_eq!(
        nebula_storage::sqlite::adopt_ledger(&pool, ADOPTION_BASELINE)
            .await
            .expect("adoption must not error on an empty database"),
        nebula_storage::LedgerAdoptionOutcome::FreshDatabase,
        "an empty database is ordinary setup's job, not adoption's"
    );
}
