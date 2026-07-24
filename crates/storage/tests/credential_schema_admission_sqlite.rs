//! SQLite ready-store admission and migration boundary.

#![cfg(feature = "sqlite")]

use std::{path::Path, str::FromStr};

use nebula_core::CredentialId;
use nebula_storage::credential::{
    CredentialSchemaAdmissionReason, CredentialStoreStartupError, SqliteCredentialPersistence,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

async fn raw_pool(path: &Path) -> sqlx::SqlitePool {
    let url = format!("sqlite://{}", path.display());
    let options = SqliteConnectOptions::from_str(&url)
        .expect("temporary SQLite URL must parse")
        .create_if_missing(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("temporary SQLite database must open")
}

async fn rewrite_schema_sql(
    pool: &sqlx::SqlitePool,
    object_type: &str,
    name: &str,
    needle: &str,
    replacement: &str,
) {
    let original: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = ? AND name = ?")
            .bind(object_type)
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("canonical schema SQL must exist");
    let rewritten = original.replacen(needle, replacement, 1);
    assert_ne!(
        rewritten, original,
        "fixture rewrite must alter the intended canonical fragment"
    );
    set_schema_sql(pool, object_type, name, &rewritten).await;
}

async fn set_schema_sql(pool: &sqlx::SqlitePool, object_type: &str, name: &str, rewritten: &str) {
    sqlx::query("PRAGMA writable_schema = ON")
        .execute(pool)
        .await
        .expect("test fixture must enable catalog rewriting");
    let updated = sqlx::query("UPDATE sqlite_schema SET sql = ? WHERE type = ? AND name = ?")
        .bind(rewritten)
        .bind(object_type)
        .bind(name)
        .execute(pool)
        .await
        .expect("test fixture must rewrite one catalog entry");
    assert_eq!(updated.rows_affected(), 1);
    sqlx::query("PRAGMA schema_version = 424242")
        .execute(pool)
        .await
        .expect("fixture catalog rewrite must invalidate schema caches");
    sqlx::query("PRAGMA writable_schema = OFF")
        .execute(pool)
        .await
        .expect("test fixture must disable catalog rewriting");
}

async fn assert_invalid_current_shape(path: &Path) {
    let error = SqliteCredentialPersistence::connect(&file_url(path))
        .await
        .expect_err("structural schema drift must fail closed");
    assert!(
        matches!(
            &error,
            CredentialStoreStartupError::UnsupportedSchemaVersion(unsupported)
                if unsupported.reason()
                    == &CredentialSchemaAdmissionReason::InvalidCredentialsRelation
        ),
        "expected invalid credential relation, got {error:?}"
    );
}

fn file_url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

fn sidecar_membership(path: &Path) -> [bool; 3] {
    let base = path.as_os_str().to_string_lossy();
    [
        Path::new(&format!("{base}-journal")).exists(),
        Path::new(&format!("{base}-wal")).exists(),
        Path::new(&format!("{base}-shm")).exists(),
    ]
}

async fn reject_file_unchanged(path: &Path) -> CredentialStoreStartupError {
    let bytes_before = std::fs::read(path).expect("fixture database bytes must be readable");
    let sidecars_before = sidecar_membership(path);
    let error = SqliteCredentialPersistence::connect(&file_url(path))
        .await
        .expect_err("unsupported fixture must fail closed");
    assert_eq!(
        std::fs::read(path).expect("rejected database bytes must remain readable"),
        bytes_before
    );
    assert_eq!(sidecar_membership(path), sidecars_before);
    error
}

async fn insert_legacy_metadata(pool: &sqlx::SqlitePool, metadata: &str) {
    insert_legacy_record(
        pool,
        &CredentialId::new().to_string(),
        Some("owner-rejection"),
        None,
        0,
        1,
        metadata,
    )
    .await;
}

async fn insert_legacy_record(
    pool: &sqlx::SqlitePool,
    id: &str,
    owner: Option<&str>,
    name: Option<&str>,
    state_version: i64,
    version: i64,
    metadata: &str,
) {
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, ?, ?, ?, ?, ?, zeroblob(0), ?, ?, ?, NULL, 0, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(owner)
    .bind("provider.rejection")
    .bind("ready")
    .bind(state_version)
    .bind(version)
    .bind(1_700_000_000_000_i64)
    .bind(1_700_000_000_001_i64)
    .bind(metadata)
    .execute(pool)
    .await
    .expect("legacy rejection fixture must seed");
}

#[tokio::test]
async fn fresh_file_and_memory_are_admitted_and_file_reopens_at_0040() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("fresh.sqlite");
    let url = file_url(&path);

    let first = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("genuine fresh file must be admitted and migrated");
    drop(first);

    let pool = raw_pool(&path).await;
    let head: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .expect("migration ledger must be readable");
    assert_eq!(head, 40);
    pool.close().await;

    let second = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("a canonical current file must reopen without mutation");
    drop(second);

    let memory = SqliteCredentialPersistence::connect("sqlite::memory:")
        .await
        .expect("the supported single-connection memory form must be admitted");
    drop(memory);
}

#[tokio::test]
async fn two_fresh_file_starters_serialize_readiness_without_partial_schema() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("contended-readiness.sqlite");
    let url = file_url(&path);

    let (first, second) = tokio::join!(
        SqliteCredentialPersistence::connect(&url),
        SqliteCredentialPersistence::connect(&url),
    );
    drop(first.expect("first starter must reach the canonical ready state"));
    drop(second.expect("second starter must observe the same canonical ready state"));

    let pool = raw_pool(&path).await;
    let (head, successful): (i64, i64) =
        sqlx::query_as("SELECT MAX(version), COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .expect("the contended migration ledger must be readable");
    assert_eq!(head, 40);
    assert_eq!(
        successful,
        i64::try_from(MIGRATOR.iter().count()).expect("migration count must fit in i64"),
        "serialized starters must not duplicate or partially apply migrations"
    );
    pool.close().await;
}

#[tokio::test]
async fn canonical_0035_is_admitted_and_upgraded() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("upgrade.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run_to(35, &pool)
        .await
        .expect("canonical legacy prefix must install");
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, NULL, ?, ?, ?, 0, ?, 1, ?, ?, NULL, 0, '{}')",
    )
    .bind(CredentialId::new().to_string())
    .bind("owner-upgrade")
    .bind("provider.upgrade")
    .bind("ready")
    .bind(Vec::<u8>::new())
    .bind(1_700_000_000_000_i64)
    .bind(1_700_000_000_001_i64)
    .execute(&pool)
    .await
    .expect("valid legacy credential must seed");
    pool.close().await;

    let ready = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect("canonical 0035 must upgrade through the ready-store gate");
    drop(ready);

    let pool = raw_pool(&path).await;
    let state: String =
        sqlx::query_scalar("SELECT record_state FROM credentials WHERE owner_id = 'owner-upgrade'")
            .fetch_one(&pool)
            .await
            .expect("upgraded credential must be readable");
    assert_eq!(state, "live");
    pool.close().await;
}

