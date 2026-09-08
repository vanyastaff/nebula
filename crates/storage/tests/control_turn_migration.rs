//! The accepted-turn audit survives the additive command-source extension.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn resume_and_restart_are_valid_acceptance_sources() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    // Disable only the parent FK here: this is a schema CHECK oracle, not an
    // execution ownership fixture. Runtime conformance uses real parents.
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    for source in ["ControlResume", "ControlRestart"] {
        sqlx::query("INSERT INTO port_execution_turn_acceptances (execution_id,workspace_id,org_id,last_accepted_fencing_generation,source_kind,source_queue_id) VALUES (?, 'workspace', 'org', 1, ?, zeroblob(16))")
            .bind(source).bind(source).execute(&pool).await.unwrap();
    }
}
