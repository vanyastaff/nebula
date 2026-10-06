//! Migration 0064 backfills the execution listing projection from each row's
//! own state, on both SQL backends.
//!
//! Rows written before 0064 carry the insert-time status `'Created'` no
//! matter how far the execution got; the real status and lifecycle
//! timestamps live only in the state JSON. The migration must project them
//! into the new columns so history filters see the truth for old rows too.

use serde_json::json;

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

/// A row written before 0064 and the projection the backfill must give it.
struct LegacyRow {
    id: &'static str,
    state: serde_json::Value,
    status: &'static str,
    started_at: Option<&'static str>,
    finished_at: Option<&'static str>,
}

fn legacy_rows() -> Vec<LegacyRow> {
    vec![
        LegacyRow {
            id: "exe_failed",
            state: json!({"status": "failed", "started_at": "2026-10-01T10:00:00.250Z", "completed_at": "2026-10-01T10:05:00.500Z"}),
            status: "failed",
            started_at: Some("2026-10-01T10:00:00.250Z"),
            finished_at: Some("2026-10-01T10:05:00.500Z"),
        },
        LegacyRow {
            id: "exe_running",
            // `completed_at` on a non-terminal snapshot never becomes finished_at.
            state: json!({"status": "running", "started_at": "2026-10-01T11:00:00Z", "completed_at": "2026-10-01T11:00:01Z"}),
            status: "running",
            started_at: Some("2026-10-01T11:00:00Z"),
            finished_at: None,
        },
        LegacyRow {
            id: "exe_unknown",
            state: json!({"status": "pending"}),
            status: "created",
            started_at: None,
            finished_at: None,
        },
        LegacyRow {
            id: "exe_bare",
            state: json!({}),
            status: "created",
            started_at: None,
            finished_at: None,
        },
        LegacyRow {
            id: "exe_garbled",
            // Not RFC 3339: left NULL rather than aborting the upgrade.
            state: json!({"status": "completed", "started_at": "", "completed_at": "yesterday"}),
            status: "completed",
            started_at: None,
            finished_at: None,
        },
    ]
}

fn instant(raw: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .expect("fixture timestamp")
        .with_timezone(&chrono::Utc)
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_0064_backfills_listing_from_state() {
    use sqlx::Row;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite");
    MIGRATOR.run_to(63, &pool).await.expect("migrate to 0063");
    for LegacyRow { id, state, .. } in legacy_rows() {
        sqlx::query(
            "INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, \
             version, fencing_generation, created_at, updated_at) \
             VALUES (?, 'ws', 'org', 'wf', 'Created', ?, 3, 0, \
                     '2026-10-01T09:00:00.123456789+00:00', '2026-10-01T09:00:00+00:00')",
        )
        .bind(id)
        .bind(state.to_string())
        .execute(&pool)
        .await
        .expect("legacy row");
    }
    MIGRATOR.run(&pool).await.expect("migrate to head");

    for LegacyRow {
        id,
        status,
        started_at: started,
        finished_at: finished,
        ..
    } in legacy_rows()
    {
        let row = sqlx::query(
            "SELECT status, started_at, finished_at, created_at_us, version \
             FROM port_executions WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), status, "{id} status");
        assert_eq!(
            row.get::<Option<String>, _>("started_at")
                .map(|raw| instant(&raw)),
            started.map(instant),
            "{id} started_at"
        );
        assert_eq!(
            row.get::<Option<String>, _>("finished_at")
                .map(|raw| instant(&raw)),
            finished.map(instant),
            "{id} finished_at"
        );
        // SQLite date functions resolve milliseconds for legacy rows.
        assert_eq!(
            row.get::<i64, _>("created_at_us"),
            instant("2026-10-01T09:00:00.123Z").timestamp_micros(),
            "{id} created_at_us"
        );
        assert_eq!(row.get::<i64, _>("version"), 3, "{id} version untouched");
    }
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_0064_backfills_listing_from_state() {
    use sqlx::Row;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
            "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL is not"
        );
        eprintln!("WARN [execution_listing_migration] DATABASE_URL unset; Postgres case skipped");
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_listing_migration")
        .await
        .expect("postgres");
    MIGRATOR.run_to(63, &pool).await.expect("migrate to 0063");
    for LegacyRow { id, state, .. } in legacy_rows() {
        sqlx::query(
            "INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, \
             version, fencing_generation, created_at, updated_at) \
             VALUES ($1, 'ws', 'org', 'wf', 'Created', $2, 3, 0, \
                     '2026-10-01T09:00:00.123456Z', '2026-10-01T09:00:00Z')",
        )
        .bind(id)
        .bind(&state)
        .execute(&pool)
        .await
        .expect("legacy row");
    }
    MIGRATOR.run(&pool).await.expect("migrate to head");

    for LegacyRow {
        id,
        status,
        started_at: started,
        finished_at: finished,
        ..
    } in legacy_rows()
    {
        let row = sqlx::query(
            "SELECT status, started_at, finished_at, created_at_us, version \
             FROM port_executions WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), status, "{id} status");
        assert_eq!(
            row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("started_at"),
            started.map(instant),
            "{id} started_at"
        );
        assert_eq!(
            row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finished_at"),
            finished.map(instant),
            "{id} finished_at"
        );
        assert_eq!(
            row.get::<i64, _>("created_at_us"),
            instant("2026-10-01T09:00:00.123456Z").timestamp_micros(),
            "{id} created_at_us keeps full microseconds"
        );
        assert_eq!(row.get::<i64, _>("version"), 3, "{id} version untouched");
    }

    // The backfill default is gone: a writer predating 0064 fails closed.
    let legacy_insert = sqlx::query(
        "INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, \
         version, fencing_generation, created_at, updated_at) \
         VALUES ('exe_old_writer', 'ws', 'org', 'wf', 'Created', '{}', 0, 0, now(), now())",
    )
    .execute(&pool)
    .await;
    assert!(
        legacy_insert.is_err(),
        "an insert without created_at_us must be rejected"
    );
}