#[tokio::test]
async fn unledgered_nonempty_file_is_rejected_without_mutation() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("unledgered.sqlite");
    let pool = raw_pool(&path).await;
    sqlx::query("CREATE TABLE unrelated (value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("unrelated relation must seed");
    sqlx::query("INSERT INTO unrelated (value) VALUES ('preserve')")
        .execute(&pool)
        .await
        .expect("unrelated row must seed");
    pool.close().await;

    let bytes_before = std::fs::read(&path).expect("fixture database bytes must be readable");
    let sidecars_before = sidecar_membership(&path);
    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("unledgered non-empty database must fail closed");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::UnledgeredDatabase
    ));
    assert_eq!(
        std::fs::read(&path).expect("rejected database bytes must remain readable"),
        bytes_before
    );
    assert_eq!(sidecar_membership(&path), sidecars_before);
}

#[tokio::test]
async fn release_rejection_matrix_preserves_file_bytes_and_sidecars() {
    for case in [
        "empty-ledger",
        "below-floor",
        "invalid-ledger",
        "missing-credentials",
        "checksum",
        "description",
        "gap",
        "future",
        "reserved",
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
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join(format!("{case}.sqlite"));
        let pool = raw_pool(&path).await;
        if case == "below-floor" {
            MIGRATOR
                .run_to(29, &pool)
                .await
                .expect("pre-floor canonical prefix must install");
        } else {
            MIGRATOR
                .run_to(35, &pool)
                .await
                .expect("canonical legacy prefix must install");
            match case {
                "empty-ledger" => {
                    sqlx::query("DELETE FROM _sqlx_migrations")
                        .execute(&pool)
                        .await
                        .expect("empty ledger fixture must remove canonical rows");
                },
                "invalid-ledger" => {
                    rewrite_schema_sql(
                        &pool,
                        "table",
                        "_sqlx_migrations",
                        "DEFAULT CURRENT_TIMESTAMP",
                        "DEFAULT '1970-01-01 00:00:00'",
                    )
                    .await;
                },
                "missing-credentials" => {
                    sqlx::query("DROP TABLE credentials")
                        .execute(&pool)
                        .await
                        .expect("missing relation fixture must drop credentials");
                },
                "checksum" => {
                    sqlx::query("UPDATE _sqlx_migrations SET checksum = x'00' WHERE version = 35")
                        .execute(&pool)
                        .await
                        .expect("checksum fixture must drift");
                },
                "description" => {
                    sqlx::query(
                        "UPDATE _sqlx_migrations
                         SET description = 'drifted'
                         WHERE version = 35",
                    )
                    .execute(&pool)
                    .await
                    .expect("description fixture must drift");
                },
                "gap" => {
                    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 34")
                        .execute(&pool)
                        .await
                        .expect("gap fixture must remove one canonical row");
                },
                "future" => {
                    sqlx::query(
                        "INSERT INTO _sqlx_migrations (
                             version, description, success, checksum, execution_time
                         ) VALUES (999, 'future', 1, x'00', 0)",
                    )
                    .execute(&pool)
                    .await
                    .expect("future ledger fixture must seed");
                },
                "reserved" => {
                    sqlx::query(
                        "INSERT INTO _sqlx_migrations (
                             version, description, success, checksum, execution_time
                         ) VALUES (29, 'postgres-reserved', 1, x'00', 0)",
                    )
                    .execute(&pool)
                    .await
                    .expect("reserved ledger fixture must seed");
                },
                "dirty" => {
                    sqlx::query("UPDATE _sqlx_migrations SET success = 0 WHERE version = 35")
                        .execute(&pool)
                        .await
                        .expect("dirty ledger fixture must seed");
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
                    .await;
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
                    .await;
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
                    .await;
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
                    .await;
                },
                "malformed-json" => insert_legacy_metadata(&pool, "{not-json").await,
                "metadata-not-object" => insert_legacy_metadata(&pool, "[]").await,
                "recursive-duplicate-key" => {
                    insert_legacy_metadata(&pool, r#"{"nested":{"key":1,"key":2}}"#).await;
                },
                "invalid-display" => {
                    insert_legacy_metadata(&pool, r#"{"display":{"description":7}}"#).await;
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
                    .await;
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
                    .await;
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
                        .await;
                    }
                },
                _ => unreachable!("closed release-rejection fixture set"),
            }
        }
        pool.close().await;

        let error = reject_file_unchanged(&path).await;
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
                        CredentialSchemaAdmissionReason::ChecksumMismatch { migration: 35 }
                    )
            )),
            "description" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::DescriptionMismatch { migration: 35 }
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
            "reserved" => assert!(matches!(
                error,
                CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                    if matches!(
                        unsupported.reason(),
                        CredentialSchemaAdmissionReason::ReservedForOtherBackend { migration: 29 }
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
                        CredentialSchemaAdmissionReason::FailedMigration { migration: 35 }
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
    }
}

#[tokio::test]
async fn ownerless_legacy_file_is_rejected_before_migration_without_mutation() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("ownerless.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run_to(35, &pool)
        .await
        .expect("canonical legacy prefix must install");
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, created_at, updated_at, expires_at,
             reauth_required, metadata
         ) VALUES (?, NULL, NULL, ?, ?, 0, ?, 1, ?, ?, NULL, 0, '{}')",
    )
    .bind(CredentialId::new().to_string())
    .bind("provider.ownerless")
    .bind("ready")
    .bind(Vec::<u8>::new())
    .bind(1_700_000_000_000_i64)
    .bind(1_700_000_000_001_i64)
    .execute(&pool)
    .await
    .expect("ownerless legacy credential must seed");
    pool.close().await;

    let bytes_before = std::fs::read(&path).expect("fixture database bytes must be readable");
    let sidecars_before = sidecar_membership(&path);
    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("ownerless legacy database must fail before migration");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::OwnerlessCredential
    ));
    assert_eq!(
        std::fs::read(&path).expect("rejected database bytes must remain readable"),
        bytes_before
    );
    assert_eq!(sidecar_membership(&path), sidecars_before);
}

