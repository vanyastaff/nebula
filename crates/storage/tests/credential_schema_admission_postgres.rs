//! PostgreSQL ready-store admission and migration boundary.

#![cfg(feature = "postgres")]

use std::{
    error::Error,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nebula_core::CredentialId;
use nebula_storage::credential::{
    CredentialSchemaAdmissionReason, CredentialStoreStartupError, PgCredentialPersistence,
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

struct IsolatedSchema {
    admin: PgPool,
    options: PgConnectOptions,
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
            .expect("connect to DATABASE_URL");
        let schema = unique_schema_name();
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create isolated admission schema");
        let options = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        Some(Self {
            admin,
            options,
            schema,
        })
    }

    async fn raw_pool(&self) -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_with(self.options.clone())
            .await
            .expect("connect to isolated schema")
    }

    async fn cleanup(self) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated admission schema");
        self.admin.close().await;
    }
}

fn unique_schema_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("nebula_k2_admission_{}_{nanos}", std::process::id())
}

#[derive(Debug, PartialEq, Eq)]
struct LogicalSnapshot {
    relations: Vec<String>,
    ledger: Vec<String>,
    credentials: Vec<String>,
}

async fn logical_snapshot(pool: &PgPool) -> TestResult<LogicalSnapshot> {
    let relations: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname || ':' || c.relkind::text
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = current_schema()
         ORDER BY c.relname, c.relkind",
    )
    .fetch_all(pool)
    .await?;
    let ledger_exists: bool = sqlx::query_scalar(
        "SELECT to_regclass(current_schema() || '._sqlx_migrations') IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    let ledger = if ledger_exists {
        sqlx::query_scalar(
            "SELECT concat_ws(
                 '|', version::text, description, success::text,
                 encode(checksum, 'hex'), execution_time::text
             )
             FROM _sqlx_migrations
             ORDER BY version, installed_on",
        )
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    let credentials_exist: bool =
        sqlx::query_scalar("SELECT to_regclass(current_schema() || '.credentials') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    let credentials = if credentials_exist {
        sqlx::query_scalar("SELECT row_to_json(credentials)::text FROM credentials ORDER BY id")
            .fetch_all(pool)
            .await?
    } else {
        Vec::new()
    };
    Ok(LogicalSnapshot {
        relations,
        ledger,
        credentials,
    })
}

async fn reject_logically_unchanged(
    database: &IsolatedSchema,
) -> TestResult<CredentialStoreStartupError> {
    let pool = database.raw_pool().await;
    let before = logical_snapshot(&pool).await?;
    pool.close().await;
    let error = PgCredentialPersistence::connect_with(database.options.clone())
        .await
        .expect_err("unsupported PostgreSQL fixture must fail closed");
    let pool = database.raw_pool().await;
    let after = logical_snapshot(&pool).await?;
    pool.close().await;
    assert_eq!(
        after, before,
        "schema admission rejection must be read-only"
    );
    Ok(error)
}

async fn insert_legacy_metadata(pool: &PgPool, metadata: &str) -> TestResult<()> {
    insert_legacy_record(
        pool,
        &CredentialId::new().to_string(),
        Some("owner-rejection"),
        None,
        0,
        1,
        metadata,
    )
    .await
}

