//! Start-authority materialization contract on the in-memory and SQLite adapters.
//!
//! The PostgreSQL arm lives in `start_materialization_postgres` (an evidence
//! binary that needs a live database).

use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, WorkerFlavorRevisionId,
    WorkflowId,
};
use nebula_storage_port::dto::{
    ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution,
    PlanFlavorRevisionIds,
};
use nebula_storage_port::store::{
    StartAcceptanceStore, StartContractIdentity, StartMaterializationError,
};
#[path = "support/start_materialization_common.rs"]
mod common;
#[path = "support/start_materialization_oracle.rs"]
mod oracle;

use common::write_observations;

#[tokio::test]
async fn in_memory_materialization_contract() {
    let executions = nebula_storage::InMemoryExecutionStore::new();
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&executions);
    let queue = nebula_storage::inmem::InMemoryControlQueue::new(&executions);
    let catalog = executions.plan_flavor_catalog();
    let oracle::RunEvidence { observations, .. } =
        oracle::run(&starts, &executions, &queue, &catalog, &catalog).await;
    write_observations(
        "in-memory",
        "NEBULA_START_AUTHORITY_IN_MEMORY_OBSERVATIONS_PATH",
        observations,
    );
}

#[tokio::test]
async fn in_memory_trigger_contract() {
    let executions = nebula_storage::InMemoryExecutionStore::new();
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&executions);
    let catalog = executions.plan_flavor_catalog();
    let queue = nebula_storage::inmem::InMemoryControlQueue::new(&executions);
    oracle::trigger_replay(&starts, &executions, &queue, &catalog, &catalog).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_materialization_contract() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("start.db"))
        .create_if_missing(true)
        .busy_timeout(std::time::Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let starts = nebula_storage::sqlite::SqliteStartAcceptanceStore::new(pool.clone());
    let executions = nebula_storage::sqlite::SqliteExecutionStore::new(pool.clone());
    let queue = nebula_storage::sqlite::SqliteControlQueue::new(pool.clone());
    let catalog = nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    );
    let evidence = oracle::run(&starts, &executions, &queue, &catalog, &catalog).await;
    write_observations(
        "sqlite",
        "NEBULA_START_AUTHORITY_SQLITE_OBSERVATIONS_PATH",
        evidence.observations,
    );
    let stored = evidence.stored;
    oracle::trigger_replay(&starts, &executions, &queue, &catalog, &catalog).await;
    pool.close().await;
    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let starts = nebula_storage::sqlite::SqliteStartAcceptanceStore::new(reopened);
    assert_eq!(
        starts
            .read_contract_bundle(stored.scope(), stored.execution_id())
            .await
            .unwrap(),
        Some(stored)
    );
}

#[tokio::test]
async fn malformed_materialization_has_no_durable_delta_or_payload_debug() {
    let execution = nebula_storage::InMemoryExecutionStore::new();
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&execution);
    let scope = nebula_storage_port::Scope::new("ws", "org");
    let execution_id = ExecutionId::new().to_string();
    let workflow_id = WorkflowId::new().to_string();
    let state = serde_json::json!({"private":"secret-canary"});
    let command = ControlMsg {
        id: [1; 16],
        execution_id: execution_id.clone(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let identity = StartContractIdentity::new(
        ExecutionContractBundleId::new(),
        PlanFlavorRevisionIds::new(
            ExecutablePlanRevisionId::from_bytes([1; 32]),
            WorkerFlavorRevisionId::from_bytes([2; 32]),
        ),
    );
    let bundle =
        ContractBundleRecord::v1_json(identity, b"{\"private\":\"secret-canary\"}".to_vec())
            .unwrap();
    let start = MaterializedStart::new(
        &scope,
        None,
        &execution_id,
        NewExecution::new(&workflow_id, &state),
        &command,
        &bundle,
    );
    assert!(!format!("{start:?}").contains("secret-canary"));
    assert!(matches!(
        starts.materialize_start(&start).await,
        Err(StartMaterializationError::InvalidEnvelope)
    ));
    assert!(
        starts
            .read_contract_bundle(&scope, &execution_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        nebula_storage_port::store::ExecutionStore::get(&execution, &scope, &execution_id)
            .await
            .unwrap()
            .is_none()
    );
}
