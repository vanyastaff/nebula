//! Single Authentication Service Example
//!
//! This example demonstrates the simpler pattern for services that support
//! ONLY ONE authentication method (e.g., only API Key, or only OAuth2).
//!
//! Pattern:
//! 1. Define single credential struct (not enum!)
//! 2. Implement Credential trait
//! 3. Simple authenticator - no pattern matching needed
//! 4. Config contains only credential_id

use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use nebula_credential::{
    authenticator::{AuthenticateWithState, StatefulAuthenticator},
    core::{
        AccessToken, CredentialContext, CredentialError, CredentialId, CredentialMetadata,
        CredentialState, SecureString,
    },
    testing::{MockLock, MockStateStore},
    traits::Credential,
    CredentialManager,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ════════════════════════════════════════════════════════════════════════════
// Example 1: API Key Only Service
// ════════════════════════════════════════════════════════════════════════════

/// Input for API Key credential - simple struct, not enum!
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub key: String,
    #[serde(default = "default_header_name")]
    pub header_name: String,
}

fn default_header_name() -> String {
    "X-API-Key".to_string()
}

/// State for API Key credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub key: SecureString,
    pub header_name: String,
}

impl CredentialState for ApiKeyState {
    const KIND: &'static str = "api_key";
    const VERSION: u16 = 1;
}

/// API Key credential implementation
pub struct ApiKeyCredential;

#[async_trait]
impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type State = ApiKeyState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "api_key",
            name: "API Key",
            description: "Simple API Key authentication",
            supports_refresh: false,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        if input.key.is_empty() {
            return Err(CredentialError::Internal(
                "API key cannot be empty".to_string(),
            ));
        }

        let state = ApiKeyState {
            key: SecureString::new(input.key.clone()),
            header_name: input.header_name.clone(),
        };

        let token = AccessToken::bearer(input.key.clone())
            .with_expiration(SystemTime::now() + Duration::from_secs(86400 * 365)); // 1 year

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken, CredentialError> {
        Err(CredentialError::Internal(
            "API Keys do not support refresh".to_string(),
        ))
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        Ok(!state.key.with_exposed(|s| s.is_empty()))
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Example 2: OAuth2 Only Service
// ════════════════════════════════════════════════════════════════════════════

/// Input for OAuth2 credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Input {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// State for OAuth2 credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub client_id: String,
    pub client_secret: SecureString,
    pub access_token: SecureString,
    pub refresh_token: Option<SecureString>,
    pub scopes: Vec<String>,
}

impl CredentialState for OAuth2State {
    const KIND: &'static str = "oauth2";
    const VERSION: u16 = 1;
}

/// OAuth2 credential implementation
pub struct OAuth2Credential;

#[async_trait]
impl Credential for OAuth2Credential {
    type Input = OAuth2Input;
    type State = OAuth2State;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "oauth2",
            name: "OAuth2",
            description: "OAuth2 authentication with automatic refresh",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        let state = OAuth2State {
            client_id: input.client_id.clone(),
            client_secret: SecureString::new(input.client_secret.clone()),
            access_token: SecureString::new(input.access_token.clone()),
            refresh_token: input.refresh_token.as_ref().map(|t| SecureString::new(t.clone())),
            scopes: input.scopes.clone(),
        };

