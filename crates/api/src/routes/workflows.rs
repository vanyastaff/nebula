//! Workflow route group.

use axum::{Router, routing::get};

use crate::{handlers::workflows, state::ApiState};

pub(super) fn v1_routes() -> Router<ApiState> {
    Router::new()
        .route(
            "/workflows",
            get(workflows::list_workflows).post(workflows::create_workflow),
        )
        .route(
            "/workflows/{id}",
            get(workflows::get_workflow)
                .patch(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
}
