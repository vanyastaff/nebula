pub mod keyring;
pub mod oauth;
pub mod pkce;
pub mod server;

use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::types::{AuthState, AuthStatus, AuthUser, UserProfile};

use self::oauth::OAuthProvider;

/// Global mutex to prevent concurrent auth flows.
static AUTH_LOCK: Mutex<()> = Mutex::const_new(());

/// Runs a full OAuth login flow:
/// 1. Generates PKCE pair
/// 2. Starts localhost callback server
/// 3. Builds authorization URL
/// 4. Opens browser
/// 5. Awaits callback with authorization code
/// 6. Exchanges code for tokens
/// 7. Stores tokens in OS keyring
/// 8. Fetches user profile
/// 9. Emits `auth_state_changed`
pub async fn login(provider: &str, app: &AppHandle) -> Result<AuthState, AppError> {
    let _guard = AUTH_LOCK.lock().await;

    let oauth_provider =
        OAuthProvider::from_str(provider).ok_or_else(|| AppError::Auth(format!("unknown provider: {provider}")))?;

    // Emit authorizing state
    let authorizing = AuthState {
        status: AuthStatus::Authorizing,
        provider: Some(provider.to_string()),
        access_token: String::new(),
        user: None,
        error: None,
    };
    emit_state(app, &authorizing);

    // Generate PKCE pair
    let verifier = pkce::generate_verifier();
    let challenge = pkce::generate_challenge(&verifier);

    // Start localhost callback server
    let (port, callback_handle) = server::start_callback_server()
        .await
        .map_err(|e| AppError::Auth(e))?;

    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state_token = pkce::generate_verifier(); // reuse as random state

    // Build auth URL and open browser
    let auth_url = oauth::build_auth_url(oauth_provider, &redirect_uri, &state_token, &challenge);

    app.opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| {
            let err = AppError::Auth(format!("failed to open browser: {e}"));
            emit_error(app, provider, &err);
            err
        })?;

    // Await the callback
    let callback_result = callback_handle
        .await
        .map_err(|e| AppError::Auth(format!("callback task panicked: {e}")))?
        .map_err(|e| {
            let err = AppError::Auth(e);
            emit_error(app, provider, &err);
            err
        })?;

    // Verify state matches
    if callback_result.state != state_token {
        let err = AppError::Auth("OAuth state mismatch — possible CSRF".to_string());
        emit_error(app, provider, &err);
        return Err(err);
    }

    // Exchange code for tokens (client-side PKCE — no client_secret needed)
    let tokens = oauth::exchange_code(
        oauth_provider,
        &callback_result.code,
        &verifier,
        &redirect_uri,
        "", // client_id managed server-side or empty for PKCE-only
        "", // no client_secret for public clients
    )
    .await
    .map_err(|e| {
        let err = AppError::Network(e);
        emit_error(app, provider, &err);
        err
    })?;

    // Store tokens in keyring
    keyring::store_tokens(&tokens.access_token, tokens.refresh_token.as_deref()).await?;

    // Fetch user profile
    let auth_user = oauth::fetch_user_profile(oauth_provider, &tokens.access_token)
        .await
        .map_err(|e| AppError::Network(e))?;

    let user_profile = auth_user_to_profile(&auth_user);

    let state = AuthState {
        status: AuthStatus::SignedIn,
        provider: Some(provider.to_string()),
        access_token: tokens.access_token,
        user: Some(user_profile),
        error: None,
    };
    emit_state(app, &state);

    Ok(state)
}

/// Signs out: clears keyring tokens and emits signed-out state.
pub async fn logout(app: &AppHandle) -> Result<(), AppError> {
    keyring::delete_tokens().await?;

    let state = AuthState {
        status: AuthStatus::SignedOut,
        provider: None,
        access_token: String::new(),
        user: None,
        error: None,
    };
    emit_state(app, &state);
    Ok(())
}

/// Returns the current user profile by reading the access token from keyring
/// and fetching the profile from the provider.
pub async fn get_user(provider: &str) -> Result<UserProfile, AppError> {
    let oauth_provider =
        OAuthProvider::from_str(provider).ok_or_else(|| AppError::Auth(format!("unknown provider: {provider}")))?;

    let access_token = keyring::get_access_token().await?;
    let auth_user = oauth::fetch_user_profile(oauth_provider, &access_token)
        .await
        .map_err(|e| AppError::Network(e))?;

    Ok(auth_user_to_profile(&auth_user))
}

