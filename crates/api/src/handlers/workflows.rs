//! Workflow HTTP handlers.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    errors::{ApiHttpError, ApiResult},
    extractors::Authenticated,
    models::{CreateWorkflowRequest, PaginatedResponse, PaginationQuery, UpdateWorkflowRequest},
    services::{error::ServiceError, workflows::WorkflowService},
    state::ApiState,
};

fn workflow_service_from_state(state: &ApiState) -> ApiResult<WorkflowService> {
    let repo = state.workflow_repo.clone().ok_or_else(|| {
        ApiHttpError::service_unavailable(
            "workflow_repo_unavailable",
            "workflow repository is not configured",
        )
    })?;
    Ok(WorkflowService::new(repo))
}

fn map_service_error(error: ServiceError) -> ApiHttpError {
    match error {
        ServiceError::InvalidInput { code, message } => ApiHttpError::bad_request(code, message),
        ServiceError::NotFound { code, message } => ApiHttpError::not_found(code, message),
        ServiceError::Conflict { code, message } => ApiHttpError::conflict(code, message),
        ServiceError::Internal { code, message } => ApiHttpError::internal(code, message),
    }
}

pub(crate) async fn list_workflows(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Query(query): Query<PaginationQuery>,
) -> ApiResult<impl IntoResponse> {
    let service = workflow_service_from_state(&state)?;
    let (offset, limit) = WorkflowService::normalize_pagination(query.offset, query.limit);
    let items = service
        .list(offset, limit)
        .await
        .map_err(map_service_error)?;
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
    let service = workflow_service_from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id).map_err(map_service_error)?;
    let detail = service
        .get(workflow_id, &id)
        .await
        .map_err(map_service_error)?;
    Ok(Json(detail))
}

pub(crate) async fn create_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Json(req): Json<CreateWorkflowRequest>,
) -> ApiResult<impl IntoResponse> {
    let service = workflow_service_from_state(&state)?;
    let detail = service.create(req).await.map_err(map_service_error)?;
    Ok((StatusCode::CREATED, Json(detail)))
}

pub(crate) async fn update_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkflowRequest>,
) -> ApiResult<impl IntoResponse> {
    let service = workflow_service_from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id).map_err(map_service_error)?;
    let detail = service
        .update(workflow_id, &id, req)
        .await
        .map_err(map_service_error)?;
    Ok(Json(detail))
}

pub(crate) async fn delete_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let service = workflow_service_from_state(&state)?;
    let workflow_id = WorkflowService::parse_workflow_id(&id).map_err(map_service_error)?;
    service
        .delete(workflow_id, &id)
        .await
        .map_err(map_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
