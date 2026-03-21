use chrono::Utc;
use serde_json::json;
use std::fs;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_store::StoreExt;

use crate::types::{
    CreateWorkflowRequest, PluginAction, UpdateWorkflowRequest, Workflow, WorkflowMetadata,
};

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

/// Get available plugin actions from registry
///
/// Returns a list of available plugin actions that can be used in workflows.
/// Currently returns a hardcoded list of common plugins. In the future, this
/// will be extended to load from an actual PluginRegistry or remote server.
fn get_available_plugins() -> Vec<PluginAction> {
    vec![
        PluginAction {
            key: "http_request".to_string(),
            name: "HTTP Request".to_string(),
            description: "Make HTTP calls to external APIs".to_string(),
            version: 1,
            group: vec!["Network".to_string()],
            icon: Some("globe".to_string()),
            icon_url: None,
            color: Some("#3b82f6".to_string()),
            tags: vec!["http".to_string(), "api".to_string(), "network".to_string()],
        },
        PluginAction {
            key: "delay".to_string(),
            name: "Delay".to_string(),
            description: "Wait for a specified duration before continuing".to_string(),
            version: 1,
            group: vec!["Flow Control".to_string()],
            icon: Some("clock".to_string()),
            icon_url: None,
            color: Some("#8b5cf6".to_string()),
            tags: vec!["delay".to_string(), "wait".to_string(), "sleep".to_string()],
        },
        PluginAction {
            key: "transform".to_string(),
            name: "Transform Data".to_string(),
            description: "Transform and manipulate data using expressions".to_string(),
            version: 1,
            group: vec!["Data".to_string()],
            icon: Some("refresh-cw".to_string()),
            icon_url: None,
            color: Some("#10b981".to_string()),
            tags: vec!["transform".to_string(), "map".to_string(), "data".to_string()],
        },
        PluginAction {
            key: "condition".to_string(),
            name: "Condition".to_string(),
            description: "Branch workflow based on conditional logic".to_string(),
            version: 1,
            group: vec!["Flow Control".to_string()],
            icon: Some("git-branch".to_string()),
            icon_url: None,
            color: Some("#f59e0b".to_string()),
            tags: vec!["if".to_string(), "branch".to_string(), "condition".to_string()],
        },
        PluginAction {
            key: "loop".to_string(),
            name: "Loop".to_string(),
            description: "Iterate over a list of items".to_string(),
            version: 1,
            group: vec!["Flow Control".to_string()],
            icon: Some("repeat".to_string()),
            icon_url: None,
            color: Some("#ec4899".to_string()),
            tags: vec!["loop".to_string(), "foreach".to_string(), "iterate".to_string()],
        },
        PluginAction {
            key: "webhook".to_string(),
            name: "Webhook".to_string(),
            description: "Trigger workflow from external webhook events".to_string(),
            version: 1,
            group: vec!["Triggers".to_string()],
            icon: Some("webhook".to_string()),
            icon_url: None,
            color: Some("#06b6d4".to_string()),
            tags: vec!["webhook".to_string(), "trigger".to_string(), "event".to_string()],
        },
        PluginAction {
            key: "schedule".to_string(),
            name: "Schedule".to_string(),
            description: "Trigger workflow on a schedule using cron expressions".to_string(),
            version: 1,
            group: vec!["Triggers".to_string()],
            icon: Some("calendar".to_string()),
            icon_url: None,
            color: Some("#84cc16".to_string()),
            tags: vec!["cron".to_string(), "schedule".to_string(), "timer".to_string()],
        },
        PluginAction {
            key: "email".to_string(),
            name: "Send Email".to_string(),
            description: "Send email notifications via SMTP".to_string(),
            version: 1,
            group: vec!["Notifications".to_string()],
            icon: Some("mail".to_string()),
            icon_url: None,
            color: Some("#ef4444".to_string()),
            tags: vec!["email".to_string(), "smtp".to_string(), "notification".to_string()],
        },
        PluginAction {
            key: "slack".to_string(),
            name: "Slack".to_string(),
            description: "Send messages to Slack channels".to_string(),
            version: 1,
            group: vec!["Notifications".to_string()],
            icon: Some("message-square".to_string()),
            icon_url: None,
            color: Some("#4a154b".to_string()),
            tags: vec!["slack".to_string(), "chat".to_string(), "notification".to_string()],
        },
        PluginAction {
            key: "database_query".to_string(),
            name: "Database Query".to_string(),
            description: "Execute SQL queries against databases".to_string(),
            version: 1,
            group: vec!["Database".to_string()],
            icon: Some("database".to_string()),
            icon_url: None,
            color: Some("#6366f1".to_string()),
            tags: vec!["sql".to_string(), "database".to_string(), "query".to_string()],
        },
    ]
}

