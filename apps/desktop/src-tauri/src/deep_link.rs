use tauri::{AppHandle, Emitter};

use crate::{
    auth,
    types::{AuthState, AuthStatus},
};

pub async fn handle(raw_url: &str, app: &AppHandle) {
    let Ok(parsed) = raw_url.parse::<url::Url>() else {
        return;
    };

    if parsed.scheme() != "nebula" || parsed.host_str() != Some("auth") {
        return;
    }

    if parsed.path().trim_end_matches('/') != "/callback" {
        return;
    }

    let params: std::collections::HashMap<String, String> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Flow 1: backend delivered token directly (legacy / test mock)
    if let Some(token) = params
        .get("access_token")
        .or_else(|| params.get("token"))
        .filter(|t| !t.is_empty())
    {
        let provider = params.get("provider").cloned();

        // Store in keyring
        let _ = auth::keyring::store_tokens(token, None).await;

        let state = AuthState {
            status: AuthStatus::SignedIn,
            provider,
            access_token: token.clone(),
            user: None,
            error: None,
        };
        let _ = app.emit("auth_state_changed", &state);
        return;
    }

    // Flow 2: OAuth authorization code — exchange for token via auth module
    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => return,
    };
    let provider = match params.get("provider") {
        Some(p) => p.clone(),
        None => {
            let state = AuthState {
                status: AuthStatus::SignedOut,
                provider: None,
                access_token: String::new(),
                user: None,
                error: Some("OAuth callback missing provider parameter.".to_string()),
            };
            let _ = app.emit("auth_state_changed", &state);
            return;
        }
    };

    let _ = auth::handle_deep_link_callback(code, provider, app).await;
}
