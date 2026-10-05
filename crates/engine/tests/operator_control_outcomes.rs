//! Actual engine decision inventory: durable owner journal plus independent telemetry.

#[path = "support/operators/mod.rs"]
mod operators;

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn in_memory_operator_control_outcomes() {
    let report = operators::in_memory().await;
    operators::write_report(
        "NEBULA_OPERATOR_CONTROL_IN_MEMORY_OBSERVATIONS_PATH",
        &report,
    );
}

#[tokio::test]
async fn sqlite_operator_control_outcomes() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let report = operators::sqlite(pool.clone()).await;
    operators::write_report("NEBULA_OPERATOR_CONTROL_SQLITE_OBSERVATIONS_PATH", &report);
    pool.close().await;
}

// PostgreSQL evidence runs only when explicitly requested: the report must never
// be emitted by a test that skipped connecting to its deployment backend.
#[tokio::test]
#[ignore = "requires a live PostgreSQL and its admitted isolated schema"]
async fn postgres_operator_control_outcomes() {
    let url =
        std::env::var("DATABASE_URL").expect("PostgreSQL outcome evidence requires DATABASE_URL");
    let pool = postgres_schema::connect_with_private_schema(&url, "operator_control")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let report = operators::postgres(pool.clone()).await;
    operators::write_report(
        "NEBULA_OPERATOR_CONTROL_POSTGRES_OBSERVATIONS_PATH",
        &report,
    );
    pool.close().await;
}
