//! PostgreSQL acceptance coverage for the shared credential lifecycle migration.
//!
//! The test owns a unique schema so it can exercise the real `0038 -> 0039`
//! transition without disturbing a developer database or racing other storage
//! integration tests. As with the rest of the PostgreSQL suite, an absent
//! `DATABASE_URL` skips cleanly while a configured but unusable database fails.

#![cfg(feature = "postgres")]

use std::{
    error::Error,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct CredentialRow {
    id: String,
    name: Option<String>,
    owner_id: String,
    credential_key: String,
    state_kind: String,
    state_version: i64,
    data: Vec<u8>,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    reauth_required: bool,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct ExpectedRows {
    credentials: Vec<CredentialRow>,
    user_id: Vec<u8>,
    user_email: String,
    user_display_name: String,
    user_created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct MigrationEvidence {
    expected: ExpectedRows,
    first_credentials: Vec<CredentialRow>,
    second_credentials: Vec<CredentialRow>,
    first_ledger_count: i64,
    second_ledger_count: i64,
    migration_0039: (i64, String, bool),
    owner_id_column: (String, Option<String>),
    record_state_column: (String, Option<String>),
    indexes: Vec<(String, String)>,
    sentinel_claim_id_column: (String, String, Option<String>),
    sentinel_indexes: Vec<(String, String)>,
    historical_sentinel_claim_id: Option<Uuid>,
    duplicate_incident_rejected: bool,
    unrelated_user: (Vec<u8>, String, String, DateTime<Utc>),
    owner_null_rejected: bool,
    live_max_version_rejected: bool,
    tombstone_payload_rejected: bool,
    tombstone_max_version_accepted: bool,
    staging_relation: Option<String>,
}

struct IsolatedDatabase {
    admin: PgPool,
    pool: PgPool,
    schema: String,
}

impl IsolatedDatabase {
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

        // The identifier is generated exclusively from a fixed prefix, PID,
        // and decimal timestamp, so it cannot contain SQL syntax.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create isolated migration-test schema");

        let options = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect to isolated migration-test schema");

        Some(Self {
            admin,
            pool,
            schema,
        })
    }

    async fn cleanup(self) {
        self.pool.close().await;
        // `schema` came from `unique_schema_name`, whose alphabet excludes
        // identifier delimiters and SQL syntax.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated migration-test schema");
        self.admin.close().await;
    }
}

fn unique_schema_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("nebula_k2_{}_{nanos}", std::process::id())
}

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixture timestamp must be RFC 3339")
        .with_timezone(&Utc)
}

async fn applied_head(pool: &PgPool) -> TestResult<i64> {
    let head = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(pool)
    .await?
    .unwrap_or_default();
    Ok(head)
}