#[tauri::command]
#[specta::specta]
pub async fn list_plugin_actions() -> Vec<PluginAction> {
    get_available_plugins()
}

/// Save a workflow to a local file via file dialog
#[tauri::command]
#[specta::specta]
pub async fn save_workflow_to_file(id: String, app: AppHandle) -> Result<String, String> {
    // Load the workflow from store
    let workflow = load_one(&app, &id)
        .ok_or_else(|| format!("Workflow not found: {}", id))?;

    // Get the main window for the file dialog parent
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;

    // Show save file dialog
    let file_path = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Save Workflow")
        .add_filter("Nebula Workflow", &["json"])
        .set_file_name(&format!("{}.json", workflow.name))
        .blocking_save_file();

    // If user selected a file path, write the workflow JSON
    if let Some(path) = file_path {
        let json = serde_json::to_string_pretty(&workflow)
            .map_err(|e| format!("Failed to serialize workflow: {}", e))?;

        let path_ref = path.as_path()
            .ok_or_else(|| "Invalid file path".to_string())?;

        fs::write(path_ref, json)
            .map_err(|e| format!("Failed to write file: {}", e))?;

        Ok(path.to_string())
    } else {
        Err("Save cancelled by user".to_string())
    }
}

/// Load a workflow from a local file via file dialog
#[tauri::command]
#[specta::specta]
pub async fn load_workflow_from_file(app: AppHandle) -> Result<Workflow, String> {
    // Get the main window for the file dialog parent
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;

    // Show open file dialog
    let file_path = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Load Workflow")
        .add_filter("Nebula Workflow", &["json"])
        .blocking_pick_file();

    // If user selected a file path, read and parse the workflow JSON
    if let Some(path) = file_path {
        let path_ref = path.as_path()
            .ok_or_else(|| "Invalid file path".to_string())?;

        let json = fs::read_to_string(path_ref)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let workflow: Workflow = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse workflow JSON: {}", e))?;

        Ok(workflow)
    } else {
        Err("Load cancelled by user".to_string())
    }
}

/// Deploy a workflow to a Nebula server via HTTP API
///
/// Makes a POST request to the server's workflow deployment endpoint with
/// the workflow definition as JSON payload. Updates the workflow's server_url
/// and last_deployed timestamp on success.
#[tauri::command]
#[specta::specta]
pub async fn deploy_workflow(
    id: String,
    server_url: String,
    app: AppHandle,
) -> Result<Workflow, String> {
    // Load the workflow
    let mut workflow = load_one(&app, &id)
        .ok_or_else(|| format!("Workflow not found: {}", id))?;

    // Build the deployment payload (WorkflowDefinition format)
    let definition = json!({
        "id": workflow.id,
        "name": workflow.name,
        "description": workflow.metadata.description,
        "version": {
            "major": workflow.metadata.version,
            "minor": 0,
            "patch": 0,
        },
        "nodes": workflow.nodes,
        "connections": workflow.edges,
        "variables": {},
        "config": {
            "timeout": null,
            "max_parallel_nodes": 10,
            "checkpointing": {
                "enabled": true,
                "interval": null,
            },
            "retry_policy": null,
        },
        "tags": workflow.metadata.tags,
        "metadata": {
            "author": workflow.metadata.author,
            "created_at": workflow.metadata.created_at,
            "last_modified": workflow.metadata.last_modified,
        },
    });

    // Make HTTP POST request to server
    let endpoint = format!("{}/api/workflows", server_url.trim_end_matches('/'));
    let client = reqwest::Client::new();

    let response = client
        .post(&endpoint)
        .json(&definition)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to server: {}", e))?;

    // Check response status
    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Server returned error {}: {}",
            status, error_body
        ));
    }

    // Update workflow metadata
    workflow.server_url = Some(server_url);
    workflow.metadata.last_deployed = Some(Utc::now().to_rfc3339());
    workflow.metadata.last_modified = Utc::now().to_rfc3339();

    // Save updated workflow
    save_workflow(&app, &workflow)?;
    app.emit("workflow_updated", &workflow)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(workflow)
}
