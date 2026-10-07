mod common;

use axum::http::StatusCode;
use common::{http_helpers::auth_get, *};
use nebula_api::{ApiConfig, AppState, app};
use nebula_core::{ExecutionId, WorkflowId, node_key};
use nebula_execution::{
    ExecutionOutput, ExecutionState, ExecutionStatus, NodeCheckpoint, state::AttemptOutcome,
};
use nebula_workflow::NodeState;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn inspect(state: AppState, id: ExecutionId) -> (StatusCode, Value) {
    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(auth_get(
            &ws_path(&format!("/executions/{id}")),
            &create_test_jwt(),
        ))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn snapshot() -> ExecutionState {
    ExecutionState::new(ExecutionId::new(), WorkflowId::new(), &[node_key!("fetch")])
}

fn record_result(snapshot: &mut ExecutionState, result: nebula_action::ActionResult<Value>) {
    snapshot.checkpoint.as_mut().unwrap().insert(
        node_key!("fetch"),
        NodeCheckpoint::ActionResult {
            format_version: 1,
            value: serde_json::to_value(result).unwrap(),
        },
    );
}

fn failed_snapshot() -> ExecutionState {
    let mut snapshot = snapshot();
    snapshot.status = ExecutionStatus::Failed;
    snapshot.started_at = Some(snapshot.created_at);
    snapshot.completed_at = Some(snapshot.updated_at);
    let failure = nebula_execution::ErrorEnvelope::new(
        nebula_error::ErrorCode::new("ENGINE:NODE_FAILED"),
        nebula_error::ErrorCategory::External,
        false,
    )
    .with_redacted_message("Node failed");
    snapshot
        .record_node_attempt(
            node_key!("fetch"),
            AttemptOutcome::Failure {
                error: failure.clone(),
            },
        )
        .unwrap();
    let node = snapshot.node_states.get_mut(&node_key!("fetch")).unwrap();
    node.state = NodeState::Failed;
    node.error_message = Some(failure);
    snapshot
}

#[tokio::test]
async fn created_execution_does_not_claim_to_have_started() {
    let (state, handles) = create_state_with_port_handles().await;
    let snapshot = snapshot();
    handles
        .seed_execution(
            snapshot.execution_id,
            snapshot.workflow_id,
            serde_json::to_value(&snapshot).unwrap(),
        )
        .await;
    let record = state
        .execution_store
        .get(&port_scope(), &snapshot.execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let (status, body) = inspect(state, snapshot.execution_id).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("started_at").is_none(),
        "created is not started: {body}"
    );
    assert_eq!(
        body["created_at"],
        record
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    );
    assert_eq!(body["status"], "created");
    assert_eq!(body["nodes"]["fetch"]["status"], "pending");
}

#[tokio::test]
async fn corrupt_snapshot_is_not_successful_detail_or_payload_echo() {
    for field in ["workflow_id", "created_at", "node_states"] {
        let (state, handles) = create_state_with_port_handles().await;
        let snapshot = snapshot();
        let mut wire = serde_json::to_value(&snapshot).unwrap();
        wire[field] = json!("private-corrupt-value");
        handles
            .seed_execution(snapshot.execution_id, snapshot.workflow_id, wire)
            .await;
        let (status, body) = inspect(state, snapshot.execution_id).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{field}: {body}");
        assert!(!body.to_string().contains("private-corrupt-value"));
    }
}

#[tokio::test]
async fn snapshot_identity_must_match_the_scoped_record() {
    let (state, handles) = create_state_with_port_handles().await;
    let snapshot = snapshot();
    let other = ExecutionId::new();
    handles
        .seed_execution(
            other,
            snapshot.workflow_id,
            serde_json::to_value(&snapshot).unwrap(),
        )
        .await;
    let (status, _) = inspect(state, other).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn detail_reads_persisted_attempts_and_outputs_without_the_node_cache() {
    let (state, _) = create_state_with_port_handles().await;
    let mut snapshot = snapshot();
    snapshot.status = ExecutionStatus::Completed;
    snapshot.started_at = Some(snapshot.created_at);
    snapshot.completed_at = Some(snapshot.updated_at);
    let node = snapshot.node_states.get_mut(&node_key!("fetch")).unwrap();
    node.state = NodeState::Completed;
    snapshot
        .record_node_attempt(
            node_key!("fetch"),
            AttemptOutcome::Success {
                output: ExecutionOutput::inline(json!({"answer":42})),
                output_bytes: 13,
            },
        )
        .unwrap();
    record_result(
        &mut snapshot,
        nebula_action::ActionResult::success(json!({"answer":42})),
    );
    persist_execution_snapshot(state.execution_store.as_ref(), &snapshot).await;
    let (status, body) = inspect(state, snapshot.execution_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["nodes"]["fetch"]["output"],
        json!({"type":"inline","value":{"answer":42}})
    );
    assert_eq!(body["nodes"]["fetch"]["attempts"][0]["attempt_number"], 1);
    assert_eq!(
        body["nodes"]["fetch"]["attempts"][0]["output"]["value"]["answer"],
        42
    );
    assert!(body.get("checkpoint").is_none());
    assert!(!body.to_string().contains("idempotency_key"));
}

#[tokio::test]
async fn safe_failures_round_trip_but_bare_provider_errors_are_rejected() {
    let (state, handles) = create_state_with_port_handles().await;
    let failed = failed_snapshot();
    persist_execution_snapshot(state.execution_store.as_ref(), &failed).await;
    let (status, body) = inspect(state.clone(), failed.execution_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["nodes"]["fetch"]["error"]["code"],
        "ENGINE:NODE_FAILED"
    );
    assert_eq!(
        body["nodes"]["fetch"]["attempts"][0]["error"]["message"],
        "Node failed"
    );
    assert_eq!(body["nodes"]["fetch"]["error"]["retryable"], false);
    let snapshot = snapshot();
    let mut corrupt = serde_json::to_value(&snapshot).unwrap();
    corrupt["node_states"]["fetch"]["error_message"] = json!("provider secret: credential-token");
    handles
        .seed_execution(snapshot.execution_id, snapshot.workflow_id, corrupt)
        .await;
    let (status, body) = inspect(state, snapshot.execution_id).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.to_string().contains("credential-token"));
}

#[tokio::test]
async fn inline_null_is_data_and_external_storage_keys_stay_private() {
    for (output, expected) in [
        (
            ExecutionOutput::inline(Value::Null),
            json!({"type":"inline","value":null}),
        ),
        (
            ExecutionOutput::BlobRef {
                key: "private-backend-key".into(),
                size: 8192,
                mime: "application/json".into(),
            },
            json!({"type":"external","size":8192,"mime":"application/json"}),
        ),
    ] {
        let (state, _) = create_state_with_port_handles().await;
        let mut snapshot = snapshot();
        snapshot
            .record_node_attempt(
                node_key!("fetch"),
                AttemptOutcome::Success {
                    output,
                    output_bytes: 8192,
                },
            )
            .unwrap();
        persist_execution_snapshot(state.execution_store.as_ref(), &snapshot).await;
        let (status, body) = inspect(state, snapshot.execution_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["nodes"]["fetch"]["attempts"][0]["output"], expected);
        assert!(!body.to_string().contains("private-backend-key"));
    }
}

#[tokio::test]
async fn checkpoint_null_and_outputless_results_remain_distinct() {
    for (result, expected) in [
        (
            nebula_action::ActionResult::success(Value::Null),
            Some(json!({"type":"inline","value":null})),
        ),
        (
            nebula_action::ActionResult::<Value>::Drop { reason: None },
            None,
        ),
    ] {
        let (state, _) = create_state_with_port_handles().await;
        let mut snapshot = snapshot();
        // Historical attempt data must never become the current result when a
        // later checkpoint records an outputless completion.
        snapshot
            .record_node_attempt(
                node_key!("fetch"),
                AttemptOutcome::Success {
                    output: ExecutionOutput::inline(json!("earlier-result")),
                    output_bytes: 16,
                },
            )
            .unwrap();
        record_result(&mut snapshot, result);
        persist_execution_snapshot(state.execution_store.as_ref(), &snapshot).await;
        let (status, body) = inspect(state, snapshot.execution_id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["nodes"]["fetch"].get("output"), expected.as_ref());
    }
}

#[tokio::test]
async fn unsupported_or_corrupt_checkpoint_fails_without_exposing_its_value() {
    for (version, value) in [
        (2, json!({})),
        (1, json!({"type":"private-invalid-result"})),
    ] {
        let (state, _) = create_state_with_port_handles().await;
        let mut snapshot = snapshot();
        snapshot.checkpoint.as_mut().unwrap().insert(
            node_key!("fetch"),
            NodeCheckpoint::ActionResult {
                format_version: version,
                value,
            },
        );
        persist_execution_snapshot(state.execution_store.as_ref(), &snapshot).await;
        let (status, body) = inspect(state, snapshot.execution_id).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!body.to_string().contains("private-invalid-result"));
    }
}

