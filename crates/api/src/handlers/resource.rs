//! Resource management handlers.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::models::resource::{ResourceListResponse, ResourceMetrics, ResourceSummary};
use crate::state::AppState;

/// List all registered resources with their state and metrics.
///
/// `GET /api/v1/resources`
pub async fn list_resources(
    State(state): State<AppState>,
) -> Result<Json<ResourceListResponse>, (StatusCode, &'static str)> {
    let manager = state.resource_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "resource manager not configured",
    ))?;

    let snapshots = manager.health_all();
    let count = snapshots.len();

    let resources: Vec<ResourceSummary> = snapshots
        .into_iter()
        .map(|s| ResourceSummary {
            key: s.key.to_string(),
            topology: s.topology.as_str().to_owned(),
            phase: s.phase.to_string(),
            generation: s.generation,
        })
        .collect();

    let metrics = manager.metrics().map(|m| {
        let snap = m.snapshot();
        ResourceMetrics {
            acquire_total: snap.acquire_total,
            acquire_errors: snap.acquire_errors,
            release_total: snap.release_total,
            create_total: snap.create_total,
            destroy_total: snap.destroy_total,
        }
    });

    Ok(Json(ResourceListResponse {
        resources,
        count,
        is_shutdown: manager.is_shutdown(),
        metrics,
    }))
}
