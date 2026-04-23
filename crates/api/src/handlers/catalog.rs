//! Action and plugin catalog handlers

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    errors::{ApiError, ApiResult},
    models::{
        ActionDetailResponse, ActionSummary, ListActionsResponse, ListPluginsResponse,
        PluginDetailResponse, PluginSummary,
    },
    state::AppState,
};

/// List all registered actions.
///
/// Returns key, name, and version for every action in the action registry.
/// Requires an [`ActionRegistry`](nebula_engine::ActionRegistry) to be
/// attached to [`AppState`] via [`AppState::with_action_registry`].
///
/// # Errors
///
/// Returns [`ApiError::ServiceUnavailable`] if no action registry is configured.
pub async fn list_actions(State(state): State<AppState>) -> ApiResult<Json<ListActionsResponse>> {
    let registry = state
        .action_registry
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Action registry not configured".into()))?;

    let actions: Vec<ActionSummary> = registry
        .keys()
        .into_iter()
        .map(|key| {
            let entry = registry.get(&key);
            let name = entry.as_ref().map_or_else(
                || key.as_str().to_string(),
                |(meta, _)| meta.base.name.clone(),
            );
            let version = entry.map_or_else(
                || "1.0.0".to_string(),
                |(meta, _)| meta.base.version.to_string(),
            );
            ActionSummary {
                key: key.as_str().to_string(),
                name,
                version,
            }
        })
        .collect();

    Ok(Json(ListActionsResponse { actions }))
}

/// Get detail for a specific action by key.
///
/// Returns the action's metadata including description, version, and
/// isolation level.
///
/// # Errors
///
/// - [`ApiError::ServiceUnavailable`] if no action registry is configured.
/// - [`ApiError::NotFound`] if the action key is not registered.
pub async fn get_action(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<ActionDetailResponse>> {
    let registry = state
        .action_registry
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Action registry not configured".into()))?;

    let action_key = nebula_core::ActionKey::new(&key)
        .map_err(|e| ApiError::validation_message(format!("Invalid action key: {e}")))?;

    let (meta, _handler) = registry
        .get(&action_key)
        .ok_or_else(|| ApiError::NotFound(format!("Action '{key}' not found")))?;

    Ok(Json(ActionDetailResponse {
        key: meta.base.key.as_str().to_string(),
        name: meta.base.name.clone(),
        description: meta.base.description.clone(),
        version: meta.base.version.to_string(),
        // IsolationLevel does not implement Display; {:?} produces the variant name.
        isolation_level: format!("{:?}", meta.isolation_level),
    }))
}

/// List all registered plugins.
///
/// Returns key, name, and version for every plugin in the plugin registry.
/// Requires a [`PluginRegistry`](nebula_plugin::PluginRegistry) to be attached
/// to [`AppState`] via [`AppState::with_plugin_registry`].
///
/// # Errors
///
/// Returns [`ApiError::ServiceUnavailable`] if no plugin registry is configured.
pub async fn list_plugins(State(state): State<AppState>) -> ApiResult<Json<ListPluginsResponse>> {
    let registry_lock = state
        .plugin_registry
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Plugin registry not configured".into()))?;

    let registry = registry_lock.read().await;

    let plugins: Vec<PluginSummary> = registry
        .iter()
        .map(|(key, resolved)| {
            let manifest = resolved.manifest();
            PluginSummary {
                key: key.as_str().to_string(),
                name: manifest.name().to_string(),
                version: resolved.version().to_string(),
            }
        })
        .collect();

    Ok(Json(ListPluginsResponse { plugins }))
}

/// Get detail for a specific plugin by key.
///
/// Returns plugin metadata including description, group, tags, and available
/// version numbers.
///
/// # Errors
///
/// - [`ApiError::ServiceUnavailable`] if no plugin registry is configured.
/// - [`ApiError::NotFound`] if the plugin key is not registered.
pub async fn get_plugin(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<PluginDetailResponse>> {
    let registry_lock = state
        .plugin_registry
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Plugin registry not configured".into()))?;

    let plugin_key: nebula_core::PluginKey = key
        .parse()
        .map_err(|e| ApiError::validation_message(format!("Invalid plugin key: {e}")))?;

    let registry = registry_lock.read().await;

    let resolved = registry
        .get(&plugin_key)
        .ok_or_else(|| ApiError::NotFound(format!("Plugin '{key}' not found")))?;

    let manifest = resolved.manifest();

    Ok(Json(PluginDetailResponse {
        key: manifest.key().as_str().to_string(),
        name: manifest.name().to_string(),
        description: manifest.description().to_string(),
        version: resolved.version().to_string(),
        group: manifest.group().to_vec(),
        tags: manifest.tags().to_vec(),
        icon_url: manifest.icon().as_url().map(str::to_string),
        author: manifest.author().map(str::to_string),
        license: manifest.license().map(str::to_string),
    }))
}
