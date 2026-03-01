use tauri::AppHandle;

use crate::{
    commands::{
        auth::{complete_sign_in, exchange_code, load, save_and_emit},
        connection::get_connection,
    },
    types::{AuthStatus, ConnectionMode, UserProfile},
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
        let user: Option<UserProfile> = None;
        let _ = complete_sign_in(token.clone(), provider, user, app);
        return;
    }

    // Flow 2: OAuth authorization code — exchange for token via backend
    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => return,
    };
    let provider = match params.get("provider") {
        Some(p) => p.clone(),
        None => {
            let mut state = load(app);
            state.status = AuthStatus::SignedOut;
            state.error = Some("OAuth callback missing provider parameter.".to_string());
            let _ = save_and_emit(app, &state);
            return;
        }
    };

    // Resolve active API base URL from connection store
    let conn = get_connection(app.clone()).await;
    let api_base_url = match conn.mode {
        ConnectionMode::Local => conn.local_base_url,
        ConnectionMode::Remote => conn.remote_base_url,
    };

    let _ = exchange_code(code, provider, api_base_url, app.clone()).await;
}
