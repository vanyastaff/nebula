//! PostgreSQL acceptance evidence for the plan/flavor revision catalog and the
//! execution revision references at the migration head.

#![cfg(feature = "postgres")]

use std::{
    error::Error,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};

#[path = "support/canonical_head.rs"]
mod canonical_head;

#[path = "support/execution_parents.rs"]
mod execution_parents;

use execution_parents::SeedExecutionParents;
use nebula_storage_port::Scope;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const LIVE_EXECUTION_ID: &str = "exe_01JAZ000000000000000000001";
const ROLLBACK_EXECUTION_ID: &str = "exe_01JAZ000000000000000000002";
const RELEASED_LIVE_EXECUTION_ID: &str = "exe_01JAZ000000000000000000003";
const RELEASED_ROLLBACK_EXECUTION_ID: &str = "exe_01JAZ000000000000000000004";
const MISSING_EXECUTION_ID: &str = "exe_01JAZ000000000000000000099";

fn revision_id(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

fn opaque_id(byte: u8) -> Vec<u8> {
    vec![byte; 16]
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
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create isolated revision-catalog schema");

        let options = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect to isolated revision-catalog schema");

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
        .expect("drop isolated revision-catalog schema");
        self.admin.close().await;
    }
}

fn unique_schema_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("nebula_revision_catalog_{}_{nanos}", std::process::id())
}