        let token = AccessToken::bearer(input.access_token.clone())
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken, CredentialError> {
        if let Some(_refresh_token) = &state.refresh_token {
            // Simulate OAuth2 token refresh
            let new_token = format!("refreshed_{}", uuid::Uuid::new_v4());
            state.access_token = SecureString::new(new_token.clone());

            let token = AccessToken::bearer(new_token)
                .with_expiration(SystemTime::now() + Duration::from_secs(3600));

            Ok(token)
        } else {
            Err(CredentialError::Internal(
                "No refresh token available".to_string(),
            ))
        }
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        Ok(!state.access_token.with_exposed(|s| s.is_empty()))
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Mock HTTP Client
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub base_url: String,
}

#[derive(Debug)]
pub struct HttpClient {
    base_url: String,
    headers: HashMap<String, String>,
}

impl HttpClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            headers: HashMap::new(),
        }
    }

    pub fn with_header(mut self, key: String, value: String) -> Self {
        self.headers.insert(key, value);
        self
    }

    pub async fn get(&self, path: &str) -> Result<HttpResponse, CredentialError> {
        println!("🌐 GET {}{}", self.base_url, path);
        for (key, value) in &self.headers {
            println!("   Header: {}: {}", key, value);
        }

        Ok(HttpResponse {
            status: 200,
            body: "Success".to_string(),
        })
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

// ════════════════════════════════════════════════════════════════════════════
// Simple Authenticators - No Pattern Matching!
// ════════════════════════════════════════════════════════════════════════════

/// Authenticator for API Key - simple, no enum matching!
pub struct ApiKeyAuthenticator;

#[async_trait]
impl StatefulAuthenticator<ApiKeyCredential> for ApiKeyAuthenticator {
    type Target = HttpClientConfig;
    type Output = HttpClient;

    async fn authenticate(
        &self,
        config: Self::Target,
        state: &ApiKeyState,
    ) -> Result<Self::Output, CredentialError> {
        println!("🔐 Configuring API Key authentication");

        let client = HttpClient::new(config.base_url);

        // Simple - just add the header with the key
        let key_value = state.key.with_exposed(ToString::to_string);
        let client = client.with_header(state.header_name.clone(), key_value);

        Ok(client)
    }
}

/// Authenticator for OAuth2 - simple, no enum matching!
pub struct OAuth2Authenticator;

#[async_trait]
impl StatefulAuthenticator<OAuth2Credential> for OAuth2Authenticator {
    type Target = HttpClientConfig;
    type Output = HttpClient;

    async fn authenticate(
        &self,
        config: Self::Target,
        state: &OAuth2State,
    ) -> Result<Self::Output, CredentialError> {
        println!("🔐 Configuring OAuth2 authentication");
        println!("   Client ID: {}", state.client_id);
        println!("   Scopes: {:?}", state.scopes);

        let client = HttpClient::new(config.base_url);

        // Simple - just add Authorization header
        let token_value = state.access_token.with_exposed(ToString::to_string);
        let auth_header = format!("Bearer {}", token_value);
        let client = client.with_header("Authorization".to_string(), auth_header);

        Ok(client)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Service Configuration - NO AuthMethod Field!
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub endpoint: String,
    pub credential_id: String,
}

// ════════════════════════════════════════════════════════════════════════════
// Main Example
// ════════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║        Single Authentication Service Examples                       ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    // Setup credential manager
    let manager = CredentialManager::builder()
        .with_store(Arc::new(MockStateStore::new()))
        .with_lock(MockLock::new())
        .build()
        .map_err(|e| format!("Failed to build manager: {}", e))?;

    let registry = manager.registry();

    // Register both credential types
    registry.register_credential(ApiKeyCredential);
    registry.register_credential(OAuth2Credential);

    println!("✅ Credential manager initialized\n");

    // ════════════════════════════════════════════════════════════════════════
    // Example 1: API Key Service (e.g., Stripe, SendGrid)
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("Example 1: API Key Only Service (e.g., Stripe)");
    println!("{}", "═".repeat(70));

    let api_key_input = ApiKeyInput {
        key: "sk_live_abc123xyz789".to_string(),
        header_name: "X-API-Key".to_string(),
    };

    let api_key_id = manager
        .create_credential("api_key", serde_json::to_value(&api_key_input)?)
        .await?;

    println!("✅ API Key credential created: {}", api_key_id);

    // Config - ONLY has credential_id!
    let config = ServiceConfig {
        endpoint: "https://api.stripe.com".to_string(),
        credential_id: api_key_id.to_string(),
    };

    println!("📋 Config: endpoint={}, credential_id={}", config.endpoint, config.credential_id);

    // Authenticate
    let client_config = HttpClientConfig {
        base_url: config.endpoint.clone(),
    };

    // Manually create state for demo
    let api_key_state = ApiKeyState {
        key: SecureString::new("sk_live_abc123xyz789".to_string()),
        header_name: "X-API-Key".to_string(),
    };

    let authenticator = ApiKeyAuthenticator;
    let client = client_config
        .authenticate_with_state(&authenticator, &api_key_state)
        .await?;

    let _response = client.get("/v1/customers").await?;
    println!("✅ Request successful\n");

    // ════════════════════════════════════════════════════════════════════════
    // Example 2: OAuth2 Service (e.g., Google APIs)
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("Example 2: OAuth2 Only Service (e.g., Google APIs)");
    println!("{}", "═".repeat(70));

    let oauth2_input = OAuth2Input {
        client_id: "my_client_id.apps.googleusercontent.com".to_string(),
        client_secret: "GOCSPX-xxxxxxxxxxxxx".to_string(),
        access_token: "ya29.a0AfH6SMC...".to_string(),
        refresh_token: Some("1//0gB8...".to_string()),
        scopes: vec!["https://www.googleapis.com/auth/drive".to_string()],
    };

    let oauth2_id = manager
        .create_credential("oauth2", serde_json::to_value(&oauth2_input)?)
        .await?;

    println!("✅ OAuth2 credential created: {}", oauth2_id);

    let config = ServiceConfig {
        endpoint: "https://www.googleapis.com".to_string(),
        credential_id: oauth2_id.to_string(),
    };

    let client_config = HttpClientConfig {
        base_url: config.endpoint.clone(),
    };

    // Manually create state for demo
    let oauth2_state = OAuth2State {
        client_id: "my_client_id.apps.googleusercontent.com".to_string(),
        client_secret: SecureString::new("GOCSPX-xxxxxxxxxxxxx".to_string()),
        access_token: SecureString::new("ya29.a0AfH6SMC...".to_string()),
        refresh_token: Some(SecureString::new("1//0gB8...".to_string())),
        scopes: vec!["https://www.googleapis.com/auth/drive".to_string()],
    };

    let oauth2_authenticator = OAuth2Authenticator;
    let client = client_config
        .authenticate_with_state(&oauth2_authenticator, &oauth2_state)
        .await?;

    let _response = client.get("/drive/v3/files").await?;
    println!("✅ Request successful\n");

    // ════════════════════════════════════════════════════════════════════════
    // Key Takeaways
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("🎯 Key Takeaways for Single-Auth Services");
    println!("{}", "═".repeat(70));
    println!("✅ Use simple structs (not enums) for Input and State");
    println!("✅ Config contains ONLY credential_id (same as multi-auth!)");
    println!("✅ Authenticator is simpler - no pattern matching needed");
    println!("✅ Type-safe access to all credential fields");
    println!("✅ Same overall pattern as multi-auth (consistent!)");
    println!("✅ Can easily upgrade to multi-auth later if needed\n");

    println!("{}", "═".repeat(70));
    println!("📊 Comparison: Single-Auth vs Multi-Auth");
    println!("{}", "═".repeat(70));
    println!("┌─────────────────┬─────────────────────┬─────────────────────┐");
    println!("│ Aspect          │ Single-Auth         │ Multi-Auth          │");
    println!("├─────────────────┼─────────────────────┼─────────────────────┤");
    println!("│ Input/State     │ Struct              │ Enum                │");
    println!("│ Authenticator   │ No pattern matching │ Pattern matching    │");
    println!("│ Config          │ Only credential_id  │ Only credential_id  │");
    println!("│ Complexity      │ Simpler             │ More flexible       │");
    println!("│ Upgrade path    │ Easy → Multi-Auth   │ Already multi       │");
    println!("└─────────────────┴─────────────────────┴─────────────────────┘\n");

    Ok(())
}
