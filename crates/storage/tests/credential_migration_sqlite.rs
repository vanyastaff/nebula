//! SQLite acceptance contract for credential migration 0039.

#![cfg(feature = "sqlite")]

use std::{path::Path, str::FromStr};

use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

// Compile the canonical on-disk catalog into this acceptance-test binary.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

const NAMED_ID: &str = "cred_01JAZ000000000000000000001";
const UNNAMED_ID: &str = "cred_01JAZ000000000000000000002";
const REVOKED_ID: &str = "cred_01JAZ000000000000000000003";

const NAMED_METADATA: &str = r#"{"owner_id":"owner-a","display":{"display_name":"Primary API","icon":"bolt"},"future":{"nested":[1,2,3]}}"#;
const UNNAMED_METADATA: &str =
    r#"{"owner_id":"owner-b","display":{"icon":"circle"},"nested":[{"x":1}]}"#;
const REVOKED_METADATA: &str =
    r#"{"revoked_at":null,"display":{"display_name":"Retired"},"future":true}"#;

async fn file_pool(path: &Path) -> sqlx::SqlitePool {
    let url = format!("sqlite://{}", path.display());
    let options = SqliteConnectOptions::from_str(&url)
        .expect("temporary SQLite URL must parse")
        .create_if_missing(true)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("temporary SQLite database must open")
}

async fn seed_legacy_rows(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, email, display_name, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(vec![0x2a_u8; 16])
    .bind("unrelated@example.test")
    .bind("Unrelated")
    .bind("2026-07-23T00:00:00Z")
    .execute(pool)
    .await
    .expect("unrelated legacy row must seed");

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(NAMED_ID)
    .bind(Option::<String>::None)
    .bind("owner-a")
    .bind("provider.token")
    .bind("ready")
    .bind(4_i64)
    .bind(vec![0x10_u8, 0x20, 0x30, 0x40])
    .bind(7_i64)
    .bind(1_700_000_000_001_i64)
    .bind(1_700_000_000_101_i64)
    .bind(1_800_000_000_001_i64)
    .bind(1_i64)
    .bind(NAMED_METADATA)
    .execute(pool)
    .await
    .expect("named live legacy credential must seed");

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(UNNAMED_ID)
    .bind(Option::<String>::None)
    .bind("owner-b")
    .bind("provider.empty")
    .bind("ready")
    .bind(0_i64)
    .bind(Vec::<u8>::new())
    .bind(2_i64)
    .bind(1_700_000_000_002_i64)
    .bind(1_700_000_000_102_i64)
    .bind(Option::<i64>::None)
    .bind(0_i64)
    .bind(UNNAMED_METADATA)
    .execute(pool)
    .await
    .expect("unnamed zero-byte live credential must seed");

    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(REVOKED_ID)
    .bind("Legacy label")
    .bind("owner-c")
    .bind("provider.retired")
    .bind("revoked")
    .bind(9_i64)
    .bind(vec![0xde_u8, 0xad, 0xbe, 0xef])
    .bind(1_i64)
    .bind(1_700_000_000_003_i64)
    .bind(1_700_000_000_103_i64)
    .bind(1_800_000_000_003_i64)
    .bind(1_i64)
    .bind(REVOKED_METADATA)
    .execute(pool)
    .await
    .expect("legacy revoked credential must seed");

    sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, detected_at, crashed_holder, generation
         ) VALUES (?, ?, ?, ?)",
    )
    .bind(NAMED_ID)
    .bind(1_700_000_000_200_i64)
    .bind("legacy-replica")
    .bind(7_i64)
    .execute(pool)
    .await
    .expect("historical sentinel event must seed");
}

