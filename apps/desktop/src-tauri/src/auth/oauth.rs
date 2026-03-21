use serde::Deserialize;

use crate::types::AuthUser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    Google,
    GitHub,
}

impl OAuthProvider {
    pub fn auth_url(&self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::GitHub => "https://github.com/login/oauth/authorize",
        }
    }

    pub fn token_url(&self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::GitHub => "https://github.com/login/oauth/access_token",
        }
    }

    pub fn userinfo_url(&self) -> &'static str {
        match self {
            Self::Google => "https://www.googleapis.com/oauth2/v2/userinfo",
            Self::GitHub => "https://api.github.com/user",
        }
    }

    pub fn scopes(&self) -> &'static str {
        match self {
            Self::Google => "openid email profile",
            Self::GitHub => "read:user user:email",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "google" => Some(Self::Google),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }
}

/// Build the full authorization URL for the given provider.
pub fn build_auth_url(
    provider: OAuthProvider,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let base = provider.auth_url();
    let scopes = provider.scopes();

    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("state", state),
        ("scope", scopes),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
    ];

    match provider {
        OAuthProvider::Google => {
            params.push(("access_type", "offline"));
            params.push(("prompt", "consent"));
        }
        OAuthProvider::GitHub => {}
    }

    let query = params
        .into_iter()
        .map(|(k, v)| format!("{k}={}", urlencoding(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{base}?{query}")
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[allow(dead_code)]
    pub token_type: Option<String>,
    pub refresh_token: Option<String>,
    #[allow(dead_code)]
    pub expires_in: Option<u64>,
    #[allow(dead_code)]
    pub scope: Option<String>,
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    provider: OAuthProvider,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();

    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    // Only include client_secret for confidential clients; PKCE flows omit it.
    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret));
    }

    // GitHub requires Accept: application/json
    let mut req = client.post(provider.token_url()).form(&form);
    if provider == OAuthProvider::GitHub {
        req = req.header("Accept", "application/json");
    }

    let response = req.send().await.map_err(|e| e.to_string())?;

    // GitHub returns errors as HTTP 200 with {"error":"...","error_description":"..."}
    let body = response.text().await.map_err(|e| e.to_string())?;

    // Check for OAuth error response before attempting to parse tokens
    if let Ok(err_val) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(err_code) = err_val.get("error").and_then(|v| v.as_str()) {
            let description = err_val
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or(err_code);
            return Err(format!("token exchange failed: {description}"));
        }
    }

    serde_json::from_str::<TokenResponse>(&body).map_err(|e| format!("failed to parse token response: {e}\nbody: {body}"))
}

/// Fetch the user profile from the provider's userinfo endpoint.
pub async fn fetch_user_profile(
    provider: OAuthProvider,
    access_token: &str,
) -> Result<AuthUser, String> {
    let client = reqwest::Client::new();

    let mut req = client
        .get(provider.userinfo_url())
        .bearer_auth(access_token);

    if provider == OAuthProvider::GitHub {
        req = req.header("User-Agent", "nebula-desktop");
    }

    let response = req.send().await.map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("userinfo request failed: {status} {body}"));
    }

    let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;

    let provider_name = match provider {
        OAuthProvider::Google => "google",
        OAuthProvider::GitHub => "github",
    };

    let id = value
        .get("id")
        .or_else(|| value.get("sub"))
        .map(|v| match v {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();

    let email = value
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let avatar_url = value
        .get("picture")
        .or_else(|| value.get("avatar_url"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(AuthUser {
        id,
        email,
        name,
        avatar_url,
        provider: provider_name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_auth_url_contains_required_params() {
        let url = build_auth_url(
            OAuthProvider::Google,
            "test-client-id",
            "http://127.0.0.1:8080/callback",
            "random-state",
            "challenge123",
        );
        assert!(url.starts_with("https://accounts.google.com"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=random-state"));
        assert!(url.contains("code_challenge=challenge123"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn provider_from_str_works() {
        assert_eq!(OAuthProvider::from_str("google"), Some(OAuthProvider::Google));
        assert_eq!(OAuthProvider::from_str("GitHub"), Some(OAuthProvider::GitHub));
        assert_eq!(OAuthProvider::from_str("unknown"), None);
    }
}