#[tokio::test]
async fn typed_checkpoint_outputs_preserve_delivery_kind_without_private_locations() {
    use nebula_action::output::{ActionOutput, BinaryData, BinaryStorage, DataReference};
    let reference = ActionOutput::Reference(DataReference {
        storage_type: "private-backend".into(),
        path: "private-location".into(),
        size: Some(8192),
        content_type: Some("application/json".into()),
    });
    let unknown_reference = ActionOutput::Reference(DataReference {
        storage_type: "private-backend".into(),
        path: "private-location".into(),
        size: None,
        content_type: None,
    });
    let binary = ActionOutput::Binary(BinaryData {
        content_type: "image/png".into(),
        data: BinaryStorage::Inline {
            bytes: vec![1, 2, 3],
        },
        size: 999,
        metadata: Some(json!({"private":"private-metadata"})),
    });
    for (output, expected) in [
        (
            reference.clone(),
            json!({"type":"external","size":8192,"mime":"application/json"}),
        ),
        (unknown_reference, json!({"type":"external"})),
        (binary, json!({"type":"binary","size":3,"mime":"image/png"})),
        (
            ActionOutput::Collection(vec![
                ActionOutput::Value(json!(42)),
                reference,
                ActionOutput::Empty,
            ]),
            json!({"type":"collection","items":[{"type":"inline","value":42},{"type":"external","size":8192,"mime":"application/json"},{"type":"empty"}]}),
        ),
    ] {
        let (state, _) = create_state_with_port_handles().await;
        let mut snapshot = snapshot();
        record_result(
            &mut snapshot,
            nebula_action::ActionResult::success_output(output),
        );
        persist_execution_snapshot(state.execution_store.as_ref(), &snapshot).await;
        let (status, body) = inspect(state, snapshot.execution_id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["nodes"]["fetch"]["output"], expected);
        assert!(!body.to_string().contains("private-"));
    }
}