async fn assert_live_rows_are_preserved(pool: &sqlx::SqlitePool) {
    let named = sqlx::query(
        "SELECT name, owner_id, credential_key, state_kind, state_version,
                data, version, created_at, updated_at, expires_at,
                reauth_required, metadata, record_state, tombstoned_at
         FROM credentials
         WHERE id = ?",
    )
    .bind(NAMED_ID)
    .fetch_one(pool)
    .await
    .expect("named live credential must survive migration");
    assert_eq!(
        named.get::<Option<String>, _>("name").as_deref(),
        Some("Primary API")
    );
    assert_eq!(named.get::<String, _>("owner_id"), "owner-a");
    assert_eq!(named.get::<String, _>("credential_key"), "provider.token");
    assert_eq!(named.get::<String, _>("state_kind"), "ready");
    assert_eq!(named.get::<i64, _>("state_version"), 4);
    assert_eq!(named.get::<Vec<u8>, _>("data"), [0x10, 0x20, 0x30, 0x40]);
    assert_eq!(named.get::<i64, _>("version"), 7);
    assert_eq!(named.get::<i64, _>("created_at"), 1_700_000_000_001);
    assert_eq!(named.get::<i64, _>("updated_at"), 1_700_000_000_101);
    assert_eq!(
        named.get::<Option<i64>, _>("expires_at"),
        Some(1_800_000_000_001)
    );
    assert_eq!(named.get::<i64, _>("reauth_required"), 1);
    assert_eq!(named.get::<String, _>("metadata"), NAMED_METADATA);
    assert_eq!(named.get::<String, _>("record_state"), "live");
    assert_eq!(named.get::<Option<i64>, _>("tombstoned_at"), None);

    let unnamed = sqlx::query(
        "SELECT name, data, metadata, record_state, tombstoned_at
         FROM credentials
         WHERE id = ?",
    )
    .bind(UNNAMED_ID)
    .fetch_one(pool)
    .await
    .expect("unnamed live credential must survive migration");
    assert_eq!(unnamed.get::<Option<String>, _>("name"), None);
    assert_eq!(unnamed.get::<Vec<u8>, _>("data"), Vec::<u8>::new());
    assert_eq!(unnamed.get::<String, _>("metadata"), UNNAMED_METADATA);
    assert_eq!(unnamed.get::<String, _>("record_state"), "live");
    assert_eq!(unnamed.get::<Option<i64>, _>("tombstoned_at"), None);
}

async fn assert_legacy_revocation_becomes_tombstone(pool: &sqlx::SqlitePool) {
    let row = sqlx::query(
        "SELECT name, owner_id, credential_key, state_kind, state_version,
                data, version, created_at, updated_at, expires_at,
                reauth_required, metadata, record_state, tombstoned_at
         FROM credentials
         WHERE id = ?",
    )
    .bind(REVOKED_ID)
    .fetch_one(pool)
    .await
    .expect("legacy revoked credential must become a tombstone");

    assert_eq!(row.get::<Option<String>, _>("name"), None);
    assert_eq!(row.get::<String, _>("owner_id"), "owner-c");
    assert_eq!(row.get::<String, _>("credential_key"), "provider.retired");
    assert_eq!(row.get::<String, _>("state_kind"), "revoked");
    assert_eq!(row.get::<i64, _>("state_version"), 9);
    assert_eq!(row.get::<Vec<u8>, _>("data"), Vec::<u8>::new());
    assert_eq!(row.get::<i64, _>("version"), 1);
    assert_eq!(row.get::<i64, _>("created_at"), 1_700_000_000_003);
    assert_eq!(row.get::<i64, _>("updated_at"), 1_700_000_000_103);
    assert_eq!(row.get::<Option<i64>, _>("expires_at"), None);
    assert_eq!(row.get::<i64, _>("reauth_required"), 0);
    assert_eq!(row.get::<String, _>("metadata"), "{}");
    assert_eq!(row.get::<String, _>("record_state"), "tombstoned");
    assert_eq!(
        row.get::<Option<i64>, _>("tombstoned_at"),
        Some(1_700_000_000_103)
    );
}

