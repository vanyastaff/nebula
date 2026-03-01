use serde_json::json;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::types::{ConnectionConfig, ConnectionMode};

const STORE_PATH: &str = "nebula-connection.json";
const KEY: &str = "connection";

fn default() -> ConnectionConfig {
    ConnectionConfig {
        mode: ConnectionMode::Local,
        local_base_url: "http://localhost:5678".to_string(),
        remote_base_url: String::new(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_connection(app: AppHandle) -> ConnectionConfig {
    let Ok(store) = app.store(STORE_PATH) else {
        return default();
    };
    store
        .get(KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(default)
}

#[tauri::command]
#[specta::specta]
pub async fn set_connection(config: ConnectionConfig, app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.set(KEY, json!(config));
    store.save().map_err(|e| e.to_string())
}