#[tokio::test]
async fn legacy_storage_class_and_boolean_drift_are_rejected_before_migration() {
    for (file_name, insert) in [
        (
            "legacy-text-data.sqlite",
            "INSERT INTO credentials (
                 id, name, owner_id, credential_key, state_kind, state_version,
                 data, version, created_at, updated_at, expires_at,
                 reauth_required, metadata
             ) VALUES (?, NULL, ?, ?, ?, 1, 'text-not-blob', 1, ?, ?, NULL, 0, '{}')",
        ),
        (
            "legacy-invalid-boolean.sqlite",
            "INSERT INTO credentials (
                 id, name, owner_id, credential_key, state_kind, state_version,
                 data, version, created_at, updated_at, expires_at,
                 reauth_required, metadata
             ) VALUES (?, NULL, ?, ?, ?, 1, zeroblob(0), 1, ?, ?, NULL, 2, '{}')",
        ),
    ] {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join(file_name);
        let pool = raw_pool(&path).await;
        MIGRATOR
            .run_to(35, &pool)
            .await
            .expect("canonical legacy prefix must install");
        sqlx::query(insert)
            .bind(CredentialId::new().to_string())
            .bind("owner-storage-class")
            .bind("provider.drift")
            .bind("ready")
            .bind(1_700_000_000_000_i64)
            .bind(1_700_000_000_001_i64)
            .execute(&pool)
            .await
            .expect("SQLite affinity must accept the intentionally drifted row");
        pool.close().await;

        let bytes_before = std::fs::read(&path).expect("fixture bytes must be readable");
        let sidecars_before = sidecar_membership(&path);
        assert_invalid_current_shape(&path).await;
        assert_eq!(
            std::fs::read(&path).expect("rejected database bytes must remain readable"),
            bytes_before
        );
        assert_eq!(sidecar_membership(&path), sidecars_before);
    }
}

