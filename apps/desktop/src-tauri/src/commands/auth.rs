use tauri::AppHandle;

use crate::auth;
use crate::error::AppError;
use crate::types::{AuthState, AuthStatus, UserProfile};

#[tauri::command]
#[specta::specta]
pub async fn get_auth_state(_app: AppHandle) -> AuthState {
    // Reconstruct state from keyring — never persist transient "authorizing" status.
    match auth::keyring::get_access_token().await {
        Ok(token) if !token.is_empty() => AuthState {
            status: AuthStatus::SignedIn,
            provider: None,
            access_token: token,
            user: None,
            error: None,
        },
        _ => AuthState {
            status: AuthStatus::SignedOut,
            provider: None,
            access_token: String::new(),
            user: None,
            error: None,
        },
    }
}

#[tauri::command]
#[specta::specta]
pub async fn auth_login(provider: String, app: AppHandle) -> Result<AuthState, AppError> {
    auth::login(&provider, &app).await
}

#[tauri::command]
#[specta::specta]
pub async fn auth_logout(app: AppHandle) -> Result<(), AppError> {
    auth::logout(&app).await
}

#[tauri::command]
#[specta::specta]
pub async fn auth_get_user(provider: String) -> Result<UserProfile, AppError> {
    auth::get_user(&provider).await
}

#[tauri::command]
#[specta::specta]
pub async fn auth_refresh_token(provider: String, app: AppHandle) -> Result<AuthState, AppError> {
    auth::refresh_token(&provider, &app).await
}
