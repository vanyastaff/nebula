//! Action and plugin catalog handlers

use crate::{
    errors::{ApiError, ApiResult},
    models::{
        ActionDetailResponse, ActionSummary, ListActionsResponse, ListPluginsResponse,
        PluginDetailResponse, PluginSummary,
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
};

/// List all registered actions.
///
/// Returns key, name, and version for every action in the action registry.
/// Requires an [`ActionRegistry`](nebula_action::registry::ActionRegistry) to be
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
        .map(|key| {
            let entry = registry.get(key);
            let name = entry
                .as_ref()
                .map(|(meta, _)| meta.name.clone())
                .unwrap_or_else(|| key.as_str().to_string());
            let version = entry
                .map(|(meta, _)| format!("{}.{}", meta.version.major, meta.version.minor))
                .unwrap_or_else(|| "1.0".to_string());
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
        .map_err(|e| ApiError::validation_message(format!("Invalid action key: {}", e)))?;

    let (meta, _handler) = registry
        .get(&action_key)
        .ok_or_else(|| ApiError::NotFound(format!("Action '{}' not found", key)))?;

    Ok(Json(ActionDetailResponse {
        key: meta.key.as_str().to_string(),
        name: meta.name.clone(),
        description: meta.description.clone(),
        version: format!("{}.{}", meta.version.major, meta.version.minor),
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
        .keys()
        .into_iter()
        .map(|key| {
            let pt = registry.get(&key).map_err(|e| {
                ApiError::Internal(format!("Failed to get plugin '{}': {}", key.as_str(), e))
            })?;
            let plugin = pt.latest().ok();
            let (name, version) = plugin
                .as_ref()
                .map(|p| (p.name().to_string(), p.version()))
                .unwrap_or_else(|| (key.as_str().to_string(), 1));
            Ok(PluginSummary {
                key: key.as_str().to_string(),
                name,
                version,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

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
        .map_err(|e| ApiError::validation_message(format!("Invalid plugin key: {}", e)))?;

    let registry = registry_lock.read().await;

    let plugin_type = registry
        .get(&plugin_key)
        .map_err(|_| ApiError::NotFound(format!("Plugin '{}' not found", key)))?;

    let versions = plugin_type.version_numbers();
    let latest = plugin_type
        .latest()
        .map_err(|e| ApiError::Internal(format!("Failed to get plugin: {}", e)))?;

    let meta = latest.metadata();

    Ok(Json(PluginDetailResponse {
        key: meta.key().as_str().to_string(),
        name: meta.name().to_string(),
        description: meta.description().to_string(),
        version: latest.version(),
        versions,
        group: meta.group().to_vec(),
        tags: meta.tags().to_vec(),
        icon_url: meta.icon_url().map(str::to_string),
        documentation_url: meta.documentation_url().map(str::to_string),
        author: meta.author().map(str::to_string),
        license: meta.license().map(str::to_string),
    }))
}