async fn insert_legacy_record(
    pool: &PgPool,
    id: &str,
    owner: Option<&str>,
    name: Option<&str>,
    state_version: i64,
    version: i64,
    metadata: &str,
) -> TestResult<()> {
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now(), NULL, FALSE, $9)",
    )
    .bind(id)
    .bind(name)
    .bind(owner)
    .bind("provider.rejection")
    .bind("ready")
    .bind(state_version)
    .bind(Vec::<u8>::new())
    .bind(version)
    .bind(metadata)
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
async fn fresh_and_canonical_0038_schemas_are_admitted() -> TestResult<()> {
    let Some(fresh) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    let ready = PgCredentialPersistence::connect_with(fresh.options.clone()).await?;
    drop(ready);
    let pool = fresh.raw_pool().await;
    let head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await?;
    assert_eq!(head, 40);
    pool.close().await;
    fresh.cleanup().await;

    let Some(upgrade) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    let pool = upgrade.raw_pool().await;
    MIGRATOR.run_to(38, &pool).await?;
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES ($1, NULL, $2, $3, $4, 0, $5, 1, now(), now(), NULL, FALSE, '{}')",
    )
    .bind(CredentialId::new().to_string())
    .bind("owner-upgrade")
    .bind("provider.upgrade")
    .bind("ready")
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await?;
    pool.close().await;

    let ready = PgCredentialPersistence::connect_with(upgrade.options.clone()).await?;
    drop(ready);
    let pool = upgrade.raw_pool().await;
    let state: String =
        sqlx::query_scalar("SELECT record_state FROM credentials WHERE owner_id = 'owner-upgrade'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, "live");
    pool.close().await;
    upgrade.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ownerless_legacy_schema_is_rejected_without_migration() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    let pool = database.raw_pool().await;
    MIGRATOR.run_to(38, &pool).await?;
    let id = CredentialId::new().to_string();
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES ($1, NULL, NULL, $2, $3, 0, $4, 1, now(), now(), NULL, FALSE, '{}')",
    )
    .bind(&id)
    .bind("provider.ownerless")
    .bind("ready")
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await?;
    pool.close().await;

    let error = PgCredentialPersistence::connect_with(database.options.clone())
        .await
        .expect_err("ownerless legacy row must fail before migration");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::OwnerlessCredential
    ));

    let pool = database.raw_pool().await;
    let preserved_owner: Option<String> =
        sqlx::query_scalar("SELECT owner_id FROM credentials WHERE id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    let migration_0039_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 39")
            .fetch_one(&pool)
            .await?;
    assert_eq!(preserved_owner, None);
    assert_eq!(migration_0039_rows, 0);
    pool.close().await;
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn release_rejection_matrix_preserves_postgres_logical_state() -> TestResult<()> {
    for case in [
        "empty-ledger",
        "below-floor",
        "invalid-ledger",
        "missing-credentials",
        "checksum",
        "description",
        "gap",
        "future",
        "dirty",
        "invalid-id",
        "invalid-state-version",
        "invalid-version",
        "live-version-exhausted",
        "malformed-json",
        "metadata-not-object",
        "recursive-duplicate-key",
        "invalid-display",
        "display-name-mismatch",
        "orphan-name",
        "duplicate-projected-name",
    ] {
        let Some(database) = IsolatedSchema::connect().await else {
            return Ok(());
        };
        let pool = database.raw_pool().await;
        if case == "below-floor" {
            MIGRATOR.run_to(29, &pool).await?;
        } else {
            MIGRATOR.run_to(38, &pool).await?;
            match case {
                "empty-ledger" => {
                    sqlx::query("DELETE FROM _sqlx_migrations")
                        .execute(&pool)
                        .await?;
                },
                "invalid-ledger" => {
                    sqlx::query(
                        "ALTER TABLE _sqlx_migrations
                         ALTER COLUMN execution_time TYPE NUMERIC",
                    )
                    .execute(&pool)
                    .await?;
                },
                "missing-credentials" => {
                    sqlx::query("DROP TABLE credentials").execute(&pool).await?;
                },
                "checksum" => {
                    sqlx::query(
                        "UPDATE _sqlx_migrations
                         SET checksum = decode('00', 'hex')
                         WHERE version = 38",
                    )
                    .execute(&pool)
                    .await?;
                },
                "description" => {
                    sqlx::query(
                        "UPDATE _sqlx_migrations
                         SET description = 'drifted'
                         WHERE version = 38",
                    )
                    .execute(&pool)
                    .await?;
                },
                "gap" => {
                    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 37")
                        .execute(&pool)
                        .await?;
                },
                "future" => {
                    sqlx::query(
                        "INSERT INTO _sqlx_migrations (
                             version, description, success, checksum, execution_time
                         ) VALUES (999, 'future', TRUE, decode('00', 'hex'), 0)",
                    )
                    .execute(&pool)
                    .await?;
                },
                "dirty" => {
                    sqlx::query("UPDATE _sqlx_migrations SET success = FALSE WHERE version = 38")
                        .execute(&pool)
                        .await?;
                },
                "invalid-id" => {
                    insert_legacy_record(
                        &pool,
                        "not-a-credential-id",
                        Some("owner-rejection"),
                        None,
                        0,
                        1,
                        "{}",
                    )
                    .await?;
                },
                "invalid-state-version" => {
                    insert_legacy_record(
                        &pool,
                        &CredentialId::new().to_string(),
                        Some("owner-rejection"),
                        None,
                        4_294_967_296,
                        1,
                        "{}",
                    )
                    .await?;
                },
                "invalid-version" => {
                    insert_legacy_record(
                        &pool,
                        &CredentialId::new().to_string(),
                        Some("owner-rejection"),
                        None,
                        0,
                        0,
                        "{}",
                    )
                    .await?;
                },
                "live-version-exhausted" => {
                    insert_legacy_record(
                        &pool,
                        &CredentialId::new().to_string(),
                        Some("owner-rejection"),
                        None,
                        0,
                        i64::MAX,
                        "{}",
                    )
                    .await?;
                },
                "malformed-json" => insert_legacy_metadata(&pool, "{not-json").await?,
                "metadata-not-object" => insert_legacy_metadata(&pool, "[]").await?,
                "recursive-duplicate-key" => {
                    insert_legacy_metadata(&pool, r#"{"nested":{"key":1,"key":2}}"#).await?;
                },
                "invalid-display" => {
                    insert_legacy_metadata(&pool, r#"{"display":{"description":7}}"#).await?;
                },
                "display-name-mismatch" => {
                    insert_legacy_record(
                        &pool,
                        &CredentialId::new().to_string(),
                        Some("owner-rejection"),
                        Some("Physical"),
                        0,
                        1,
                        r#"{"display":{"display_name":"Projected"}}"#,
                    )
                    .await?;
                },
                "orphan-name" => {
                    insert_legacy_record(
                        &pool,
                        &CredentialId::new().to_string(),
                        Some("owner-rejection"),
                        Some("Physical"),
                        0,
                        1,
                        "{}",
                    )
                    .await?;
                },
                "duplicate-projected-name" => {
                    for _ in 0..2 {
                        insert_legacy_record(
                            &pool,
                            &CredentialId::new().to_string(),
                            Some("owner-rejection"),
                            None,
                            0,
                            1,
                            r#"{"display":{"display_name":"Shared"}}"#,
                        )
                        .await?;
                    }
                },
                _ => unreachable!("closed release-rejection fixture set"),
            }
        }
        pool.close().await;

        let error = reject_logically_unchanged(&database).await?;
        match case {
            "empty-ledger" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::EmptyLedger
            )),
            "below-floor" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::BelowSupportedFloor { .. }
                    )
            )),
            "invalid-ledger" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::InvalidMigrationLedger
            )),
            "missing-credentials" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::MissingCredentialsRelation
            )),
            "checksum" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::ChecksumMismatch { migration: 38 }
                    )
            )),
            "description" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::DescriptionMismatch { migration: 38 }
                    )
            )),
            "gap" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::NonCanonicalOrder { .. }
                            | CredentialSchemaAdmissionReason::UnknownMigration { .. }
                    )
            )),
            "future" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::UnknownMigration { migration: 999 }
                    )
            )),
            "dirty" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::FailedMigration { migration: 38 }
                    )
            )),
            "invalid-id" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::InvalidCredentialId
            )),
            "invalid-state-version" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::InvalidStateVersion
            )),
            "invalid-version" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::InvalidCredentialVersion
            )),
            "live-version-exhausted" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::LiveVersionExhausted
            )),
            "malformed-json" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::MalformedMetadata
            )),
            "metadata-not-object" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::MetadataNotObject
            )),
            "recursive-duplicate-key" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::DuplicateMetadataKey
            )),
            "invalid-display" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::InvalidDisplay
            )),
            "display-name-mismatch" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::DisplayNameMismatch
            )),
            "orphan-name" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason() == &CredentialSchemaAdmissionReason::OrphanPhysicalName
            )),
            "duplicate-projected-name" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if unsupported.reason()
                        == &CredentialSchemaAdmissionReason::DuplicateProjectedName
            )),
            _ => unreachable!("closed release-rejection assertion set"),
        }
        database.cleanup().await;
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_fresh_starters_serialize_the_schema_transition() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    let lock_pool = database.raw_pool().await;
    let mut lock_connection = lock_pool.acquire().await?;
    let lock_key: i64 = sqlx::query_scalar(
        "SELECT hashtextextended(
             'nebula:credential-schema:' || current_database() || ':' || current_schema(),
             0
         )",
    )
    .fetch_one(&mut *lock_connection)
    .await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock_key)
        .execute(&mut *lock_connection)
        .await?;

    let first_options = database.options.clone();
    let second_options = database.options.clone();
    let first = PgCredentialPersistence::connect_with(first_options);
    let second = PgCredentialPersistence::connect_with(second_options);
    tokio::pin!(first, second);
    tokio::select! {
        result = &mut first => {
            panic!("first starter bypassed the held schema lock: {result:?}");
        },
        result = &mut second => {
            panic!("second starter bypassed the held schema lock: {result:?}");
        },
        () = tokio::time::sleep(Duration::from_millis(100)) => {},
    }

    let unlocked: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(lock_key)
        .fetch_one(&mut *lock_connection)
        .await?;
    assert!(unlocked);
    drop(lock_connection);
    lock_pool.close().await;

    let (first, second) = tokio::join!(first, second);
    drop(first?);
    drop(second?);

    let pool = database.raw_pool().await;
    let successful: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await?;
    let head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await?;
    assert_eq!(successful, 40);
    assert_eq!(head, 40);
    pool.close().await;
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn current_schema_rejects_same_named_dummy_constraint() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    drop(PgCredentialPersistence::connect_with(database.options.clone()).await?);
    let pool = database.raw_pool().await;
    sqlx::query("ALTER TABLE credentials DROP CONSTRAINT credentials_version_range")
        .execute(&pool)
        .await?;
    sqlx::query(
        "ALTER TABLE credentials
         ADD CONSTRAINT credentials_version_range CHECK (TRUE)",
    )
    .execute(&pool)
    .await?;
    pool.close().await;

    let error = PgCredentialPersistence::connect_with(database.options.clone())
        .await
        .expect_err("a same-named dummy constraint must not pass admission");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidCredentialsRelation
    ));
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn current_schema_rejects_missing_claim_incident_uniqueness() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    drop(PgCredentialPersistence::connect_with(database.options.clone()).await?);
    let pool = database.raw_pool().await;
    sqlx::query("DROP INDEX idx_credential_sentinel_events_claim_id")
        .execute(&pool)
        .await?;
    pool.close().await;

    let error = PgCredentialPersistence::connect_with(database.options.clone())
        .await
        .expect_err("a current ledger cannot substitute for incident uniqueness");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidSentinelEventsRelation
    ));

    let pool = database.raw_pool().await;
    let index_still_absent: bool =
        sqlx::query_scalar("SELECT to_regclass('idx_credential_sentinel_events_claim_id') IS NULL")
            .fetch_one(&pool)
            .await?;
    assert!(index_still_absent, "admission rejection must be read-only");
    pool.close().await;
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn current_schema_rejects_column_and_index_drift() -> TestResult<()> {
    let Some(column_drift) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    drop(PgCredentialPersistence::connect_with(column_drift.options.clone()).await?);
    let pool = column_drift.raw_pool().await;
    sqlx::query("ALTER TABLE credentials ALTER COLUMN state_version TYPE NUMERIC")
        .execute(&pool)
        .await?;
    pool.close().await;
    let error = PgCredentialPersistence::connect_with(column_drift.options.clone())
        .await
        .expect_err("a drifted column type must not pass admission");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidCredentialsRelation
    ));
    column_drift.cleanup().await;

    let Some(index_drift) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    drop(PgCredentialPersistence::connect_with(index_drift.options.clone()).await?);
    let pool = index_drift.raw_pool().await;
    sqlx::query("DROP INDEX idx_credentials_expiring")
        .execute(&pool)
        .await?;
    sqlx::query(
        "CREATE INDEX idx_credentials_expiring
         ON credentials(expires_at)
         WHERE expires_at IS NULL",
    )
    .execute(&pool)
    .await?;
    pool.close().await;
    let error = PgCredentialPersistence::connect_with(index_drift.options.clone())
        .await
        .expect_err("a same-named index with the wrong predicate must fail");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidCredentialsRelation
    ));
    index_drift.cleanup().await;

    let Some(ledger_drift) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    drop(PgCredentialPersistence::connect_with(ledger_drift.options.clone()).await?);
    let pool = ledger_drift.raw_pool().await;
    sqlx::query("ALTER TABLE _sqlx_migrations ALTER COLUMN execution_time TYPE NUMERIC")
        .execute(&pool)
        .await?;
    pool.close().await;
    let error = PgCredentialPersistence::connect_with(ledger_drift.options.clone())
        .await
        .expect_err("a non-SQLx ledger shape must fail before migration");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::InvalidMigrationLedger
    ));
    ledger_drift.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn unledgered_sequence_is_not_misclassified_as_fresh() -> TestResult<()> {
    let Some(database) = IsolatedSchema::connect().await else {
        return Ok(());
    };
    let pool = database.raw_pool().await;
    sqlx::query("CREATE SEQUENCE existing_user_sequence")
        .execute(&pool)
        .await?;
    pool.close().await;

    let error = PgCredentialPersistence::connect_with(database.options.clone())
        .await
        .expect_err("a schema containing a sequence is not a fresh empty database");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::UnledgeredDatabase
    ));
    let pool = database.raw_pool().await;
    let ledger_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert!(!ledger_exists, "rejection must be read-only");
    pool.close().await;
    database.cleanup().await;
    Ok(())
}