async fn seed_execution(pool: &PgPool, execution_id: &str, marker: &str) -> TestResult<()> {
    pool.seed_execution_parents(&Scope::new("workspace-a", "org-a"), "workflow-a")
        .await;
    sqlx::query(
        "INSERT INTO executions (
             id, workspace_id, org_id, workflow_id, status, state,
             version, created_at, updated_at
         ) VALUES (
             $1, 'workspace-a', 'org-a', 'workflow-a', 'created', $2::jsonb,
             7, '2026-07-27T00:00:00Z'::timestamptz,
             '2026-07-27T00:00:01Z'::timestamptz
         )",
    )
    .bind(execution_id)
    .bind(format!(r#"{{"marker":"{marker}"}}"#))
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_worker_flavor(
    pool: &PgPool,
    worker_flavor_id: Vec<u8>,
    record_format: &str,
    lifecycle: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO worker_flavor_revisions (
             worker_flavor_id, record_format, lifecycle, record_bytes
         ) VALUES ($1, $2, $3, $4)",
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
    pool: &PgPool,
    executable_plan_id: Vec<u8>,
    worker_flavor_id: Vec<u8>,
    record_format: &str,
    lifecycle: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO executable_plan_revisions (
             executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes
         ) VALUES ($1, $2, $3, $4, $5)",
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
    pool: &PgPool,
    execution_id: &str,
    execution_contract_bundle_id: Vec<u8>,
    executable_plan_id: Vec<u8>,
    worker_flavor_id: Vec<u8>,
    reference_state: &str,
    rollback_window_id: Option<Vec<u8>>,
    retain_until_us: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO execution_revision_references (
             org_id, workspace_id, execution_id, execution_contract_bundle_id,
             executable_plan_id, worker_flavor_id, reference_state, rollback_window_id,
             retain_until
         ) VALUES ('org-a', 'workspace-a', $1, $2, $3, $4, $5, $6,
                   to_timestamp($7::bigint / 1000000.0))",
    )
    .bind(execution_id)
    .bind(execution_contract_bundle_id)
    .bind(executable_plan_id)
    .bind(worker_flavor_id)
    .bind(reference_state)
    .bind(rollback_window_id)
    .bind(retain_until_us)
    .execute(pool)
    .await?;
    Ok(())
}

async fn assert_closed_catalog_constraints(pool: &PgPool) -> TestResult<()> {
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
         FROM executable_plan_revisions
         WHERE executable_plan_id = $1",
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
    ] {
        seed_execution(pool, execution_id, execution_id).await?;
    }

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

    for (reference_state, rollback_window_id, retain_until_us, reason) in [
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
                retain_until_us,
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
        "SELECT rollback_window_id,
                (extract(epoch FROM retain_until) * 1000000)::bigint
         FROM execution_revision_references
         WHERE execution_id = $1",
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
         FROM execution_revision_references
         WHERE reference_state = 'rollback'
           AND to_timestamp($1::bigint / 1000000.0) < retain_until",
    )
    .bind(999_i64)
    .fetch_one(pool)
    .await?;
    let blocks_at_expiry: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM execution_revision_references
         WHERE reference_state = 'rollback'
           AND to_timestamp($1::bigint / 1000000.0) < retain_until",
    )
    .bind(1_000_i64)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        (blocks_before_expiry, blocks_at_expiry),
        (1, 0),
        "the retention boundary is blocking before, but expired at, equality"
    );

    assert!(
        sqlx::query("DELETE FROM executable_plan_revisions WHERE executable_plan_id = $1",)
            .bind(revision_id(0x31))
            .execute(pool)
            .await
            .is_err(),
        "plan deletion is restrictive while exact revision references exist"
    );
    assert!(
        sqlx::query("DELETE FROM worker_flavor_revisions WHERE worker_flavor_id = $1",)
            .bind(revision_id(0x11))
            .execute(pool)
            .await
            .is_err(),
        "flavor deletion is restrictive while plans depend on it"
    );
    // The reference belongs to its execution and is purged with it.
    sqlx::query("DELETE FROM executions WHERE id = $1")
        .bind(LIVE_EXECUTION_ID)
        .execute(pool)
        .await?;
    let orphaned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM execution_revision_references WHERE execution_id = $1",
    )
    .bind(LIVE_EXECUTION_ID)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        orphaned, 0,
        "an execution's revision reference cascades with it"
    );

    let index_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT indexname, indexdef
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND indexname LIKE 'ix_%revision%'
         ORDER BY indexname COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await?;
    let index_names = index_rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let declared = [
        (
            "ix_executable_plan_revisions__worker_flavor_id__undeleted",
            None,
        ),
        (
            "ix_execution_revision_references__executable_plan__rollback",
            Some(false),
        ),
        (
            "ix_execution_revision_references__executable_plan_id__live",
            Some(true),
        ),
        (
            "ix_execution_revision_references__worker_flavor__rollback",
            Some(false),
        ),
        (
            "ix_execution_revision_references__worker_flavor_execution__live",
            Some(true),
        ),
    ];
    assert_eq!(
        index_names,
        declared.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        "the catalog carries only the five blocker/dependency indexes"
    );
    for ((index_name, definition), (_, live)) in index_rows.into_iter().zip(declared) {
        let expected_predicate = if live.is_none() {
            "WHERE (lifecycle <> 'deleted'::text)"
        } else if live == Some(true) {
            "WHERE (reference_state = 'live'::text)"
        } else {
            "WHERE (reference_state = 'rollback'::text)"
        };
        assert!(
            definition.contains(expected_predicate),
            "{index_name} must remain a partial index with predicate `{expected_predicate}`"
        );
    }

    Ok(())
}

#[tokio::test]
async fn clean_catalog_reaches_head_and_enforces_closed_shapes() -> TestResult<()> {
    let Some(database) = IsolatedDatabase::connect().await else {
        panic!(
            "clean_catalog_reaches_head_and_enforces_closed_shapes: backend unreachable — the \
             case cannot run and must fail rather than pass unchecked; reach the backend (set \
             DATABASE_URL for postgres) or run without this feature"
        );
    };

    let result = async {
        MIGRATOR.run(&database.pool).await?;

        let head: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0)
             FROM _sqlx_migrations
             WHERE success = true",
        )
        .fetch_one(&database.pool)
        .await?;
        assert_eq!(
            head,
            canonical_head::of(&MIGRATOR),
            "a clean database must reach the canonical catalog head"
        );

        assert_closed_catalog_constraints(&database.pool).await
    }
    .await;

    database.cleanup().await;
    result
}