/// Attempts to refresh the access token using the stored refresh token.
pub async fn refresh_token(provider: &str, app: &AppHandle) -> Result<AuthState, AppError> {
    let _guard = AUTH_LOCK.lock().await;

    let _oauth_provider =
        OAuthProvider::from_str(provider).ok_or_else(|| AppError::Auth(format!("unknown provider: {provider}")))?;

    let refresh = keyring::get_refresh_token()
        .await?
        .ok_or_else(|| AppError::Auth("no refresh token stored".to_string()))?;

    // For now, re-store the refresh token as-is and report that refresh
    // requires a backend endpoint (token refresh varies per provider and
    // typically needs client credentials on the server side).
    // This is a placeholder — a real implementation would POST to the
    // provider's token endpoint with grant_type=refresh_token.
    let access_token = keyring::get_access_token().await.unwrap_or_default();

    let state = AuthState {
        status: if access_token.is_empty() {
            AuthStatus::SignedOut
        } else {
            AuthStatus::SignedIn
        },
        provider: Some(provider.to_string()),
        access_token,
        user: None,
        error: Some("token refresh not yet implemented — please re-login".to_string()),
    };
    emit_state(app, &state);

    // Keep the refresh token for future use
    let _ = keyring::store_tokens(&state.access_token, Some(&refresh)).await;

    Ok(state)
}

/// Handle a deep-link OAuth callback as a fallback (when the localhost server
/// was not used or missed the callback).
pub async fn handle_deep_link_callback(
    code: String,
    provider: String,
    app: &AppHandle,
) -> Result<(), AppError> {
    let oauth_provider =
        OAuthProvider::from_str(&provider).ok_or_else(|| AppError::Auth(format!("unknown provider: {provider}")))?;

    // Deep-link fallback: exchange code via backend API
    let conn = crate::commands::connection::get_connection(app.clone()).await;
    let api_base_url = match conn.mode {
        crate::types::ConnectionMode::Local => conn.local_base_url,
        crate::types::ConnectionMode::Remote => conn.remote_base_url,
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{api_base_url}/auth/oauth/callback"))
        .json(&serde_json::json!({
            "provider": provider,
            "code": code,
            "redirectUri": "nebula://auth/callback"
        }))
        .send()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::json!({}));
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
        emit_error(app, &provider, &AppError::Auth(msg.clone()));
        return Err(AppError::Auth(msg));
    }

    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;

    let token = payload
        .get("accessToken")
        .or_else(|| payload.get("access_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let user: Option<UserProfile> = payload
        .get("user")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Store token in keyring
    if !token.is_empty() {
        let _ = keyring::store_tokens(&token, None).await;
    }

    // Fetch user profile if not provided
    let user = match user {
        Some(u) => Some(u),
        None if !token.is_empty() => {
            oauth::fetch_user_profile(oauth_provider, &token)
                .await
                .ok()
                .map(|u| auth_user_to_profile(&u))
        }
        None => None,
    };

    let status = if token.is_empty() {
        AuthStatus::SignedOut
    } else {
        AuthStatus::SignedIn
    };

    let state = AuthState {
        status,
        provider: Some(provider),
        access_token: token,
        user,
        error: None,
    };
    emit_state(app, &state);

    Ok(())
}

fn auth_user_to_profile(user: &AuthUser) -> UserProfile {
    UserProfile {
        id: user.id.clone(),
        login: user.email.clone(),
        name: if user.name.is_empty() { None } else { Some(user.name.clone()) },
        email: if user.email.is_empty() { None } else { Some(user.email.clone()) },
        avatar_url: user.avatar_url.clone(),
    }
}

fn emit_state(app: &AppHandle, state: &AuthState) {
    let _ = app.emit("auth_state_changed", state);
}

fn emit_error(app: &AppHandle, provider: &str, err: &AppError) {
    let state = AuthState {
        status: AuthStatus::SignedOut,
        provider: Some(provider.to_string()),
        access_token: String::new(),
        user: None,
        error: Some(err.to_string()),
    };
    emit_state(app, &state);
}
