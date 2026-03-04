//! Workflow HTTP handlers.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    auth::Authenticated,
    contracts::{CreateWorkflowRequest, PaginatedResponse, PaginationQuery, UpdateWorkflowRequest},
    error::ApiResult,
    services::workflows::WorkflowService,
    state::ApiState,
};

pub(crate) async fn list_workflows(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Query(query): Query<PaginationQuery>,
) -> ApiResult<impl IntoResponse> {
    let service = WorkflowService::from_state(&state)?;
    let (offset, limit) = WorkflowService::normalize_pagination(query.offset, query.limit);
    let items = service.list(offset, limit).await?;
    Ok(Json(PaginatedResponse {
        items,
        offset,
        limit,
    }))
}

pub(crate) async fn get_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let service = WorkflowService::from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id)?;
    let detail = service.get(workflow_id, &id).await?;
    Ok(Json(detail))
}

pub(crate) async fn create_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Json(req): Json<CreateWorkflowRequest>,
) -> ApiResult<impl IntoResponse> {
    let service = WorkflowService::from_state(&state)?;
    let detail = service.create(req).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

pub(crate) async fn update_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkflowRequest>,
) -> ApiResult<impl IntoResponse> {
    let service = WorkflowService::from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id)?;
    let detail = service.update(workflow_id, &id, req).await?;
    Ok(Json(detail))
}

pub(crate) async fn delete_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let service = WorkflowService::from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id)?;
    service.delete(workflow_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