async fn seed_legacy_rows(pool: &PgPool) -> TestResult<ExpectedRows> {
    let named_id = CredentialId::new().to_string();
    let unnamed_id = CredentialId::new().to_string();
    let revoked_id = CredentialId::new().to_string();

    let named_created_at = timestamp("2025-01-02T03:04:05Z");
    let named_updated_at = timestamp("2025-02-03T04:05:06Z");
    let named_expires_at = timestamp("2027-03-04T05:06:07Z");
    let named_metadata = r#"{"display":{"display_name":"Production API","icon":"bolt","future":7},"owner_id":"tenant-a","opaque":{"n":1}}"#;
    let named_data: Vec<u8> = vec![0x00, 0x51, 0xff, 0x7a];

    let unnamed_created_at = timestamp("2025-03-04T05:06:07Z");
    let unnamed_updated_at = timestamp("2025-04-05T06:07:08Z");
    let unnamed_metadata = r#"{"owner_id":"tenant-b","opaque":[1,2,3]}"#;
    let unnamed_data: Vec<u8> = Vec::new();

    let revoked_created_at = timestamp("2025-05-06T07:08:09Z");
    let revoked_updated_at = timestamp("2025-06-07T08:09:10Z");
    let revoked_expires_at = timestamp("2028-07-08T09:10:11Z");
    let revoked_metadata = r#"{"revoked_at":null,"display":{"display_name":"Retired API"},"owner_id":"tenant-c","opaque":"discard-me"}"#;
    let revoked_data: Vec<u8> = vec![0xde, 0xad, 0xbe, 0xef];

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (
             $1, NULL, $2, $3, $4, $5,
             $6, $7, $8, $9, $10,
             $11, $12
         )",
    )
    .bind(&named_id)
    .bind("tenant-a")
    .bind("provider.api-token")
    .bind("active")
    .bind(17_i64)
    .bind(named_data.as_slice())
    .bind(7_i64)
    .bind(named_created_at)
    .bind(named_updated_at)
    .bind(named_expires_at)
    .bind(true)
    .bind(named_metadata)
    .execute(pool)
    .await
    .map_err(|error| std::io::Error::other(format!("seed named credential: {error}")))?;

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (
             $1, NULL, $2, $3, $4, $5,
             $6, $7, $8, $9, NULL,
             FALSE, $10
         )",
    )
    .bind(&unnamed_id)
    .bind("tenant-b")
    .bind("provider.unnamed")
    .bind("active")
    .bind(0_i64)
    .bind(unnamed_data.as_slice())
    .bind(2_i64)
    .bind(unnamed_created_at)
    .bind(unnamed_updated_at)
    .bind(unnamed_metadata)
    .execute(pool)
    .await
    .map_err(|error| std::io::Error::other(format!("seed unnamed credential: {error}")))?;

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (
             $1, $2, $3, $4, $5, $6,
             $7, $8, $9, $10, $11,
             TRUE, $12
         )",
    )
    .bind(&revoked_id)
    .bind("Retired API")
    .bind("tenant-c")
    .bind("provider.retired")
    .bind("revoked")
    .bind(i64::from(u32::MAX))
    .bind(revoked_data.as_slice())
    .bind(1_i64)
    .bind(revoked_created_at)
    .bind(revoked_updated_at)
    .bind(revoked_expires_at)
    .bind(revoked_metadata)
    .execute(pool)
    .await
    .map_err(|error| std::io::Error::other(format!("seed revoked credential: {error}")))?;

    sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, detected_at, crashed_holder, generation
         ) VALUES ($1, $2, $3, $4)",
    )
    .bind(&named_id)
    .bind(timestamp("2025-07-08T09:10:11Z"))
    .bind("legacy-replica")
    .bind(7_i64)
    .execute(pool)
    .await
    .map_err(|error| std::io::Error::other(format!("seed historical sentinel event: {error}")))?;

    let user_id: Vec<u8> = vec![0x42; 16];
    let user_email = "migration-unrelated@example.test".to_owned();
    let user_display_name = "Unrelated User".to_owned();
    let user_created_at = timestamp("2024-01-01T00:00:00Z");
    sqlx::query(
        "INSERT INTO users (id, email, display_name, created_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id.as_slice())
    .bind(&user_email)
    .bind(&user_display_name)
    .bind(user_created_at)
    .execute(pool)
    .await
    .map_err(|error| std::io::Error::other(format!("seed unrelated user: {error}")))?;

    let mut credentials = vec![
        CredentialRow {
            id: named_id,
            name: Some("Production API".to_owned()),
            owner_id: "tenant-a".to_owned(),
            credential_key: "provider.api-token".to_owned(),
            state_kind: "active".to_owned(),
            state_version: 17,
            data: named_data,
            version: 7,
            created_at: named_created_at,
            updated_at: named_updated_at,
            expires_at: Some(named_expires_at),
            reauth_required: true,
            metadata: named_metadata.to_owned(),
            record_state: "live".to_owned(),
            tombstoned_at: None,
        },
        CredentialRow {
            id: unnamed_id,
            name: None,
            owner_id: "tenant-b".to_owned(),
            credential_key: "provider.unnamed".to_owned(),
            state_kind: "active".to_owned(),
            state_version: 0,
            data: unnamed_data,
            version: 2,
            created_at: unnamed_created_at,
            updated_at: unnamed_updated_at,
            expires_at: None,
            reauth_required: false,
            metadata: unnamed_metadata.to_owned(),
            record_state: "live".to_owned(),
            tombstoned_at: None,
        },
        CredentialRow {
            id: revoked_id,
            name: None,
            owner_id: "tenant-c".to_owned(),
            credential_key: "provider.retired".to_owned(),
            state_kind: "revoked".to_owned(),
            state_version: i64::from(u32::MAX),
            data: Vec::new(),
            version: 1,
            created_at: revoked_created_at,
            updated_at: revoked_updated_at,
            expires_at: None,
            reauth_required: false,
            metadata: "{}".to_owned(),
            record_state: "tombstoned".to_owned(),
            tombstoned_at: Some(revoked_updated_at),
        },
    ];
    credentials.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(ExpectedRows {
        credentials,
        user_id,
        user_email,
        user_display_name,
        user_created_at,
    })
}

