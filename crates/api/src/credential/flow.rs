//! OAuth HTTP flow helpers for the API layer.

use nebula_credential::credentials::oauth2::AuthStyle;
use serde::Deserialize;
use url::Url;

/// Request parameters for authorization URI construction.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizationUriRequest {
    /// OAuth authorization endpoint URL.
    pub auth_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// Redirect URI registered with provider.
    pub redirect_uri: String,
    /// Space-separated scopes.
    pub scopes: Option<String>,
}

/// Build OAuth2 Authorization Code URI with mandatory PKCE S256 parameters.
pub fn build_authorization_uri(
    req: &AuthorizationUriRequest,
    state: &str,
    code_challenge: &str,
) -> Result<Url, url::ParseError> {
    let mut url = Url::parse(&req.auth_url)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &req.client_id);
        q.append_pair("redirect_uri", &req.redirect_uri);
        q.append_pair("state", state);
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", "S256");
        if let Some(scopes) = req.scopes.as_deref()
            && !scopes.trim().is_empty()
        {
            q.append_pair("scope", scopes);
        }
    }
    Ok(url)
}

/// Token endpoint exchange request.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenExchangeRequest {
    /// OAuth token endpoint URL.
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client secret.
    pub client_secret: String,
    /// Authorization code received from callback.
    pub code: String,
    /// Redirect URI to echo in token exchange.
    pub redirect_uri: String,
    /// PKCE verifier paired with `code_challenge`.
    pub code_verifier: String,
    /// Client auth style for token endpoint.
    #[serde(default)]
    pub auth_style: AuthStyle,
}

/// Exchange authorization code for tokens.
pub async fn exchange_code(req: &TokenExchangeRequest) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("oauth client build failed: {e}"))?;

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", req.code.as_str()),
        ("redirect_uri", req.redirect_uri.as_str()),
        ("code_verifier", req.code_verifier.as_str()),
    ];

    let mut builder = client.post(&req.token_url);
    match req.auth_style {
        AuthStyle::Header => {
            builder = builder.basic_auth(&req.client_id, Some(&req.client_secret));
            builder = builder.form(&form);
        },
        AuthStyle::PostBody => {
            form.push(("client_id", req.client_id.as_str()));
            form.push(("client_secret", req.client_secret.as_str()));
            builder = builder.form(&form);
        },
    }

    let response = builder
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("token endpoint returned {status}"));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("token response parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_uri_contains_pkce_fields() {
        let req = AuthorizationUriRequest {
            auth_url: "https://provider.example.com/oauth/authorize".to_owned(),
            client_id: "client_123".to_owned(),
            redirect_uri: "https://app.example.com/callback".to_owned(),
            scopes: Some("read write".to_owned()),
        };

        let url = build_authorization_uri(&req, "signed_state", "code_challenge_123")
            .expect("auth url should build");
        let text = url.to_string();
        assert!(text.contains("code_challenge_method=S256"));
        assert!(text.contains("code_challenge=code_challenge_123"));
        assert!(text.contains("state=signed_state"));
    }
}
