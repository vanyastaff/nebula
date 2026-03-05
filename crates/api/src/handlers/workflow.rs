//! Workflow handlers

use crate::{
    errors::{ApiError, ApiResult},
    models::{CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowRequest, WorkflowResponse},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

/// List workflows
/// GET /api/v1/workflows
pub async fn list_workflows(
    State(_state): State<AppState>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    // TODO: Implement via workflow_repo.list()
    Ok(Json(ListWorkflowsResponse {
        workflows: vec![],
        total: 0,
        page: 1,
        page_size: 10,
    }))
}

/// Get workflow by ID
/// GET /api/v1/workflows/:id
pub async fn get_workflow(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowResponse>> {
    // TODO: Implement via workflow_repo.get()
    Err(ApiError::NotFound(format!("Workflow {} not found", id)))
}

/// Create workflow
/// POST /api/v1/workflows
pub async fn create_workflow(
    State(_state): State<AppState>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<WorkflowResponse>)> {
    // TODO: Validate and create via workflow_repo.create()
    let _name = payload.name;
    Err(ApiError::Internal("Not implemented yet".to_string()))
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

