//! Live revision records require artifact bytes at the admitted schema seam.

#[cfg(feature = "postgres")]
mod support {
    pub(crate) mod postgres_schema;
}

#[cfg(feature = "postgres")]
use support::postgres_schema;

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_live_revision_records_cannot_have_null_bytes() {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL is required for PostgreSQL revision evidence");
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_revision_shape")
        .await
        .expect("open isolated PostgreSQL schema");
    nebula_storage::postgres::init_schema(&pool)
        .await
        .expect("admit deployment schema");

    let flavor = vec![1_u8; 32];
    sqlx::query(
        "INSERT INTO worker_flavor_revisions \
         (worker_flavor_id, record_format, lifecycle, record_bytes) \
         VALUES ($1, 'v1_json', 'active', $2)",
    )
    .bind(&flavor)
    .bind(vec![1_u8])
    .execute(&pool)
    .await
    .expect("nonempty artifact is a valid live flavor");
    for (id, lifecycle, bytes) in [(8_u8, "active", Some(vec![1_u8])), (9_u8, "deleted", None)] {
        sqlx::query(
            "INSERT INTO executable_plan_revisions \
             (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES ($1, $2, 'graph_v1_json', $3, $4)",
        )
        .bind(vec![id; 32])
        .bind(&flavor)
        .bind(lifecycle)
        .bind(bytes)
        .execute(&pool)
        .await
        .expect("live plan bytes and deleted plan tombstones are valid");
    }

    let mut rejected = Vec::new();
    for (index, lifecycle) in ["active", "draining"].into_iter().enumerate() {
        let missing_flavor = sqlx::query(
            "INSERT INTO worker_flavor_revisions \
             (worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES ($1, 'v1_json', $2, NULL)",
        )
        .bind(vec![
            u8::try_from(index + 2).expect("fixture id fits u8");
            32
        ])
        .bind(lifecycle)
        .execute(&pool)
        .await;
        let missing_plan = sqlx::query(
            "INSERT INTO executable_plan_revisions \
             (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES ($1, $2, 'graph_v1_json', $3, NULL)",
        )
        .bind(vec![
            u8::try_from(index + 4).expect("fixture id fits u8");
            32
        ])
        .bind(&flavor)
        .bind(lifecycle)
        .execute(&pool)
        .await;
        rejected.push((missing_flavor.is_err(), missing_plan.is_err()));
    }

    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .expect("private schema name");
    assert!(
        schema
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&pool)
        .await
        .expect("remove revision test schema");
    pool.close().await;
    assert_eq!(
        rejected,
        vec![(true, true), (true, true)],
        "active and draining flavors and plans require nonempty artifact bytes"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_live_revision_records_cannot_have_null_bytes() {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .expect("admit SQLite deployment schema");
    let flavor = vec![1_u8; 32];
    sqlx::query(
        "INSERT INTO worker_flavor_revisions \
         (worker_flavor_id, record_format, lifecycle, record_bytes) \
         VALUES (?, 'v1_json', 'active', ?)",
    )
    .bind(&flavor)
    .bind(vec![1_u8])
    .execute(&pool)
    .await
    .expect("nonempty artifact is a valid live flavor");
    for (id, lifecycle, bytes) in [(8_u8, "active", Some(vec![1_u8])), (9_u8, "deleted", None)] {
        sqlx::query(
            "INSERT INTO executable_plan_revisions \
             (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES (?, ?, 'graph_v1_json', ?, ?)",
        )
        .bind(vec![id; 32])
        .bind(&flavor)
        .bind(lifecycle)
        .bind(bytes)
        .execute(&pool)
        .await
        .expect("live plan bytes and deleted plan tombstones are valid");
    }
    for lifecycle in ["active", "draining"] {
        assert!(
            sqlx::query(
                "INSERT INTO worker_flavor_revisions \
             (worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES (?, 'v1_json', ?, NULL)",
            )
            .bind(vec![2_u8; 32])
            .bind(lifecycle)
            .execute(&pool)
            .await
            .is_err()
        );
        assert!(
            sqlx::query(
                "INSERT INTO executable_plan_revisions \
             (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES (?, ?, 'graph_v1_json', ?, NULL)",
            )
            .bind(vec![3_u8; 32])
            .bind(&flavor)
            .bind(lifecycle)
            .execute(&pool)
            .await
            .is_err()
        );
    }
    pool.close().await;
}