async fn assert_final_schema(pool: &sqlx::SqlitePool) {
    let columns = sqlx::query("PRAGMA table_info('credentials')")
        .fetch_all(pool)
        .await
        .expect("credentials table metadata must be readable");
    let column = |name: &str| {
        columns
            .iter()
            .find(|row| row.get::<String, _>("name") == name)
            .unwrap_or_else(|| panic!("missing credentials column {name}"))
    };
    assert_eq!(column("owner_id").get::<i64, _>("notnull"), 1);
    assert_eq!(column("record_state").get::<i64, _>("notnull"), 1);
    assert_eq!(
        column("record_state").get::<Option<String>, _>("dflt_value"),
        None
    );

    let indexes = sqlx::query("PRAGMA index_list('credentials')")
        .fetch_all(pool)
        .await
        .expect("credentials indexes must be readable");
    let owner_name = indexes
        .iter()
        .find(|row| row.get::<String, _>("name") == "idx_credentials_owner_name")
        .expect("owner/name unique index must exist");
    assert_eq!(owner_name.get::<i64, _>("unique"), 1);
    assert_eq!(owner_name.get::<i64, _>("partial"), 0);
    for required in [
        "idx_credentials_owner_name",
        "idx_credentials_state_kind",
        "idx_credentials_expiring",
    ] {
        assert!(
            indexes
                .iter()
                .any(|row| row.get::<String, _>("name") == required),
            "missing required credential index {required}"
        );
    }

    let indexed_columns = sqlx::query("PRAGMA index_info('idx_credentials_owner_name')")
        .fetch_all(pool)
        .await
        .expect("owner/name index metadata must be readable")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(indexed_columns, ["owner_id", "name"]);

    let index_shapes = sqlx::query(
        "SELECT indexes.name AS index_name,
                group_concat(columns.name, ',') AS indexed_columns
         FROM pragma_index_list('credentials') AS indexes
         JOIN pragma_index_info(indexes.name) AS columns
         GROUP BY indexes.name",
    )
    .fetch_all(pool)
    .await
    .expect("all credential index shapes must be readable");
    assert!(
        index_shapes
            .iter()
            .all(|row| row.get::<String, _>("indexed_columns") != "owner_id"),
        "0039 must not leave a redundant owner-only index"
    );

    let staging_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE type = 'table' AND name LIKE 'credentials_0039%'",
    )
    .fetch_one(pool)
    .await
    .expect("SQLite schema must be queryable");
    assert_eq!(staging_tables, 0, "migration must not leave staging tables");

    let sentinel_columns = sqlx::query("PRAGMA table_info('credential_sentinel_events')")
        .fetch_all(pool)
        .await
        .expect("sentinel event metadata must be readable");
    let claim_id = sentinel_columns
        .iter()
        .find(|row| row.get::<String, _>("name") == "claim_id")
        .expect("0039 must add sentinel incident identity");
    assert_eq!(claim_id.get::<String, _>("type"), "TEXT");
    assert_eq!(claim_id.get::<i64, _>("notnull"), 0);

    let historical_claim_id: Option<String> = sqlx::query_scalar(
        "SELECT claim_id
         FROM credential_sentinel_events
         WHERE crashed_holder = 'legacy-replica'",
    )
    .fetch_one(pool)
    .await
    .expect("historical event must survive migration");
    assert_eq!(
        historical_claim_id, None,
        "historical evidence has no fabricated incident UUID"
    );

    let incident_index = sqlx::query(
        "SELECT [unique], partial
         FROM pragma_index_list('credential_sentinel_events')
         WHERE name = 'idx_credential_sentinel_events_claim_id'",
    )
    .fetch_one(pool)
    .await
    .expect("global claim-id uniqueness index must exist");
    assert_eq!(incident_index.get::<i64, _>("unique"), 1);
    assert_eq!(incident_index.get::<i64, _>("partial"), 1);

    let incident = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, claim_id, detected_at, crashed_holder, generation
         ) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(NAMED_ID)
    .bind(&incident)
    .bind(1_700_000_000_300_i64)
    .bind("first")
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("first incident identity must insert");
    let duplicate = sqlx::query(
        "INSERT INTO credential_sentinel_events (
             credential_id, claim_id, detected_at, crashed_holder, generation
         ) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(UNNAMED_ID)
    .bind(&incident)
    .bind(1_700_000_000_301_i64)
    .bind("second")
    .bind(99_i64)
    .execute(pool)
    .await;
    assert!(
        duplicate.is_err(),
        "claim UUID uniqueness must be global across credentials"
    );
}

