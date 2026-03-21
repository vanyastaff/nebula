//! Execution handlers

use crate::{
    errors::{ApiError, ApiResult},
    handlers::workflow::PaginationParams,
    models::{ExecutionResponse, ListExecutionsResponse, StartExecutionRequest},
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

/// List executions for a workflow
/// GET /api/v1/workflows/:workflow_id/executions
pub async fn list_executions(
    State(_state): State<AppState>,
    Path(_workflow_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    // TODO: ExecutionRepo doesn't have a list() method yet.
    // This requires extending the ExecutionRepo trait with:
    //   async fn list(&self, workflow_id: Option<WorkflowId>, offset: usize, limit: usize)
    //       -> Result<Vec<(ExecutionId, serde_json::Value)>, ExecutionRepoError>;
    // For now, return empty list with proper pagination metadata.

    Ok(Json(ListExecutionsResponse {
        executions: vec![],
        total: 0,
        page: params.page,
        page_size: params.limit(),
    }))
}

/// Get execution by ID
/// GET /api/v1/executions/:id
pub async fn get_execution(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionResponse>> {
    use nebula_core::ExecutionId;

    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {}", e)))?;

    // Fetch execution state from repository
    let state_result = state
        .execution_repo
        .get_state(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get execution: {}", e)))?;

    // Check if execution exists (get_state returns Option<(version, state)>)
    let (_version, execution_state) = state_result
        .ok_or_else(|| ApiError::NotFound(format!("Execution {} not found", id)))?;

    // Extract fields from execution state JSON
    let workflow_id = execution_state
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let started_at = execution_state
        .get("started_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let finished_at = execution_state
        .get("finished_at")
        .and_then(|v| v.as_i64());

    let input = execution_state.get("input").cloned();

    let output = execution_state.get("output").cloned();

    Ok(Json(ExecutionResponse {
        id,
        workflow_id,
        status,
        started_at,
        finished_at,
        input,
        output,
    }))
}

/// Start workflow execution (enqueue and return 202 Accepted)
/// POST /api/v1/workflows/:workflow_id/executions
pub async fn start_execution(
    State(_state): State<AppState>,
    Path(_workflow_id): Path<String>,
    Json(_payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    // TODO: Validate workflow exists, enqueue execution, return 202
    // This should NOT wait for execution to complete!
    Err(ApiError::Internal("Not implemented yet".to_string()))
}

/// Cancel execution
/// POST /api/v1/executions/:id/cancel
pub async fn cancel_execution(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionResponse>> {
    // TODO: Implement via execution_repo.cancel()
    Err(ApiError::NotFound(format!("Execution {} not found", id)))
}