#[tokio::test]
async fn forged_current_shape_is_rejected_before_writable_open() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("forged-current.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    sqlx::query("DROP INDEX idx_credentials_owner_name")
        .execute(&pool)
        .await
        .expect("fixture must remove one required structural guard");
    pool.close().await;

    let bytes_before = std::fs::read(&path).expect("fixture database bytes must be readable");
    let sidecars_before = sidecar_membership(&path);
    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("a ledger claim cannot substitute for the final schema");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidCredentialsRelation
    ));
    assert_eq!(
        std::fs::read(&path).expect("rejected database bytes must remain readable"),
        bytes_before
    );
    assert_eq!(sidecar_membership(&path), sidecars_before);
}

#[tokio::test]
async fn current_schema_without_claim_incident_uniqueness_is_rejected_read_only() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("missing-claim-incident-index.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    sqlx::query("DROP INDEX idx_credential_sentinel_events_claim_id")
        .execute(&pool)
        .await
        .expect("fixture must remove incident uniqueness");
    pool.close().await;

    let bytes_before = std::fs::read(&path).expect("fixture database bytes must be readable");
    let sidecars_before = sidecar_membership(&path);
    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("a current ledger cannot substitute for incident uniqueness");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidSentinelEventsRelation
    ));
    assert_eq!(
        std::fs::read(&path).expect("rejected database bytes must remain readable"),
        bytes_before
    );
    assert_eq!(sidecar_membership(&path), sidecars_before);
}

