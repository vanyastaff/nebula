//! Execution handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use nebula_core::{ExecutionId, WorkflowId};

use crate::{
    errors::{ApiError, ApiResult},
    handlers::workflow::PaginationParams,
    models::{
        ExecutionLogsResponse, ExecutionOutputsResponse, ExecutionResponse, ListExecutionsResponse,
        RunningExecutionSummary, StartExecutionRequest,
    },
    state::AppState,
};

/// List all executions (global) — returns running execution IDs with count.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn list_all_executions(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let running_ids = state
        .execution_repo
        .list_running()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list executions: {}", e)))?;

    let total = running_ids.len();

    // Apply pagination over the running list.
    let offset = params.offset();
    let limit = params.limit();
    let page_ids: Vec<&ExecutionId> = running_ids.iter().skip(offset).take(limit).collect();

    let executions: Vec<RunningExecutionSummary> = page_ids
        .into_iter()
        .map(|id| RunningExecutionSummary { id: id.to_string() })
        .collect();

    Ok(Json(ListExecutionsResponse {
        executions,
        total,
        page: params.page,
        page_size: params.limit(),
    }))
}

/// List executions for a workflow — returns running executions for the workflow.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn list_executions(
    State(state): State<AppState>,
    Path(workflow_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    let total = state
        .execution_repo
        .count(Some(workflow_id_parsed))
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to count executions: {}", e)))?;

    // TODO: list_running() returns ALL running execution IDs with no workflow filter.
    // Replace with a workflow-scoped query once ExecutionRepo gains a list(workflow_id)
    // method. The `workflow_id` parameter is validated above but not yet applied.
    let running_ids = state
        .execution_repo
        .list_running()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list executions: {}", e)))?;

    let offset = params.offset();
    let limit = params.limit();
    let executions: Vec<RunningExecutionSummary> = running_ids
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|id| RunningExecutionSummary { id: id.to_string() })
        .collect();

    Ok(Json(ListExecutionsResponse {
        executions,
        total: total as usize,
        page: params.page,
        page_size: params.limit(),
    }))
}

/// Get all node outputs for an execution.
///
/// Returns a map of `node_id → output_value` for every node that has
/// completed at least one attempt.
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid execution ID.
/// - [`ApiError::NotFound`] if no execution with that ID exists.
/// - [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn get_execution_outputs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionOutputsResponse>> {
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {}", e)))?;

    // Verify the execution exists before loading outputs.
    state
        .execution_repo
        .get_state(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to check execution: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {} not found", id)))?;

    let outputs = state
        .execution_repo
        .load_all_outputs(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load outputs: {}", e)))?;

    // Convert NodeId keys to strings for JSON serialisation.
    let string_outputs: std::collections::HashMap<String, serde_json::Value> = outputs
        .into_iter()
        .map(|(node_id, val)| (node_id.to_string(), val))
        .collect();

    Ok(Json(ExecutionOutputsResponse {
        execution_id: id,
        outputs: string_outputs,
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

    // Current timestamp via chrono — does not panic on misconfigured clocks.
    let now = chrono::Utc::now().timestamp();

    // Create initial execution state
    let execution_state = serde_json::json!({
        "workflow_id": workflow_id,
        "status": "pending",
        "started_at": now,
        "input": payload.input,
    });

    // Create execution record. We must call `create` here — the previous
    // implementation called `transition(id, expected_version = 0, ...)`,
    // which is a CAS UPDATE that can never match a brand-new ID (no row
    // exists yet), so every call returned `Ok(false)` and the handler
    // surfaced an Internal error unconditionally.
    state
        .execution_repo
        .create(execution_id, workflow_id_parsed, execution_state.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create execution: {}", e)))?;

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
            let now = chrono::Utc::now().timestamp();
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

/// Return journal (log) entries for an execution.
///
/// Journal entries are appended by the engine as execution progresses.
/// Each entry is an arbitrary JSON object — the shape is engine-defined.
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid execution ID.
/// - [`ApiError::NotFound`] if no execution with that ID exists.
/// - [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn get_execution_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ExecutionLogsResponse>> {
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {}", e)))?;

    // Verify the execution exists before loading the journal.
    state
        .execution_repo
        .get_state(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to check execution: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {} not found", id)))?;

    let logs = state
        .execution_repo
        .get_journal(execution_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load execution logs: {}", e)))?;

    Ok(Json(ExecutionLogsResponse {
        execution_id: id,
        logs,
    }))
}
