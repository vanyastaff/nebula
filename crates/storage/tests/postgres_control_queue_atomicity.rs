//! PostgreSQL control-queue claim atomicity regressions.

#![cfg(feature = "postgres")]

use nebula_storage::postgres::{PgControlQueue, init_schema};
use nebula_storage_port::StorageError;
use nebula_storage_port::store::ControlQueue;

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

fn database_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(url) => Some(url),
        Err(std::env::VarError::NotPresent) => {
            assert!(
                std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                "required PostgreSQL conformance needs DATABASE_URL"
            );
            None
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("configured PostgreSQL URL must be Unicode")
        },
    }
}

#[tokio::test]
async fn decoding_failure_rolls_back_every_claimed_row() {
    let Some(url) = database_url() else {
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "control_claim_atomicity")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let row_id = [0x41_u8; 16];
    sqlx::query(
        "INSERT INTO port_control_queue \
         (id, execution_id, workspace_id, org_id, command, status) \
         VALUES ($1, $2, $3, $4, $5, 'Pending')",
    )
    .bind(row_id.as_slice())
    .bind("execution")
    .bind("workspace")
    .bind("org")
    .bind("malformed-command")
    .execute(&pool)
    .await
    .unwrap();

    let queue = PgControlQueue::new(pool.clone());
    let result = queue.claim_pending(&[0x51; 16], 256).await;
    assert!(matches!(result, Err(StorageError::Serialization(_))));

    let (status, generation): (String, i64) =
        sqlx::query_as("SELECT status, claim_generation FROM port_control_queue WHERE id = $1")
            .bind(row_id.as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "Pending");
    assert_eq!(generation, 0);
}