#[tokio::test]
async fn named_dummy_check_cannot_impersonate_the_canonical_constraint() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("dummy-check.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    rewrite_schema_sql(
        &pool,
        "table",
        "credentials",
        "typeof(state_version) = 'integer'\n            AND state_version BETWEEN 0 AND 4294967295",
        "1",
    )
    .await;
    pool.close().await;

    assert_invalid_current_shape(&path).await;
}

#[tokio::test]
async fn canonical_check_text_inside_a_comment_cannot_spoof_constraints() {
    const COMMENT_SPOOFED_TABLE: &str = r#"
        CREATE TABLE "credentials" (
            id TEXT NOT NULL PRIMARY KEY,
            name TEXT,
            owner_id TEXT NOT NULL,
            credential_key TEXT NOT NULL,
            state_kind TEXT NOT NULL,
            state_version INTEGER NOT NULL,
            data BLOB NOT NULL,
            version INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            expires_at INTEGER,
            reauth_required INTEGER NOT NULL,
            metadata TEXT NOT NULL,
            record_state TEXT NOT NULL,
            tombstoned_at INTEGER
            /*
            , CONSTRAINT credentials_state_version_range
              CHECK (
                  typeof(state_version) = 'integer'
                  AND state_version BETWEEN 0 AND 4294967295
              )
            , CONSTRAINT credentials_version_range
              CHECK (
                  typeof(version) = 'integer'
                  AND version BETWEEN 1 AND 9223372036854775807
              )
            , CONSTRAINT credentials_reauth_boolean
              CHECK (
                  typeof(reauth_required) = 'integer'
                  AND reauth_required IN (0, 1)
              )
            , CONSTRAINT credentials_data_blob
              CHECK (typeof(data) = 'blob')
            , CONSTRAINT credentials_metadata_object
              CHECK (
                  typeof(metadata) = 'text'
                  AND json_valid(metadata)
                  AND json_type(metadata) = 'object'
              )
            , CONSTRAINT credentials_live_name_projection
              CHECK (
                  record_state = 'tombstoned'
                  OR (
                      record_state = 'live'
                      AND (
                          (
                              name IS NULL
                              AND (
                                  json_type(metadata, '$.display.display_name') IS NULL
                                  OR json_type(metadata, '$.display.display_name') = 'null'
                              )
                          )
                          OR (
                              name IS NOT NULL
                              AND json_type(metadata, '$.display.display_name') = 'text'
                              AND name = json_extract(metadata, '$.display.display_name')
                          )
                      )
                  )
              )
            , CONSTRAINT credentials_record_shape
              CHECK (
                  (
                      record_state = 'live'
                      AND tombstoned_at IS NULL
                      AND version <= 9223372036854775806
                  )
                  OR
                  (
                      record_state = 'tombstoned'
                      AND tombstoned_at IS NOT NULL
                      AND length(data) = 0
                      AND name IS NULL
                      AND expires_at IS NULL
                      AND reauth_required = 0
                      AND metadata = '{}'
                  )
              )
            */
        )
    "#;

    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("comment-spoof.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    sqlx::query("DROP TABLE credentials")
        .execute(&pool)
        .await
        .expect("fixture must remove the canonical constrained table");
    sqlx::query(COMMENT_SPOOFED_TABLE)
        .execute(&pool)
        .await
        .expect("comment-spoofed unconstrained table must parse");
    sqlx::query("CREATE UNIQUE INDEX idx_credentials_owner_name ON credentials(owner_id, name)")
        .execute(&pool)
        .await
        .expect("canonical owner-name index must be restored");
    sqlx::query("CREATE INDEX idx_credentials_state_kind ON credentials(state_kind)")
        .execute(&pool)
        .await
        .expect("canonical state index must be restored");
    sqlx::query(
        "CREATE INDEX idx_credentials_expiring ON credentials(expires_at)
         WHERE expires_at IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("canonical expiry index must be restored");
    pool.close().await;

    assert_invalid_current_shape(&path).await;
}

#[tokio::test]
async fn wrong_current_column_type_and_default_are_rejected() {
    for (file_name, needle, replacement) in [
        (
            "wrong-type.sqlite",
            "owner_id                      TEXT    NOT NULL",
            "owner_id                      BLOB    NOT NULL",
        ),
        (
            "wrong-default.sqlite",
            "metadata                      TEXT    NOT NULL,",
            "metadata                      TEXT    NOT NULL DEFAULT '{}',",
        ),
    ] {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join(file_name);
        let pool = raw_pool(&path).await;
        MIGRATOR
            .run(&pool)
            .await
            .expect("canonical current schema must install");
        rewrite_schema_sql(&pool, "table", "credentials", needle, replacement).await;
        pool.close().await;

        assert_invalid_current_shape(&path).await;
    }
}

#[tokio::test]
async fn refresh_retry_check_and_admission_reject_unknown_codes() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("unknown-refresh-retry-code.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    let credential_id = CredentialId::new().to_string();
    sqlx::query(
        "INSERT INTO credentials (
             id, name, owner_id, credential_key, state_kind, state_version,
             data, version, material_epoch, created_at, updated_at, expires_at,
             reauth_required, metadata, record_state, tombstoned_at
         ) VALUES (?1, NULL, 'owner-a', 'provider.token', 'active', 1,
                   zeroblob(0), 1, 1, 1700000000000, 1700000000000, NULL,
                   0, '{}', 'live', NULL)",
    )
    .bind(&credential_id)
    .execute(&pool)
    .await
    .expect("valid live fixture must seed");

    let rejected = sqlx::query(
        "UPDATE credentials
         SET refresh_retry_mode = 'never',
             refresh_retry_phase = 'future_phase',
             refresh_retry_kind = 'protocol_error'
         WHERE id = ?1",
    )
    .bind(&credential_id)
    .execute(&pool)
    .await;
    assert!(
        rejected.is_err(),
        "the physical CHECK must reject an unknown phase"
    );

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test fixture must bypass checks deliberately");
    sqlx::query(
        "UPDATE credentials
         SET refresh_retry_mode = 'never',
             refresh_retry_phase = 'future_phase',
             refresh_retry_kind = 'protocol_error'
         WHERE id = ?1",
    )
    .bind(&credential_id)
    .execute(&pool)
    .await
    .expect("corrupt fixture must be representable with checks bypassed");
    pool.close().await;

    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("unknown persisted retry evidence must fail readiness");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::InvalidRefreshRetryGate
    ));
}

