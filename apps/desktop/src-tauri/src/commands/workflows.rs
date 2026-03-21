use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tauri_plugin_store::StoreExt;

use crate::types::{CreateWorkflowRequest, UpdateWorkflowRequest, Workflow, WorkflowMetadata};

const STORE_PATH: &str = "nebula-workflows.json";
const KEY_PREFIX: &str = "workflow_";
const KEY_LIST: &str = "workflow_ids";

/// Load all workflows from storage
fn load_all(app: &AppHandle) -> Vec<Workflow> {
    let Ok(store) = app.store(STORE_PATH) else {
        return Vec::new();
    };

    let ids: Vec<String> = store
        .get(KEY_LIST)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    ids.iter()
        .filter_map(|id| {
            let key = format!("{}{}", KEY_PREFIX, id);
            store
                .get(&key)
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .collect()
}

/// Load a single workflow by ID
fn load_one(app: &AppHandle, id: &str) -> Option<Workflow> {
    let store = app.store(STORE_PATH).ok()?;
    let key = format!("{}{}", KEY_PREFIX, id);
    store
        .get(&key)
        .and_then(|v| serde_json::from_value(v).ok())
}

/// Save workflow and update ID list
fn save_workflow(app: &AppHandle, workflow: &Workflow) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;

    // Save workflow
    let key = format!("{}{}", KEY_PREFIX, workflow.id);
    store.set(&key, json!(workflow));

    // Update ID list
    let mut ids: Vec<String> = store
        .get(KEY_LIST)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    if !ids.contains(&workflow.id) {
        ids.push(workflow.id.clone());
        store.set(KEY_LIST, json!(ids));
    }

    store.save().map_err(|e| e.to_string())
}

/// Remove workflow and update ID list
fn remove_workflow(app: &AppHandle, id: &str) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;

    // Remove workflow
    let key = format!("{}{}", KEY_PREFIX, id);
    store.delete(&key);

    // Update ID list
    let mut ids: Vec<String> = store
        .get(KEY_LIST)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    ids.retain(|existing_id| existing_id != id);
    store.set(KEY_LIST, json!(ids));

    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_workflows(app: AppHandle) -> Vec<Workflow> {
    load_all(&app)
}

#[tauri::command]
#[specta::specta]
pub async fn get_workflow(id: String, app: AppHandle) -> Result<Workflow, String> {
    load_one(&app, &id).ok_or_else(|| format!("Workflow not found: {}", id))
}

#[tauri::command]
#[specta::specta]
pub async fn create_workflow(
    request: CreateWorkflowRequest,
    app: AppHandle,
) -> Result<Workflow, String> {
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();

    let workflow = Workflow {
        id: id.clone(),
        name: request.name,
        status: "draft".to_string(),
        trigger_mode: request.trigger_mode,
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: WorkflowMetadata {
            created_at: now.to_rfc3339(),
            last_modified: now.to_rfc3339(),
            last_deployed: None,
            last_executed: None,
            version: 1,
            tags: request.tags.unwrap_or_default(),
            author: None,
            description: request.description,
        },
        server_url: None,
    };

    save_workflow(&app, &workflow)?;
    app.emit("workflow_created", &workflow)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(workflow)
}

#[tauri::command]
#[specta::specta]
pub async fn update_workflow(
    id: String,
    request: UpdateWorkflowRequest,
    app: AppHandle,
) -> Result<Workflow, String> {
    let mut workflow = load_one(&app, &id)
        .ok_or_else(|| format!("Workflow not found: {}", id))?;

    // Update fields if provided
    if let Some(name) = request.name {
        workflow.name = name;
    }
    if let Some(status) = request.status {
        workflow.status = status;
    }
    if let Some(trigger_mode) = request.trigger_mode {
        workflow.trigger_mode = trigger_mode;
    }
    if let Some(nodes) = request.nodes {
        workflow.nodes = nodes;
    }
    if let Some(edges) = request.edges {
        workflow.edges = edges;
    }
    if let Some(tags) = request.tags {
        workflow.metadata.tags = tags;
    }
    if let Some(description) = request.description {
        workflow.metadata.description = Some(description);
    }
    if let Some(server_url) = request.server_url {
        workflow.server_url = Some(server_url);
    }

    // Update metadata
    workflow.metadata.last_modified = Utc::now().to_rfc3339();
    workflow.metadata.version += 1;

    save_workflow(&app, &workflow)?;
    app.emit("workflow_updated", &workflow)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(workflow)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_workflow(id: String, app: AppHandle) -> Result<(), String> {
    // Verify workflow exists before deleting
    load_one(&app, &id).ok_or_else(|| format!("Workflow not found: {}", id))?;

    remove_workflow(&app, &id)?;
    app.emit("workflow_deleted", &id)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}