#[tokio::test]
async fn foreign_snapshot_is_not_found_even_if_its_payload_is_corrupt() {
    let (state, _) = create_state_with_port_handles().await;
    let snapshot = snapshot();
    let foreign = nebula_storage_port::Scope::new(
        nebula_core::WorkspaceId::new().to_string(),
        nebula_core::OrgId::new().to_string(),
    );
    state
        .execution_store
        .create(
            &foreign,
            &snapshot.execution_id.to_string(),
            &snapshot.workflow_id.to_string(),
            json!({"private":"foreign-payload"}),
        )
        .await
        .unwrap();
    let (status, body) = inspect(state, snapshot.execution_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.to_string().contains("foreign-payload"));
}

async fn seed_parents(
    tenants: &dyn nebula_storage_port::store::TenantProvisioningStore,
    workflows: &dyn nebula_storage_port::store::WorkflowStore,
    workflow_id: WorkflowId,
) {
    use nebula_storage_port::dto::{
        PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningRequest,
        WorkflowRecord,
    };
    let scope = port_scope();
    let org = TenantOrgCreate::new(
        scope.org_id.clone(),
        "inspection".into(),
        "Inspection".into(),
        "test".into(),
        "free".into(),
        None,
        json!({}),
    )
    .unwrap();
    let workspace = TenantDefaultWorkspaceCreate::new(
        scope.workspace_id.clone(),
        "default".into(),
        "Default".into(),
        None,
        "test".into(),
        json!({}),
    )
    .unwrap();
    tenants
        .provision_tenant(
            TenantProvisioningRequest::new(
                org,
                workspace,
                PrincipalKind::User,
                "test-owner".into(),
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    workflows
        .create(
            &scope,
            WorkflowRecord {
                id: workflow_id.to_string(),
                scope: scope.clone(),
                version: 1,
                slug: "inspection".into(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_inspection_survives_closing_all_connections() {
    use nebula_storage::sqlite::{
        SqliteExecutionStore, SqliteTenantProvisioningStore, SqliteWorkflowStore,
    };
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("inspection.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let snapshot = failed_snapshot();
    seed_parents(
        &SqliteTenantProvisioningStore::new(pool.clone()),
        &SqliteWorkflowStore::new(pool.clone()),
        snapshot.workflow_id,
    )
    .await;
    persist_execution_snapshot(&SqliteExecutionStore::new(pool.clone()), &snapshot).await;
    pool.close().await;

    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.create_if_missing(false))
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&reopened)
        .await
        .unwrap();
    let (mut state, _) = create_state_with_port_handles().await;
    state.execution_store = Arc::new(SqliteExecutionStore::new(reopened.clone()));
    let (status, body) = inspect(state, snapshot.execution_id).await;
    reopened.close().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed");
    assert_eq!(
        body["nodes"]["fetch"]["attempts"][0]["error"]["code"],
        "ENGINE:NODE_FAILED"
    );
    assert_eq!(body["snapshot_version"], 1);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_inspection_survives_closing_all_connections() {
    use nebula_storage::postgres::{PgExecutionStore, PgTenantProvisioningStore, PgWorkflowStore};
    use std::sync::Arc;
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
            "Postgres evidence requires DATABASE_URL"
        );
        return;
    };
    let schema = format!("inspection_{}", uuid::Uuid::new_v4().simple());
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let snapshot = failed_snapshot();
    seed_parents(
        &PgTenantProvisioningStore::new(pool.clone()),
        &PgWorkflowStore::new(pool.clone()),
        snapshot.workflow_id,
    )
    .await;
    persist_execution_snapshot(&PgExecutionStore::new(pool.clone()), &snapshot).await;
    pool.close().await;

    let reopened = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&reopened)
        .await
        .unwrap();
    let (mut state, _) = create_state_with_port_handles().await;
    state.execution_store = Arc::new(PgExecutionStore::new(reopened.clone()));
    let (status, body) = inspect(state, snapshot.execution_id).await;
    reopened.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed");
    assert_eq!(
        body["nodes"]["fetch"]["attempts"][0]["error"]["code"],
        "ENGINE:NODE_FAILED"
    );
    assert_eq!(body["snapshot_version"], 1);
}