async fn credential_rows(pool: &PgPool) -> TestResult<Vec<CredentialRow>> {
    let rows = sqlx::query_as::<_, CredentialRow>(
        "SELECT
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata, record_state, tombstoned_at
         FROM credentials
         ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn update_is_rejected(pool: &PgPool, statement: &'static str, id: &str) -> TestResult<bool> {
    let mut transaction = pool.begin().await?;
    let rejected = sqlx::query(statement)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .is_err();
    transaction.rollback().await?;
    Ok(rejected)
}

async fn update_is_accepted(pool: &PgPool, statement: &'static str, id: &str) -> TestResult<bool> {
    let mut transaction = pool.begin().await?;
    let accepted = sqlx::query(statement)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .is_ok();
    transaction.rollback().await?;
    Ok(accepted)
}

async fn exercise_migration(pool: &PgPool) -> TestResult<MigrationEvidence> {
    MIGRATOR.run_to(38, pool).await?;
    let pre_migration_head = applied_head(pool).await?;
    if pre_migration_head != 38 {
        return Err(std::io::Error::other(format!(
            "fixture must stop at PostgreSQL migration 0038, got {pre_migration_head:04}"
        ))
        .into());
    }

    let expected = seed_legacy_rows(pool).await?;
    MIGRATOR.run(pool).await?;

    let post_migration_head = applied_head(pool).await?;
    if post_migration_head != 39 {
        return Err(std::io::Error::other(format!(
            "expected shared credential migration 0039, catalog stopped at {post_migration_head:04}"
        ))
        .into());
    }

    let first_credentials = credential_rows(pool).await?;
    let first_ledger_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await?;
    let migration_0039 = sqlx::query_as::<_, (i64, String, bool)>(
        "SELECT version, description, success
         FROM _sqlx_migrations
         WHERE version = 39",
    )
    .fetch_one(pool)
    .await?;

    let owner_id_column = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credentials'
           AND column_name = 'owner_id'",
    )
    .fetch_one(pool)
    .await?;
    let record_state_column = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credentials'
           AND column_name = 'record_state'",
    )
    .fetch_one(pool)
    .await?;
    let indexes = sqlx::query_as::<_, (String, String)>(
        "SELECT indexname, indexdef
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND tablename = 'credentials'
         ORDER BY indexname",
    )
    .fetch_all(pool)
    .await?;
    let sentinel_claim_id_column = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT udt_name, is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credential_sentinel_events'
           AND column_name = 'claim_id'",
    )
    .fetch_one(pool)
    .await?;
    let sentinel_indexes = sqlx::query_as::<_, (String, String)>(
        "SELECT indexname, indexdef
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND tablename = 'credential_sentinel_events'
         ORDER BY indexname",
    )
    .fetch_all(pool)
    .await?;
    let historical_sentinel_claim_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT claim_id
         FROM credential_sentinel_events
         WHERE crashed_holder = 'legacy-replica'",
    )
    .fetch_one(pool)
    .await?;

    let incident = Uuid::new_v4();
    let mut incident_transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, claim_id, detected_at, crashed_holder, generation
         ) VALUES ($1, $2, CURRENT_TIMESTAMP, $3, $4)",
    )
    .bind(CredentialId::new().to_string())
    .bind(incident)
    .bind("first")
    .bind(0_i64)
    .execute(&mut *incident_transaction)
    .await?;
    let duplicate_incident_rejected = sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, claim_id, detected_at, crashed_holder, generation
         ) VALUES ($1, $2, CURRENT_TIMESTAMP, $3, $4)",
    )
    .bind(CredentialId::new().to_string())
    .bind(incident)
    .bind("second")
    .bind(99_i64)
    .execute(&mut *incident_transaction)
    .await
    .is_err();
    incident_transaction.rollback().await?;

    let unrelated_user = sqlx::query_as::<_, (Vec<u8>, String, String, DateTime<Utc>)>(
        "SELECT id, email, display_name, created_at
         FROM users
         WHERE id = $1",
    )
    .bind(expected.user_id.as_slice())
    .fetch_one(pool)
    .await?;

    let named_id = expected
        .credentials
        .iter()
        .find(|row| row.record_state == "live" && row.name.is_some())
        .map(|row| row.id.as_str())
        .ok_or_else(|| std::io::Error::other("missing named live fixture"))?;
    let tombstoned_id = expected
        .credentials
        .iter()
        .find(|row| row.record_state == "tombstoned")
        .map(|row| row.id.as_str())
        .ok_or_else(|| std::io::Error::other("missing tombstone fixture"))?;

    let owner_null_rejected = update_is_rejected(
        pool,
        "UPDATE credentials SET owner_id = NULL WHERE id = $1",
        named_id,
    )
    .await?;
    let live_max_version_rejected = update_is_rejected(
        pool,
        "UPDATE credentials SET version = 9223372036854775807 WHERE id = $1",
        named_id,
    )
    .await?;
    let tombstone_payload_rejected = update_is_rejected(
        pool,
        "UPDATE credentials SET data = '\\x01'::bytea WHERE id = $1",
        tombstoned_id,
    )
    .await?;
    let tombstone_max_version_accepted = update_is_accepted(
        pool,
        "UPDATE credentials SET version = 9223372036854775807 WHERE id = $1",
        tombstoned_id,
    )
    .await?;

    let staging_relation = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass(current_schema() || '.credentials_0039')::text",
    )
    .fetch_one(pool)
    .await?;

    MIGRATOR.run(pool).await?;
    let second_credentials = credential_rows(pool).await?;
    let second_ledger_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await?;

    Ok(MigrationEvidence {
        expected,
        first_credentials,
        second_credentials,
        first_ledger_count,
        second_ledger_count,
        migration_0039,
        owner_id_column,
        record_state_column,
        indexes,
        sentinel_claim_id_column,
        sentinel_indexes,
        historical_sentinel_claim_id,
        duplicate_incident_rejected,
        unrelated_user,
        owner_null_rejected,
        live_max_version_rejected,
        tombstone_payload_rejected,
        tombstone_max_version_accepted,
        staging_relation,
    })
}

