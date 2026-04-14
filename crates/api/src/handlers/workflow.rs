//! Workflow handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use nebula_core::{ExecutionId, WorkflowId};
use serde::Deserialize;

use crate::{
    errors::{ApiError, ApiResult},
    models::{
        CreateWorkflowRequest, ExecutionResponse, ListWorkflowsResponse, StartExecutionRequest,
        UpdateWorkflowRequest, WorkflowResponse, WorkflowValidateResponse,
    },
    state::AppState,
};

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Page number (1-indexed)
    #[serde(default = "default_page")]
    pub page: usize,
    /// Page size (default 10, max 100)
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    10
}

impl PaginationParams {
    /// Calculate offset for database query (0-indexed)
    pub fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    /// Get validated limit (capped at 100)
    pub fn limit(&self) -> usize {
        self.page_size.min(100)
    }
}

/// List workflows
/// GET /api/v1/workflows
pub async fn list_workflows(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    let offset = params.offset();
    let limit = params.limit();

    // Fetch workflows from repository
    let workflows = state
        .workflow_repo
        .list(offset, limit)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list workflows: {}", e)))?;

    // Map to response DTOs
    let workflow_responses: Vec<WorkflowResponse> = workflows
        .into_iter()
        .map(|(id, definition)| {
            // Extract fields from workflow definition JSON
            let name = definition
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed Workflow")
                .to_string();

            let description = definition
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let created_at = definition
                .get("created_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let updated_at = definition
                .get("updated_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            WorkflowResponse {
                id: id.to_string(),
                name,
                description,
                created_at,
                updated_at,
            }
        })
        .collect();

    // Note: total count is approximated as we don't have a count() method yet
    // For accurate pagination, we would need to add count() to WorkflowRepo
    let total = workflow_responses.len();

    Ok(Json(ListWorkflowsResponse {
        workflows: workflow_responses,
        total,
        page: params.page,
        page_size: params.page_size,
    }))
}

/// Get workflow by ID
/// GET /api/v1/workflows/:id
pub async fn get_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Fetch workflow from repository
    let definition = state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?;

    // Check if workflow exists
    let definition =
        definition.ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Extract fields from workflow definition JSON
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let created_at = definition
        .get("created_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let updated_at = definition
        .get("updated_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(Json(WorkflowResponse {
        id,
        name,
        description,
        created_at,
        updated_at,
    }))
}

/// Create workflow
/// POST /api/v1/workflows
pub async fn create_workflow(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<WorkflowResponse>)> {
    // Validate workflow name
    if payload.name.trim().is_empty() {
        return Err(ApiError::validation_message(
            "Workflow name cannot be empty",
        ));
    }

    // Generate new workflow ID
    let workflow_id = WorkflowId::new();

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

    // Build workflow definition by merging request definition with metadata
    let mut definition = payload.definition.clone();
    if let Some(obj) = definition.as_object_mut() {
        obj.insert("name".to_string(), serde_json::json!(payload.name));
        if let Some(desc) = &payload.description {
            obj.insert("description".to_string(), serde_json::json!(desc));
        }
        obj.insert("created_at".to_string(), serde_json::json!(now));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        // If definition is not an object, wrap it with metadata
        definition = serde_json::json!({
            "name": payload.name,
            "description": payload.description,
            "created_at": now,
            "updated_at": now,
            "definition": definition,
        });
    }

    // Save workflow with version 0 (new workflow)
    state
        .workflow_repo
        .save(workflow_id, 0, definition.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create workflow: {}", e)))?;

    // Build response
    let response = WorkflowResponse {
        id: workflow_id.to_string(),
        name: payload.name,
        description: payload.description,
        created_at: now,
        updated_at: now,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Update workflow
/// PUT /api/v1/workflows/:id
pub async fn update_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateWorkflowRequest>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Get current workflow with version
    let (version, mut definition) = state
        .workflow_repo
        .get_with_version(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

    // Update definition with new values
    if let Some(obj) = definition.as_object_mut() {
        // Update name if provided
        if let Some(name) = &payload.name {
            if name.trim().is_empty() {
                return Err(ApiError::validation_message(
                    "Workflow name cannot be empty",
                ));
            }
            obj.insert("name".to_string(), serde_json::json!(name));
        }

        // Update description if provided
        if let Some(desc) = &payload.description {
            obj.insert("description".to_string(), serde_json::json!(desc));
        }

        // Merge definition if provided
        if let Some(new_def) = &payload.definition
            && let Some(new_obj) = new_def.as_object()
        {
            for (key, value) in new_obj {
                // Don't allow overwriting metadata fields
                if !["name", "description", "created_at", "updated_at"].contains(&key.as_str()) {
                    obj.insert(key.clone(), value.clone());
                }
            }
        }

        // Update the updated_at timestamp
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        return Err(ApiError::Internal(
            "Invalid workflow definition format".to_string(),
        ));
    }

    // Save with optimistic concurrency control
    state
        .workflow_repo
        .save(workflow_id, version, definition.clone())
        .await
        .map_err(|e| {
            use nebula_storage::WorkflowRepoError;
            match e {
                WorkflowRepoError::Conflict { .. } => {
                    ApiError::Conflict("Workflow was modified by another request".to_string())
                },
                _ => ApiError::Internal(format!("Failed to update workflow: {}", e)),
            }
        })?;

    // Extract fields for response
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let created_at = definition
        .get("created_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let updated_at = definition
        .get("updated_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(Json(WorkflowResponse {
        id,
        name,
        description,
        created_at,
        updated_at,
    }))
}

/// Delete workflow
/// DELETE /api/v1/workflows/:id
pub async fn delete_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Delete workflow from repository
    let existed = state
        .workflow_repo
        .delete(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete workflow: {}", e)))?;

    // Return 404 if workflow didn't exist, 204 No Content if it was deleted
    if existed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("Workflow {} not found", id)))
    }
}

