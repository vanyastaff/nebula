//! Workflow handlers

use crate::{
    errors::{ApiError, ApiResult},
    models::{
        CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowRequest, WorkflowResponse,
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use nebula_core::WorkflowId;
use serde::Deserialize;

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
    fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    /// Get validated limit (capped at 100)
    fn limit(&self) -> usize {
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

    // Get current timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

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
    State(_state): State<AppState>,
    Path(id): Path<String>,
    Json(_payload): Json<UpdateWorkflowRequest>,
) -> ApiResult<Json<WorkflowResponse>> {
    // TODO: Implement via workflow_repo.update()
    Err(ApiError::NotFound(format!("Workflow {} not found", id)))
}

/// Delete workflow
/// DELETE /api/v1/workflows/:id
pub async fn delete_workflow(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    // TODO: Implement via workflow_repo.delete()
    Err(ApiError::NotFound(format!("Workflow {} not found", id)))
}