#[tokio::test]
async fn postgres_0038_to_0039_preserves_live_rows_and_converts_tombstones() {
    let Some(database) = IsolatedDatabase::connect().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };

    let outcome = exercise_migration(&database.pool).await;
    database.cleanup().await;
    let evidence = outcome.expect("exercise PostgreSQL credential migration 0039");

    assert_eq!(
        evidence.first_credentials, evidence.expected.credentials,
        "0039 must preserve every live byte and convert only terminal rows"
    );
    assert_eq!(
        evidence.second_credentials, evidence.first_credentials,
        "a second migrator start must not mutate credential rows"
    );
    assert_eq!(
        evidence.second_ledger_count, evidence.first_ledger_count,
        "a second migrator start must not append migration rows"
    );
    assert_eq!(
        evidence.migration_0039,
        (39, "credentials owner and record state".to_owned(), true),
        "the shared migration must have one successful canonical ledger entry"
    );
    assert_eq!(
        evidence.owner_id_column,
        ("NO".to_owned(), None),
        "owner_id must be non-null without a write-time repair default"
    );
    assert_eq!(
        evidence.record_state_column,
        ("NO".to_owned(), None),
        "record_state must be non-null and have no default"
    );
    assert_eq!(
        evidence.sentinel_claim_id_column,
        ("uuid".to_owned(), "YES".to_owned(), None),
        "historical sentinel rows need a nullable typed incident identity"
    );
    assert_eq!(
        evidence.historical_sentinel_claim_id, None,
        "0039 must not fabricate identities for historical evidence"
    );
    let incident_index = evidence
        .sentinel_indexes
        .iter()
        .find(|(name, _)| name == "idx_credential_sentinel_events_claim_id")
        .expect("global claim-id uniqueness index must exist");
    assert!(
        incident_index.1.contains("UNIQUE INDEX")
            && incident_index.1.contains("(claim_id)")
            && incident_index.1.contains("claim_id IS NOT NULL"),
        "incident identity must be globally unique and partial: {}",
        incident_index.1
    );
    assert!(
        evidence.duplicate_incident_rejected,
        "one claim UUID cannot identify incidents for two credentials"
    );
    assert_eq!(
        evidence.unrelated_user,
        (
            evidence.expected.user_id,
            evidence.expected.user_email,
            evidence.expected.user_display_name,
            evidence.expected.user_created_at,
        ),
        "0039 must not alter unrelated runtime data"
    );

    let owner_name = evidence
        .indexes
        .iter()
        .find(|(name, _)| name == "idx_credentials_owner_name")
        .expect("full owner/name unique index must exist");
    assert!(
        owner_name.1.contains("UNIQUE INDEX")
            && owner_name.1.contains("(owner_id, name)")
            && !owner_name.1.contains(" WHERE "),
        "owner/name uniqueness must be full and non-partial: {}",
        owner_name.1
    );
    assert!(
        evidence
            .indexes
            .iter()
            .any(|(name, _)| name == "idx_credentials_state_kind"),
        "state-kind lookup semantics must be preserved"
    );
    assert!(
        evidence.indexes.iter().any(|(name, definition)| {
            name == "idx_credentials_expiring"
                && definition.contains("expires_at")
                && definition.contains(" WHERE ")
        }),
        "sparse expiry lookup semantics must be preserved"
    );
    assert!(
        evidence.indexes.iter().all(|(name, definition)| {
            name == "idx_credentials_owner_name" || !definition.ends_with("(owner_id)")
        }),
        "0039 must not leave a redundant owner-only index"
    );

    assert!(evidence.owner_null_rejected, "owner_id NULL must fail");
    assert!(
        evidence.live_max_version_rejected,
        "live rows must reserve i64::MAX for tombstoning"
    );
    assert!(
        evidence.tombstone_payload_rejected,
        "tombstones must not regain secret bytes"
    );
    assert!(
        evidence.tombstone_max_version_accepted,
        "tombstones may consume i64::MAX"
    );
    assert_eq!(
        evidence.staging_relation, None,
        "0039 must not leave a staging relation"
    );
}