#[tokio::test]
async fn wrong_migration_ledger_default_is_rejected() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("wrong-ledger-default.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    rewrite_schema_sql(
        &pool,
        "table",
        "_sqlx_migrations",
        "DEFAULT CURRENT_TIMESTAMP",
        "DEFAULT '1970-01-01 00:00:00'",
    )
    .await;
    pool.close().await;

    let error = SqliteCredentialPersistence::connect(&file_url(&path))
        .await
        .expect_err("migration ledger column drift must fail closed");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason() == &CredentialSchemaAdmissionReason::InvalidMigrationLedger
    ));
}

#[tokio::test]
async fn wrong_expiring_index_predicate_is_rejected() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("wrong-index-predicate.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    rewrite_schema_sql(
        &pool,
        "index",
        "idx_credentials_expiring",
        "WHERE expires_at IS NOT NULL",
        "WHERE expires_at IS NULL",
    )
    .await;
    pool.close().await;

    assert_invalid_current_shape(&path).await;
}

#[tokio::test]
async fn extra_index_is_rejected_by_exact_inventory() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("extra-index.sqlite");
    let pool = raw_pool(&path).await;
    MIGRATOR
        .run(&pool)
        .await
        .expect("canonical current schema must install");
    sqlx::query("CREATE INDEX idx_credentials_extra ON credentials(updated_at)")
        .execute(&pool)
        .await
        .expect("fixture extra index must install");
    pool.close().await;

    assert_invalid_current_shape(&path).await;
}