async fn assert_constraints_reject_invalid_rows(pool: &sqlx::SqlitePool) {
    let invalid_tombstone = sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata, record_state, tombstoned_at
         ) VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, '{}', 'tombstoned', NULL)",
    )
    .bind("cred_invalid_tombstone")
    .bind("owner-invalid")
    .bind("provider.invalid")
    .bind("ready")
    .bind(0_i64)
    .bind(Vec::<u8>::new())
    .bind(1_i64)
    .bind(1_i64)
    .bind(1_i64)
    .execute(pool)
    .await;
    assert!(
        invalid_tombstone.is_err(),
        "a tombstone without tombstoned_at must violate the structural check"
    );

    let exhausted_live = sqlx::query(
        "UPDATE credentials
         SET version = 9223372036854775807
         WHERE id = ?",
    )
    .bind(NAMED_ID)
    .execute(pool)
    .await;
    assert!(
        exhausted_live.is_err(),
        "a live row must reserve the final i64 version for tombstoning"
    );

    let mismatched_name = sqlx::query("UPDATE credentials SET name = 'different' WHERE id = ?")
        .bind(NAMED_ID)
        .execute(pool)
        .await;
    assert!(
        mismatched_name.is_err(),
        "a live physical name must equal its metadata projection"
    );

    let invalid_state_version =
        sqlx::query("UPDATE credentials SET state_version = 4294967296 WHERE id = ?")
            .bind(NAMED_ID)
            .execute(pool)
            .await;
    assert!(
        invalid_state_version.is_err(),
        "state_version must remain inside the u32 range"
    );

    let zero_version = sqlx::query("UPDATE credentials SET version = 0 WHERE id = ?")
        .bind(NAMED_ID)
        .execute(pool)
        .await;
    assert!(
        zero_version.is_err(),
        "persistence version zero must be rejected"
    );

    let unknown_record_state =
        sqlx::query("UPDATE credentials SET record_state = 'unknown' WHERE id = ?")
            .bind(NAMED_ID)
            .execute(pool)
            .await;
    assert!(
        unknown_record_state.is_err(),
        "record_state must be live or tombstoned"
    );

    let non_object_metadata = sqlx::query("UPDATE credentials SET metadata = '[]' WHERE id = ?")
        .bind(NAMED_ID)
        .execute(pool)
        .await;
    assert!(
        non_object_metadata.is_err(),
        "credential metadata must remain a JSON object"
    );
}

#[tokio::test]
async fn migration_0039_rejects_a_legacy_name_projection_mismatch() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let pool = file_pool(&directory.path().join("credential-mismatch.sqlite")).await;

    MIGRATOR
        .run_to(35, &pool)
        .await
        .expect("legacy SQLite migrations through 0035 must apply");
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, ?, ?, ?, ?, 0, ?, 1, ?, ?, NULL, 0, ?)",
    )
    .bind("cred_01JAZ000000000000000000098")
    .bind("physical")
    .bind("owner-mismatch")
    .bind("provider.mismatch")
    .bind("ready")
    .bind(Vec::<u8>::new())
    .bind(1_700_000_000_098_i64)
    .bind(1_700_000_000_198_i64)
    .bind(r#"{"display":{"display_name":"projected"}}"#)
    .execute(&pool)
    .await
    .expect("mismatched legacy fixture must seed before 0039");

    let failed = MIGRATOR.run(&pool).await;
    assert!(
        failed.is_err(),
        "0039 must reject rather than guess between physical and projected names"
    );

    let legacy_name: String =
        sqlx::query_scalar("SELECT name FROM credentials WHERE owner_id = 'owner-mismatch'")
            .fetch_one(&pool)
            .await
            .expect("failed migration must preserve the legacy row");
    assert_eq!(legacy_name, "physical");
    let migration_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 39")
            .fetch_one(&pool)
            .await
            .expect("migration ledger must remain readable");
    assert_eq!(migration_rows, 0);
}

