//! SQLite acceptance evidence for the dormant plan/flavor revision catalog.

#![cfg(feature = "sqlite")]

use std::{error::Error, path::Path, str::FromStr};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const LIVE_EXECUTION_ID: &str = "exe_01JAZ000000000000000000001";
const ROLLBACK_EXECUTION_ID: &str = "exe_01JAZ000000000000000000002";
const RELEASED_LIVE_EXECUTION_ID: &str = "exe_01JAZ000000000000000000003";
const RELEASED_ROLLBACK_EXECUTION_ID: &str = "exe_01JAZ000000000000000000004";
const INVALID_EXECUTION_ID: &str = "exe_81JAZ000000000000000000005";
const LOWERCASE_EXECUTION_ID: &str = "exe_01JaZ000000000000000000006";
const SUBMILLISECOND_EXECUTION_ID: &str = "exe_01JAZ000000000000000000007";
const MISSING_EXECUTION_ID: &str = "exe_01JAZ000000000000000000099";

fn revision_id(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

fn opaque_id(byte: u8) -> Vec<u8> {
    vec![byte; 16]
}

async fn file_pool(path: &Path) -> SqlitePool {
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

async fn seed_execution(pool: &SqlitePool, execution_id: &str, marker: &str) -> TestResult<()> {
    sqlx::query(
        "INSERT INTO port_executions (
             id, workspace_id, org_id, workflow_id, status, state,
             version, created_at, updated_at
         ) VALUES (?, 'workspace-a', 'org-a', 'workflow-a', 'Pending', ?, 7, ?, ?)",
    )
    .bind(execution_id)
    .bind(format!(r#"{{"marker":"{marker}"}}"#))
    .bind("2026-07-27T00:00:00Z")
    .bind("2026-07-27T00:00:01Z")
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_worker_flavor(
    pool: &SqlitePool,
    worker_flavor_id: Vec<u8>,
    record_format: &str,
    lifecycle: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO port_worker_flavor_revisions (
             worker_flavor_id, record_format, lifecycle, record_bytes
         ) VALUES (?, ?, ?, ?)",
    )
    .bind(worker_flavor_id)
    .bind(record_format)
    .bind(lifecycle)
    .bind(record_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_executable_plan(
    pool: &SqlitePool,
    executable_plan_id: Vec<u8>,
    worker_flavor_id: Vec<u8>,
    record_format: &str,
    lifecycle: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO port_executable_plan_revisions (
             executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes
         ) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(executable_plan_id)
    .bind(worker_flavor_id)
    .bind(record_format)
    .bind(lifecycle)
    .bind(record_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the helper mirrors the closed persisted reference row without hiding a field"
)]
async fn insert_revision_reference(
    pool: &SqlitePool,
    execution_id: &str,
    execution_contract_bundle_id: Vec<u8>,
    executable_plan_id: Vec<u8>,
    worker_flavor_id: Vec<u8>,
    reference_state: &str,
    rollback_window_id: Option<Vec<u8>>,
    retain_until_ms: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO port_execution_revision_refs (
             execution_id, execution_contract_bundle_id, executable_plan_id,
             worker_flavor_id, reference_state, rollback_window_id, retain_until_ms
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(execution_id)
    .bind(execution_contract_bundle_id)
    .bind(executable_plan_id)
    .bind(worker_flavor_id)
    .bind(reference_state)
    .bind(rollback_window_id)
    .bind(retain_until_ms)
    .execute(pool)
    .await?;
    Ok(())
}

async fn assert_closed_catalog_constraints(pool: &SqlitePool) -> TestResult<()> {
    let foreign_keys_enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    assert_eq!(
        foreign_keys_enabled, 1,
        "the exact plan/flavor and execution relations require SQLite foreign keys"
    );

    for (worker_flavor_id, record_format, lifecycle, record_bytes, reason) in [
        (
            vec![0x10; 31],
            "v1_json",
            "active",
            Some(vec![1]),
            "worker-flavor ids are exactly 32 bytes",
        ),
        (
            revision_id(0x10),
            "json",
            "active",
            Some(vec![1]),
            "worker-flavor formats are closed",
        ),
        (
            revision_id(0x10),
            "v1_json",
            "paused",
            Some(vec![1]),
            "worker-flavor lifecycle is closed",
        ),
        (
            revision_id(0x10),
            "v1_json",
            "active",
            Some(Vec::new()),
            "active worker flavors carry non-empty bytes",
        ),
        (
            revision_id(0x10),
            "v1_json",
            "deleted",
            Some(vec![1]),
            "deleted worker flavors clear their bytes",
        ),
    ] {
        assert!(
            insert_worker_flavor(
                pool,
                worker_flavor_id,
                record_format,
                lifecycle,
                record_bytes
            )
            .await
            .is_err(),
            "{reason}"
        );
    }

    insert_worker_flavor(
        pool,
        revision_id(0x11),
        "v1_json",
        "active",
        Some(vec![0xa1]),
    )
    .await?;
    insert_worker_flavor(pool, revision_id(0x22), "v1_json", "deleted", None).await?;

    for (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes, reason) in [
        (
            vec![0x30; 31],
            revision_id(0x11),
            "graph_v1_json",
            "active",
            Some(vec![1]),
            "executable-plan ids are exactly 32 bytes",
        ),
        (
            revision_id(0x30),
            revision_id(0x99),
            "graph_v1_json",
            "active",
            Some(vec![1]),
            "plans cannot name an absent worker flavor",
        ),
        (
            revision_id(0x30),
            revision_id(0x11),
            "json",
            "active",
            Some(vec![1]),
            "executable-plan formats are closed",
        ),
        (
            revision_id(0x30),
            revision_id(0x11),
            "graph_v1_json",
            "paused",
            Some(vec![1]),
            "executable-plan lifecycle is closed",
        ),
        (
            revision_id(0x30),
            revision_id(0x11),
            "graph_v1_json",
            "draining",
            Some(Vec::new()),
            "draining executable plans carry non-empty bytes",
        ),
        (
            revision_id(0x30),
            revision_id(0x11),
            "graph_v1_json",
            "deleted",
            Some(vec![1]),
            "deleted executable plans clear their bytes",
        ),
    ] {
        assert!(
            insert_executable_plan(
                pool,
                executable_plan_id,
                worker_flavor_id,
                record_format,
                lifecycle,
                record_bytes,
            )
            .await
            .is_err(),
            "{reason}"
        );
    }

    insert_executable_plan(
        pool,
        revision_id(0x31),
        revision_id(0x11),
        "graph_v1_json",
        "active",
        Some(vec![0xb1]),
    )
    .await?;
    insert_executable_plan(
        pool,
        revision_id(0x32),
        revision_id(0x22),
        "graph_v1_json",
        "deleted",
        None,
    )
    .await?;

    let deleted_plan = sqlx::query(
        "SELECT worker_flavor_id, record_format, record_bytes
         FROM port_executable_plan_revisions
         WHERE executable_plan_id = ?",
    )
    .bind(revision_id(0x32))
    .fetch_one(pool)
    .await?;
    assert_eq!(
        deleted_plan.get::<Vec<u8>, _>("worker_flavor_id"),
        revision_id(0x22),
        "a tombstone retains its exact flavor relation"
    );
    assert_eq!(
        deleted_plan.get::<String, _>("record_format"),
        "graph_v1_json",
        "a tombstone retains its closed record format"
    );
    assert_eq!(
        deleted_plan.get::<Option<Vec<u8>>, _>("record_bytes"),
        None,
        "only the opaque payload is cleared on deletion"
    );

    for execution_id in [
        LIVE_EXECUTION_ID,
        ROLLBACK_EXECUTION_ID,
        RELEASED_LIVE_EXECUTION_ID,
        RELEASED_ROLLBACK_EXECUTION_ID,
        INVALID_EXECUTION_ID,
        LOWERCASE_EXECUTION_ID,
        SUBMILLISECOND_EXECUTION_ID,
    ] {
        seed_execution(pool, execution_id, execution_id).await?;
    }

    assert!(
        insert_revision_reference(
            pool,
            INVALID_EXECUTION_ID,
            opaque_id(0x41),
            revision_id(0x31),
            revision_id(0x11),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "execution references accept only canonical execution ids"
    );
    assert!(
        insert_revision_reference(
            pool,
            LOWERCASE_EXECUTION_ID,
            opaque_id(0x41),
            revision_id(0x31),
            revision_id(0x11),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "execution references reject lowercase Crockford characters"
    );
    assert!(
        insert_revision_reference(
            pool,
            MISSING_EXECUTION_ID,
            opaque_id(0x41),
            revision_id(0x31),
            revision_id(0x11),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "an execution reference cannot outlive or invent its execution aggregate"
    );
    assert!(
        insert_revision_reference(
            pool,
            LIVE_EXECUTION_ID,
            vec![0x41; 15],
            revision_id(0x31),
            revision_id(0x11),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "bundle ids are exactly 16 bytes"
    );
    assert!(
        insert_revision_reference(
            pool,
            LIVE_EXECUTION_ID,
            opaque_id(0x41),
            vec![0x31; 31],
            revision_id(0x11),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "reference plan ids are exactly 32 bytes"
    );
    assert!(
        insert_revision_reference(
            pool,
            LIVE_EXECUTION_ID,
            opaque_id(0x41),
            revision_id(0x31),
            vec![0x11; 31],
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "reference flavor ids are exactly 32 bytes"
    );
    assert!(
        insert_revision_reference(
            pool,
            LIVE_EXECUTION_ID,
            opaque_id(0x41),
            revision_id(0x31),
            revision_id(0x22),
            "live",
            None,
            None,
        )
        .await
        .is_err(),
        "a reference must name the plan's exact worker flavor"
    );

    for (reference_state, rollback_window_id, retain_until_ms, reason) in [
        ("unknown", None, None, "reference state is closed"),
        (
            "live",
            Some(opaque_id(0x51)),
            Some(1_000),
            "live references cannot carry rollback state",
        ),
        (
            "rollback",
            None,
            Some(1_000),
            "rollback references require a window id",
        ),
        (
            "rollback",
            Some(vec![0x51; 15]),
            Some(1_000),
            "rollback window ids are exactly 16 bytes",
        ),
        (
            "rollback",
            Some(opaque_id(0x51)),
            None,
            "rollback references require an expiry",
        ),
        (
            "released",
            Some(opaque_id(0x51)),
            None,
            "released rollback provenance is retained as a complete pair",
        ),
    ] {
        assert!(
            insert_revision_reference(
                pool,
                LIVE_EXECUTION_ID,
                opaque_id(0x41),
                revision_id(0x31),
                revision_id(0x11),
                reference_state,
                rollback_window_id,
                retain_until_ms,
            )
            .await
            .is_err(),
            "{reason}"
        );
    }

    insert_revision_reference(
        pool,
        LIVE_EXECUTION_ID,
        opaque_id(0x41),
        revision_id(0x31),
        revision_id(0x11),
        "live",
        None,
        None,
    )
    .await?;
    insert_revision_reference(
        pool,
        ROLLBACK_EXECUTION_ID,
        opaque_id(0x42),
        revision_id(0x31),
        revision_id(0x11),
        "rollback",
        Some(opaque_id(0x52)),
        Some(1_000),
    )
    .await?;
    insert_revision_reference(
        pool,
        RELEASED_LIVE_EXECUTION_ID,
        opaque_id(0x43),
        revision_id(0x31),
        revision_id(0x11),
        "released",
        None,
        None,
    )
    .await?;
    insert_revision_reference(
        pool,
        RELEASED_ROLLBACK_EXECUTION_ID,
        opaque_id(0x44),
        revision_id(0x31),
        revision_id(0x11),
        "released",
        Some(opaque_id(0x54)),
        Some(2_000),
    )
    .await?;

    let released_rollback = sqlx::query_as::<_, (Vec<u8>, i64)>(
        "SELECT rollback_window_id, retain_until_ms
         FROM port_execution_revision_refs
         WHERE execution_id = ?",
    )
    .bind(RELEASED_ROLLBACK_EXECUTION_ID)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        released_rollback,
        (opaque_id(0x54), 2_000),
        "released rollback references retain their exact rollback provenance"
    );
    assert!(
        insert_revision_reference(
            pool,
            LIVE_EXECUTION_ID,
            opaque_id(0x49),
            revision_id(0x31),
            revision_id(0x11),
            "released",
            None,
            None,
        )
        .await
        .is_err(),
        "each execution owns exactly one immutable revision-reference row"
    );

    let blocks_before_expiry: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM port_execution_revision_refs
         WHERE reference_state = 'rollback'
           AND ? < retain_until_ms",
    )
    .bind(999_i64)
    .fetch_one(pool)
    .await?;
    let blocks_at_expiry: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM port_execution_revision_refs
         WHERE reference_state = 'rollback'
           AND ? < retain_until_ms",
    )
    .bind(1_000_i64)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        (blocks_before_expiry, blocks_at_expiry),
        (1, 0),
        "the integer-millisecond retention boundary is blocking before, but expired at, equality"
    );

    let sub_millisecond_expiry = sqlx::query(
        "INSERT INTO port_execution_revision_refs (
             execution_id, execution_contract_bundle_id, executable_plan_id,
             worker_flavor_id, reference_state, rollback_window_id, retain_until_ms
         ) VALUES (?, ?, ?, ?, 'rollback', ?, ?)",
    )
    .bind(SUBMILLISECOND_EXECUTION_ID)
    .bind(opaque_id(0x45))
    .bind(revision_id(0x31))
    .bind(revision_id(0x11))
    .bind(opaque_id(0x55))
    .bind(1_000.5_f64)
    .execute(pool)
    .await;
    assert!(
        sub_millisecond_expiry.is_err(),
        "SQLite must not silently retain sub-millisecond REAL values in the millisecond column"
    );

    assert!(
        sqlx::query("DELETE FROM port_executions WHERE id = ?")
            .bind(LIVE_EXECUTION_ID)
            .execute(pool)
            .await
            .is_err(),
        "execution aggregate deletion is restrictive while an exact revision reference exists"
    );
    assert!(
        sqlx::query("DELETE FROM port_executable_plan_revisions WHERE executable_plan_id = ?",)
            .bind(revision_id(0x31))
            .execute(pool)
            .await
            .is_err(),
        "plan deletion is restrictive while exact revision references exist"
    );
    assert!(
        sqlx::query("DELETE FROM port_worker_flavor_revisions WHERE worker_flavor_id = ?",)
            .bind(revision_id(0x11))
            .execute(pool)
            .await
            .is_err(),
        "flavor deletion is restrictive while plans depend on it"
    );

    let index_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT name, sql
         FROM sqlite_schema
         WHERE type = 'index'
           AND name LIKE 'idx_port_%revision%'
         ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    let index_names = index_rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        index_names,
        [
            "idx_port_executable_plan_revisions_worker_flavor",
            "idx_port_execution_revision_refs_live_flavor",
            "idx_port_execution_revision_refs_live_plan",
            "idx_port_execution_revision_refs_rollback_flavor",
            "idx_port_execution_revision_refs_rollback_plan",
        ],
        "the dormant catalog carries only the five blocker/dependency indexes"
    );
    for (index_name, definition) in index_rows {
        let expected_predicate = if index_name == "idx_port_executable_plan_revisions_worker_flavor"
        {
            "WHERE lifecycle <> 'deleted'"
        } else if index_name.contains("_live_") {
            "WHERE reference_state = 'live'"
        } else {
            "WHERE reference_state = 'rollback'"
        };
        assert!(
            definition.contains(expected_predicate),
            "{index_name} must remain a partial index with predicate `{expected_predicate}`"
        );
    }

    Ok(())
}

#[tokio::test]
async fn clean_catalog_reaches_0041_and_enforces_closed_shapes() -> TestResult<()> {
    let directory = tempfile::tempdir()?;
    let pool = file_pool(&directory.path().join("revision-catalog-clean.sqlite")).await;

    MIGRATOR.run(&pool).await?;

    let head: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0)
         FROM _sqlx_migrations
         WHERE success = 1",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(head, 41, "a clean database must reach migration 0041");

    assert_closed_catalog_constraints(&pool).await?;
    Ok(())
}

#[tokio::test]
async fn migration_0041_preserves_0040_state_and_is_idempotent() -> TestResult<()> {
    let directory = tempfile::tempdir()?;
    let pool = file_pool(&directory.path().join("revision-catalog-upgrade.sqlite")).await;

    MIGRATOR.run_to(40, &pool).await?;
    let pre_upgrade_head: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        pre_upgrade_head, 40,
        "the fixture must be a real N-1 database"
    );

    seed_execution(&pool, LIVE_EXECUTION_ID, "preserved").await?;
    sqlx::query(
        "INSERT INTO port_execution_journal (execution_id, seq, payload)
         VALUES (?, 1, ?)",
    )
    .bind(LIVE_EXECUTION_ID)
    .bind(r#"{"event":"preserved"}"#)
    .execute(&pool)
    .await?;

    let catalog_tables_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE type = 'table'
           AND name IN (
               'port_worker_flavor_revisions',
               'port_executable_plan_revisions',
               'port_execution_revision_refs'
           )",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        catalog_tables_before, 0,
        "the N-1 fixture must not contain migration 0041 objects"
    );

    let ledger_count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    MIGRATOR.run(&pool).await?;

    let migration = sqlx::query_as::<_, (i64, String, bool, Vec<u8>)>(
        "SELECT version, description, success, checksum
         FROM _sqlx_migrations
         WHERE version = 41",
    )
    .fetch_one(&pool)
    .await?;
    let embedded_checksum = MIGRATOR
        .iter()
        .find(|candidate| candidate.version == 41)
        .expect("the embedded SQLite catalog must contain migration 0041")
        .checksum
        .as_ref();
    assert_eq!(
        migration,
        (
            41,
            "port plan flavor revision catalog".to_owned(),
            true,
            embedded_checksum.to_vec(),
        ),
        "the ledger must record the exact embedded 0041 description and checksum"
    );

    let preserved_execution = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT status, state, version
         FROM port_executions
         WHERE id = ?",
    )
    .bind(LIVE_EXECUTION_ID)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        preserved_execution,
        (
            "Pending".to_owned(),
            r#"{"marker":"preserved"}"#.to_owned(),
            7,
        ),
        "0041 must preserve the existing execution aggregate byte-for-byte"
    );
    let preserved_journal: String = sqlx::query_scalar(
        "SELECT payload
         FROM port_execution_journal
         WHERE execution_id = ? AND seq = 1",
    )
    .bind(LIVE_EXECUTION_ID)
    .fetch_one(&pool)
    .await?;
    assert_eq!(preserved_journal, r#"{"event":"preserved"}"#);

    let schema_before_rerun = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT type, name, sql
             FROM sqlite_schema
             WHERE name LIKE 'port_%revision%'
                OR name LIKE 'idx_port_%revision%'
             ORDER BY type, name",
    )
    .fetch_all(&pool)
    .await?;
    let ledger_count_after_upgrade: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        ledger_count_after_upgrade,
        ledger_count_before + 1,
        "the upgrade must append exactly one migration ledger row"
    );

    MIGRATOR.run(&pool).await?;

    let schema_after_rerun = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT type, name, sql
             FROM sqlite_schema
             WHERE name LIKE 'port_%revision%'
                OR name LIKE 'idx_port_%revision%'
             ORDER BY type, name",
    )
    .fetch_all(&pool)
    .await?;
    let ledger_count_after_rerun: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        schema_after_rerun, schema_before_rerun,
        "a second run must not rewrite the 0041 schema"
    );
    assert_eq!(
        ledger_count_after_rerun, ledger_count_after_upgrade,
        "a second run must not append a duplicate ledger row"
    );

    Ok(())
}
