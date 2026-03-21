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
use nebula_core::{ExecutionId, WorkflowId};

/// List all executions (global)
/// GET /api/v1/executions
pub async fn list_all_executions(
    State(_state): State<AppState>,
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
    let (_version, execution_state) =
        state_result.ok_or_else(|| ApiError::NotFound(format!("Execution {} not found", id)))?;

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

    let finished_at = execution_state.get("finished_at").and_then(|v| v.as_i64());

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
    State(state): State<AppState>,
    Path(workflow_id): Path<String>,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    // Parse workflow ID
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Verify workflow exists
    state
        .workflow_repo
        .get(workflow_id_parsed)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", workflow_id)))?;

    // Generate new execution ID
    let execution_id = ExecutionId::new();

    // Get current timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Create initial execution state
    let execution_state = serde_json::json!({
        "workflow_id": workflow_id,
        "status": "pending",
        "started_at": now,
        "input": payload.input,
    });

    // Create execution record (version 0 for new execution)
    let success = state
        .execution_repo
        .transition(execution_id, 0, execution_state.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create execution: {}", e)))?;

    if !success {
        return Err(ApiError::Internal("Failed to create execution record".to_string()));
    }

    // Build response
    let response = ExecutionResponse {
        id: execution_id.to_string(),
        workflow_id,
        status: "pending".to_string(),
        started_at: now,
        finished_at: None,
        input: payload.input,
        output: None,
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Cancel execution
/// POST /api/v1/executions/:id/cancel
pub async fn cancel_execution(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionResponse>> {
    use nebula_core::ExecutionId;

    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {}", e)))?;

    // Fetch current execution state from repository
    let state_result = state
        .execution_repo
        .get_state(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get execution: {}", e)))?;

    // Check if execution exists
    let (version, mut execution_state) =
        state_result.ok_or_else(|| ApiError::NotFound(format!("Execution {} not found", id)))?;

    // Check if execution is already in a terminal state
    let current_status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if current_status == "completed" || current_status == "failed" || current_status == "cancelled"
    {
        return Err(ApiError::validation_message(format!(
            "Cannot cancel execution in '{}' state",
            current_status
        )));
    }

    // Update state to cancelled
    if let Some(state_obj) = execution_state.as_object_mut() {
        state_obj.insert("status".to_string(), serde_json::json!("cancelled"));

        // Set finished_at timestamp if not already set
        if !state_obj.contains_key("finished_at") {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            state_obj.insert("finished_at".to_string(), serde_json::json!(now));
        }
    }

    // Apply state transition using CAS
    let transition_result = state
        .execution_repo
        .transition(execution_id, version, execution_state.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to cancel execution: {}", e)))?;

    if !transition_result {
        return Err(ApiError::Internal(
            "Failed to cancel execution: concurrent modification detected".to_string(),
        ));
    }

    // Extract fields from updated execution state
    let workflow_id = execution_state
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("cancelled")
        .to_string();

    let started_at = execution_state
        .get("started_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let finished_at = execution_state.get("finished_at").and_then(|v| v.as_i64());

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