#[tokio::test]
async fn failed_migration_0039_rolls_back_and_can_be_retried() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let pool = file_pool(&directory.path().join("credential-rollback.sqlite")).await;

    MIGRATOR
        .run_to(35, &pool)
        .await
        .expect("legacy SQLite migrations through 0035 must apply");
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, NULL, 0, '{}')",
    )
    .bind("cred_01JAZ000000000000000000099")
    .bind("provider.repairable")
    .bind("ready")
    .bind(0_i64)
    .bind(Vec::<u8>::new())
    .bind(1_i64)
    .bind(1_700_000_000_099_i64)
    .bind(1_700_000_000_199_i64)
    .execute(&pool)
    .await
    .expect("ownerless legacy fixture must seed before 0039");

    let failed = MIGRATOR.run(&pool).await;
    assert!(
        failed.is_err(),
        "0039 must fail rather than invent an owner for legacy data"
    );

    let credential_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credentials")
        .fetch_one(&pool)
        .await
        .expect("legacy credentials table must survive rollback");
    assert_eq!(credential_rows, 1);
    let record_state_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM pragma_table_info('credentials')
         WHERE name = 'record_state'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy table shape must be inspectable");
    assert_eq!(
        record_state_columns, 0,
        "failed rebuild must restore the pre-0039 table"
    );
    let migration_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 39")
            .fetch_one(&pool)
            .await
            .expect("migration ledger must remain readable");
    assert_eq!(
        migration_rows, 0,
        "failed 0039 must not leave a ledger claim"
    );
    let staging_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE type = 'table' AND name = 'credentials_0039'",
    )
    .fetch_one(&pool)
    .await
    .expect("SQLite schema must remain readable");
    assert_eq!(
        staging_tables, 0,
        "failed 0039 must not leave its rebuild table"
    );
    let claim_id_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM pragma_table_info('credential_sentinel_events')
         WHERE name = 'claim_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("sentinel event shape must remain readable");
    let claim_id_indexes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE type = 'index'
           AND name = 'idx_credential_sentinel_events_claim_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("sentinel indexes must remain readable");
    assert_eq!(
        (claim_id_columns, claim_id_indexes),
        (0, 0),
        "failed 0039 must roll back the incident column and index together"
    );

    sqlx::query("UPDATE credentials SET owner_id = ? WHERE owner_id IS NULL")
        .bind("owner-repaired")
        .execute(&pool)
        .await
        .expect("operator repair fixture must succeed");
    MIGRATOR
        .run(&pool)
        .await
        .expect("the canonical migration must succeed after an explicit repair");

    let repaired_head: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .expect("migration ledger head must remain readable");
    assert_eq!(repaired_head, 39);
    let repaired_claim_id_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM pragma_table_info('credential_sentinel_events')
         WHERE name = 'claim_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("repaired sentinel shape must be readable");
    assert_eq!(repaired_claim_id_columns, 1);
}

#[tokio::test]
async fn migration_0039_rebuilds_credentials_without_live_data_loss() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let pool = file_pool(&directory.path().join("credential-migration.sqlite")).await;

    MIGRATOR
        .run_to(35, &pool)
        .await
        .expect("legacy SQLite migrations through 0035 must apply");
    seed_legacy_rows(&pool).await;

    MIGRATOR
        .run(&pool)
        .await
        .expect("all credential migrations must apply");

    let head: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0)
         FROM _sqlx_migrations
         WHERE success = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("migration ledger head must be readable");
    assert_eq!(head, 39, "K2 must install logical migration 0039");

    assert_live_rows_are_preserved(&pool).await;
    assert_legacy_revocation_becomes_tombstone(&pool).await;
    assert_final_schema(&pool).await;
    assert_constraints_reject_invalid_rows(&pool).await;

    let unrelated_users: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = 'unrelated@example.test'")
            .fetch_one(&pool)
            .await
            .expect("unrelated table must remain readable");
    assert_eq!(
        unrelated_users, 1,
        "migration must not alter unrelated data"
    );

    let ledger_rows_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration ledger must be readable");
    MIGRATOR
        .run(&pool)
        .await
        .expect("rerunning the canonical migrator must be idempotent");
    let ledger_rows_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration ledger must remain readable");
    assert_eq!(ledger_rows_after, ledger_rows_before);

    let migration_0039: (String, bool, i64) = sqlx::query_as(
        "SELECT description, success, COUNT(*) OVER ()
         FROM _sqlx_migrations
         WHERE version = 39",
    )
    .fetch_one(&pool)
    .await
    .expect("migration 0039 must have one successful ledger row");
    assert_eq!(
        migration_0039,
        ("credentials owner and record state".to_owned(), true, 1)
    );
}
