//! PostgreSQL control-queue claim atomicity regressions.

#![cfg(feature = "postgres")]

use nebula_storage::postgres::{PgControlQueue, PgExecutionStore, init_schema};
use nebula_storage_port::store::{ControlQueue, ExecutionStore};
use nebula_storage_port::{Scope, StorageError};

#[path = "support/execution_parents.rs"]
mod execution_parents;
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

use execution_parents::SeedExecutionParents as _;

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
        panic!(
            "decoding_failure_rolls_back_every_claimed_row: backend unreachable — the case \
             cannot run and must fail rather than pass unchecked; reach the backend (set \
             DATABASE_URL for postgres) or run without this feature"
        );
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "control_claim_atomicity")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let scope = Scope::new("workspace", "org");
    pool.seed_execution_parents(&scope, "workflow").await;
    PgExecutionStore::new(pool.clone())
        .create(&scope, "execution", "workflow", serde_json::json!({}))
        .await
        .unwrap();
    // The schema admits any JSON document as a resume target; one that is no
    // `ResumeTarget` fails only when the claim decodes it.
    let row_id = [0x41_u8; 16];
    sqlx::query(
        "INSERT INTO execution_control_queue \
         (org_id, workspace_id, execution_id, id, command, status, resume_target) \
         VALUES ($1, $2, $3, $4, 'Resume', 'Pending', '{\"unexpected\": true}')",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind("execution")
    .bind(row_id.as_slice())
    .execute(&pool)
    .await
    .unwrap();

    let queue = PgControlQueue::new(pool.clone());
    let result = queue.claim_pending(&[0x51; 16], 256).await;
    assert!(
        matches!(result, Err(StorageError::Corrupt(_))),
        "an undecodable row must fail the claim, got {result:?}"
    );

    let (status, generation): (String, i64) = sqlx::query_as(
        "SELECT status, claim_generation FROM execution_control_queue WHERE id = $1",
    )
    .bind(row_id.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "Pending");
    assert_eq!(generation, 0);
}