#[tokio::test]
async fn failed_0039_rolls_back_completely_and_can_retry_cleanly() -> TestResult<()> {
    let Some(database) = IsolatedDatabase::connect().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return Ok(());
    };
    MIGRATOR.run_to(38, &database.pool).await?;
    sqlx::query("DROP INDEX idx_credentials_owner_name")
        .execute(&database.pool)
        .await?;
    sqlx::query(
        "CREATE INDEX idx_credentials_owner_name
         ON credentials(owner_id, name)
         WHERE name IS NOT NULL",
    )
    .execute(&database.pool)
    .await?;

    let first_id = CredentialId::new().to_string();
    let second_id = CredentialId::new().to_string();
    for credential_id in [&first_id, &second_id] {
        sqlx::query(
            "INSERT INTO credentials (
                 id, name, owner_id, credential_key, state_kind, state_version,
                 data, version, created_at, updated_at, expires_at,
                 reauth_required, metadata
             ) VALUES (
                 $1, 'Shared', 'rollback-owner', 'provider.rollback', 'ready', 0,
                 ''::bytea, 1, now(), now(), NULL, FALSE,
                 '{\"display\":{\"display_name\":\"Shared\"}}'
             )",
        )
        .bind(credential_id)
        .execute(&database.pool)
        .await?;
    }

    let failed = MIGRATOR.run(&database.pool).await;
    assert!(
        failed.is_err(),
        "the final unique projection must reject duplicate legacy names"
    );
    let record_state_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credentials'
           AND column_name = 'record_state'",
    )
    .fetch_one(&database.pool)
    .await?;
    let migration_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 39")
            .fetch_one(&database.pool)
            .await?;
    let claim_id_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credential_sentinel_events'
           AND column_name = 'claim_id'",
    )
    .fetch_one(&database.pool)
    .await?;
    let claim_id_indexes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND indexname = 'idx_credential_sentinel_events_claim_id'",
    )
    .fetch_one(&database.pool)
    .await?;
    assert_eq!(
        record_state_columns, 0,
        "failed 0039 must roll back every DDL change"
    );
    assert_eq!(
        migration_rows, 0,
        "failed 0039 must not append a ledger row"
    );
    assert_eq!(
        (claim_id_columns, claim_id_indexes),
        (0, 0),
        "failed 0039 must roll back the incident column and index together"
    );

    sqlx::query(
        "UPDATE credentials
         SET name = 'Recovered',
             metadata = '{\"display\":{\"display_name\":\"Recovered\"}}'
         WHERE id = $1",
    )
    .bind(&second_id)
    .execute(&database.pool)
    .await?;
    MIGRATOR.run(&database.pool).await?;

    let states: Vec<String> =
        sqlx::query_scalar("SELECT record_state FROM credentials ORDER BY id")
            .fetch_all(&database.pool)
            .await?;
    let migration_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM _sqlx_migrations
         WHERE version = 39 AND success",
    )
    .fetch_one(&database.pool)
    .await?;
    assert_eq!(states, ["live", "live"]);
    assert_eq!(migration_rows, 1);
    let repaired_claim_id_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'credential_sentinel_events'
           AND column_name = 'claim_id'",
    )
    .fetch_one(&database.pool)
    .await?;
    assert_eq!(repaired_claim_id_columns, 1);

    database.cleanup().await;
    Ok(())
}