/// Activate workflow
/// POST /api/v1/workflows/:id/activate
pub async fn activate_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Get current workflow with version for optimistic concurrency
    let (version, mut definition) = state
        .workflow_repo
        .get_with_version(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

    // Update definition to set active flag
    if let Some(obj) = definition.as_object_mut() {
        obj.insert("active".to_string(), serde_json::json!(true));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        return Err(ApiError::Internal(
            "Invalid workflow definition format".to_string(),
        ));
    }

    // Save with optimistic concurrency control
    state
        .workflow_repo
        .save(workflow_id, version, definition.clone())
        .await
        .map_err(|e| {
            use nebula_storage::WorkflowRepoError;
            match e {
                WorkflowRepoError::Conflict { .. } => {
                    ApiError::Conflict("Workflow was modified by another request".to_string())
                },
                _ => ApiError::Internal(format!("Failed to activate workflow: {}", e)),
            }
        })?;

    // Extract fields for response
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let created_at = definition
        .get("created_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let updated_at = definition
        .get("updated_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(Json(WorkflowResponse {
        id,
        name,
        description,
        created_at,
        updated_at,
    }))
}

/// Execute workflow (enqueue and return 202 Accepted)
/// POST /api/v1/workflows/:id/execute
pub async fn execute_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Verify workflow exists
    state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Generate new execution ID
    let execution_id = ExecutionId::new();

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

    // Create initial execution state
    let execution_state = serde_json::json!({
        "workflow_id": id,
        "status": "pending",
        "started_at": now,
        "input": payload.input,
    });

    // Create execution record via `create` — `transition` is a CAS UPDATE
    // and was hitting zero rows for every brand-new ID, so every call to
    // this handler previously returned a 500.
    state
        .execution_repo
        .create(execution_id, workflow_id, execution_state.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create execution: {}", e)))?;

    // Build response
    let response = ExecutionResponse {
        id: execution_id.to_string(),
        workflow_id: id,
        status: "pending".to_string(),
        started_at: now,
        finished_at: None,
        input: payload.input,
        output: None,
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Validate workflow definition without executing it.
///
/// Loads the stored workflow, deserializes it as a
/// [`nebula_workflow::WorkflowDefinition`], and runs structural validation
/// (DAG cycle check, node references, schema version, etc.).
///
/// Always returns **200 OK**. The response body indicates the outcome:
/// - `{valid: true, errors: []}` — definition is structurally valid.
/// - `{valid: false, errors: ["…"]}` — definition has validation errors.
///
/// A 422 is only returned when the stored JSON cannot be parsed at all (i.e.
/// the blob is not a `WorkflowDefinition`), which is treated as a validation
/// error rather than a not-found condition.
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid workflow ID or the definition cannot be
///   parsed.
/// - [`ApiError::NotFound`] if the workflow does not exist.
/// - [`ApiError::Internal`] if the repository is unavailable.
pub async fn validate_workflow_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowValidateResponse>> {
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    let definition = state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Deserialise the stored JSON into a WorkflowDefinition.
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_value(definition)
        .map_err(|e| {
            ApiError::validation_message(format!(
                "Workflow definition cannot be parsed as WorkflowDefinition: {}",
                e
            ))
        })?;

    let errors = nebula_workflow::validate_workflow(&workflow_def);
    if errors.is_empty() {
        Ok(Json(WorkflowValidateResponse {
            valid: true,
            errors: vec![],
        }))
    } else {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        Ok(Json(WorkflowValidateResponse {
            valid: false,
            errors: error_messages,
        }))
    }
}
