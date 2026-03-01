use serde_json::json;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_store::StoreExt;

use crate::types::{AuthState, AuthStatus, UserProfile};

const STORE_PATH: &str = "nebula-auth.json";
const KEY: &str = "auth";

pub fn load(app: &AppHandle) -> AuthState {
    let fallback = AuthState {
        status: AuthStatus::SignedOut,
        provider: None,
        access_token: String::new(),
        user: None,
        error: None,
    };
    let Ok(store) = app.store(STORE_PATH) else {
        return fallback;
    };
    store
        .get(KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(fallback)
}

pub fn save_and_emit(app: &AppHandle, state: &AuthState) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.set(KEY, json!(state));
    store.save().map_err(|e| e.to_string())?;
    app.emit("auth_state_changed", state).map_err(|e: tauri::Error| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_auth_state(app: AppHandle) -> AuthState {
    // Never restore a transient "authorizing" state across restarts.
    let mut state = load(&app);
    if state.status == AuthStatus::Authorizing {
        state.status = AuthStatus::SignedOut;
    }
    state
}

#[tauri::command]
#[specta::specta]
pub async fn start_oauth(
    provider: String,
    api_base_url: String,
    app: AppHandle,
) -> Result<(), String> {
    let mut state = load(&app);
    state.status = AuthStatus::Authorizing;
    state.provider = Some(provider.clone());
    state.error = None;
    save_and_emit(&app, &state)?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{api_base_url}/auth/oauth/start"))
        .json(&json!({ "provider": provider, "redirectUri": "nebula://auth/callback" }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let msg = format!("oauth start failed: {}", response.status());
        let mut state = load(&app);
        state.status = AuthStatus::SignedOut;
        state.error = Some(msg.clone());
        save_and_emit(&app, &state)?;
        return Err(msg);
    }

    let payload: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;

    // Backend returned a direct token (mock / test flow)
    if let Some(token) = payload.get("accessToken").and_then(|v| v.as_str()) {
        let user: Option<UserProfile> = payload
            .get("user")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        complete_sign_in(token.to_string(), Some(provider), user, &app)?;
        return Ok(());
    }

    // Backend returned an OAuth URL — open in browser, wait for deep-link callback
    if let Some(url) = payload.get("authUrl").and_then(|v| v.as_str()) {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Called by the deep-link handler after receiving nebula://auth/callback?code=…
pub async fn exchange_code(
    code: String,
    provider: String,
    api_base_url: String,
    app: AppHandle,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{api_base_url}/auth/oauth/callback"))
        .json(&json!({
            "provider": provider,
            "code": code,
            "redirectUri": "nebula://auth/callback"
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or(json!({}));
        let detail = body
            .get("message")
            .or_else(|| body.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let msg = if detail.is_empty() {
            format!("oauth callback failed: {status}")
        } else {
            format!("oauth callback failed: {status} ({detail})")
        };
        let mut state = load(&app);
        state.status = AuthStatus::SignedOut;
        state.error = Some(msg.clone());
        save_and_emit(&app, &state)?;
        return Err(msg);
    }

    let payload: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let token = payload
        .get("accessToken")
        .or_else(|| payload.get("access_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let user: Option<UserProfile> = payload
        .get("user")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    complete_sign_in(token, Some(provider), user, &app)
}

pub fn complete_sign_in(
    token: String,
    provider: Option<String>,
    user: Option<UserProfile>,
    app: &AppHandle,
) -> Result<(), String> {
    let token = token.trim().to_string();
    let status = if token.is_empty() {
        AuthStatus::SignedOut
    } else {
        AuthStatus::SignedIn
    };
    let state = AuthState {
        status,
        provider,
        access_token: token,
        user,
        error: None,
    };
    save_and_emit(app, &state)
}

#[tauri::command]
#[specta::specta]
pub async fn sign_out(app: AppHandle) -> Result<(), String> {
    let state = AuthState {
        status: AuthStatus::SignedOut,
        provider: None,
        access_token: String::new(),
        user: None,
        error: None,
    };
    save_and_emit(&app, &state)
}
