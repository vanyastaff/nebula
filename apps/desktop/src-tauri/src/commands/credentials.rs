use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tauri_plugin_store::StoreExt;

use crate::types::{Credential, CredentialMetadata, CreateCredentialRequest, UpdateCredentialRequest};

const STORE_PATH: &str = "nebula-credentials.json";
const KEY_PREFIX: &str = "credential_";
const KEY_LIST: &str = "credential_ids";

/// Load all credentials from storage
fn load_all(app: &AppHandle) -> Vec<Credential> {
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

/// Load a single credential by ID
fn load_one(app: &AppHandle, id: &str) -> Option<Credential> {
    let store = app.store(STORE_PATH).ok()?;
    let key = format!("{}{}", KEY_PREFIX, id);
    store
        .get(&key)
        .and_then(|v| serde_json::from_value(v).ok())
}

/// Save credential and update ID list
fn save_credential(app: &AppHandle, credential: &Credential) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;

    // Save credential
    let key = format!("{}{}", KEY_PREFIX, credential.id);
    store.set(&key, json!(credential));

    // Update ID list
    let mut ids: Vec<String> = store
        .get(KEY_LIST)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    if !ids.contains(&credential.id) {
        ids.push(credential.id.clone());
        store.set(KEY_LIST, json!(ids));
    }

    store.save().map_err(|e| e.to_string())
}

/// Remove credential and update ID list
fn remove_credential(app: &AppHandle, id: &str) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;

    // Remove credential
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
pub async fn list_credentials(app: AppHandle) -> Vec<Credential> {
    load_all(&app)
}

#[tauri::command]
#[specta::specta]
pub async fn get_credential(id: String, app: AppHandle) -> Result<Credential, String> {
    load_one(&app, &id).ok_or_else(|| format!("Credential not found: {}", id))
}

#[tauri::command]
#[specta::specta]
pub async fn create_credential(
    request: CreateCredentialRequest,
    app: AppHandle,
) -> Result<Credential, String> {
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();

    let credential = Credential {
        id: id.clone(),
        name: request.name,
        kind: request.kind,
        metadata: CredentialMetadata {
            created_at: now.to_rfc3339(),
            last_accessed: None,
            last_modified: now.to_rfc3339(),
            version: 1,
            expires_at: None,
            ttl_seconds: None,
            tags: request.tags.unwrap_or_default(),
        },
        state: request.state,
    };

    save_credential(&app, &credential)?;
    app.emit("credential_created", &credential)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(credential)
}

#[tauri::command]
#[specta::specta]
pub async fn update_credential(
    id: String,
    request: UpdateCredentialRequest,
    app: AppHandle,
) -> Result<Credential, String> {
    let mut credential = load_one(&app, &id)
        .ok_or_else(|| format!("Credential not found: {}", id))?;

    // Update fields if provided
    if let Some(name) = request.name {
        credential.name = name;
    }
    if let Some(state) = request.state {
        credential.state = state;
    }
    if let Some(tags) = request.tags {
        credential.metadata.tags = tags;
    }

    // Update metadata
    credential.metadata.last_modified = Utc::now().to_rfc3339();

    save_credential(&app, &credential)?;
    app.emit("credential_updated", &credential)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(credential)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_credential(id: String, app: AppHandle) -> Result<(), String> {
    // Verify credential exists before deleting
    load_one(&app, &id).ok_or_else(|| format!("Credential not found: {}", id))?;

    remove_credential(&app, &id)?;
    app.emit("credential_deleted", &id)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn rotate_credential(id: String, app: AppHandle) -> Result<Credential, String> {
    let mut credential = load_one(&app, &id)
        .ok_or_else(|| format!("Credential not found: {}", id))?;

    // Update metadata with rotation timestamp
    let now = Utc::now();
    credential.metadata.last_modified = now.to_rfc3339();
    credential.metadata.version += 1;

    // Note: In a real implementation, this would:
    // 1. Generate new credential values based on the protocol type
    // 2. Update the encrypted state with new values
    // 3. Optionally revoke the old credentials with the provider
    // For now, we just update the metadata to simulate rotation

    save_credential(&app, &credential)?;
    app.emit("credential_rotated", &credential)
        .map_err(|e: tauri::Error| e.to_string())?;

    Ok(credential)
}
