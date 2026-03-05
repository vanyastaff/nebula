//! Execution handlers

use crate::{
    errors::{ApiError, ApiResult},
    models::{ExecutionResponse, ListExecutionsResponse, StartExecutionRequest},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

/// List executions for a workflow
/// GET /api/v1/workflows/:workflow_id/executions
pub async fn list_executions(
    State(_state): State<AppState>,
    Path(_workflow_id): Path<String>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    // TODO: Implement via execution_repo.list()
    Ok(Json(ListExecutionsResponse {
        executions: vec![],
        total: 0,
        page: 1,
        page_size: 10,
    }))
}

/// Get execution by ID
/// GET /api/v1/executions/:id
pub async fn get_execution(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionResponse>> {
    // TODO: Implement via execution_repo.get()
    Err(ApiError::NotFound(format!("Execution {} not found", id)))
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

