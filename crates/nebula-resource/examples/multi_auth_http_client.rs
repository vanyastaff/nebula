//! Multi-Authentication HTTP Client Example
//!
//! This example demonstrates how to build an HTTP client resource that supports
//! multiple authentication methods using nebula-credential and nebula-resource.
//!
//! Pattern:
//! 1. Define credential enum with all auth variants (API Key, OAuth2, Basic Auth)
//! 2. Implement StatefulAuthenticator for HTTP client
//! 3. Resource configuration contains only credential_id (NO AuthMethod!)
//! 4. Authenticator pattern matches on credential state

#[cfg(feature = "credentials")]
use async_trait::async_trait;

#[cfg(feature = "credentials")]
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

#[cfg(feature = "credentials")]
use nebula_resource::{
    credentials::{stateful::StatefulCredentialProvider, ResourceCredentialProvider},
    core::error::ResourceResult,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ════════════════════════════════════════════════════════════════════════════
// HTTP Service Credentials - All Authentication Variants
// ════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HttpServiceCredentialsInput {
    /// API Key authentication
    ApiKey {
        key: String,
        #[serde(default)]
        header_name: String, // Default: "X-API-Key"
    },
    /// OAuth2 authentication
    OAuth2 {
        client_id: String,
        client_secret: String,
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
    },
    /// Basic HTTP authentication
    BasicAuth { username: String, password: String },
    /// Bearer token authentication
    BearerToken { token: String },
}

#[cfg(feature = "credentials")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HttpServiceCredentialsState {
    /// API Key authentication
    ApiKey {
        key: SecureString,
        header_name: String,
    },
    /// OAuth2 authentication
    OAuth2 {
        client_id: String,
        client_secret: SecureString,
        access_token: SecureString,
        refresh_token: Option<SecureString>,
    },
    /// Basic HTTP authentication
    BasicAuth {
        username: String,
        password: SecureString,
    },
    /// Bearer token authentication
    BearerToken { token: SecureString },
}

#[cfg(feature = "credentials")]
impl CredentialState for HttpServiceCredentialsState {
    const KIND: &'static str = "http_service";
    const VERSION: u16 = 1;
}

#[cfg(feature = "credentials")]
pub struct HttpServiceCredential;

#[cfg(feature = "credentials")]
#[async_trait]
impl Credential for HttpServiceCredential {
    type Input = HttpServiceCredentialsInput;
    type State = HttpServiceCredentialsState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "http_service",
            name: "HTTP Service Credentials",
            description: "Multi-method HTTP authentication (API Key, OAuth2, Basic Auth, Bearer)",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        let (state, token_value) = match input {
            HttpServiceCredentialsInput::ApiKey { key, header_name } => {
                let header = if header_name.is_empty() {
                    "X-API-Key".to_string()
                } else {
                    header_name.clone()
                };
                let state = HttpServiceCredentialsState::ApiKey {
                    key: SecureString::new(key.clone()),
                    header_name: header,
                };
                (state, key.clone())
            }
            HttpServiceCredentialsInput::OAuth2 {
                client_id,
                client_secret,
                access_token,
                refresh_token,
            } => {
                let state = HttpServiceCredentialsState::OAuth2 {
                    client_id: client_id.clone(),
                    client_secret: SecureString::new(client_secret.clone()),
                    access_token: SecureString::new(access_token.clone()),
                    refresh_token: refresh_token.as_ref().map(|t| SecureString::new(t.clone())),
                };
                (state, access_token.clone())
            }
            HttpServiceCredentialsInput::BasicAuth { username, password } => {
                let state = HttpServiceCredentialsState::BasicAuth {
                    username: username.clone(),
                    password: SecureString::new(password.clone()),
                };
                (state, password.clone())
            }
            HttpServiceCredentialsInput::BearerToken { token } => {
                let state = HttpServiceCredentialsState::BearerToken {
                    token: SecureString::new(token.clone()),
                };
                (state, token.clone())
            }
        };

        let token = AccessToken::bearer(token_value)
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken, CredentialError> {
        match state {
            HttpServiceCredentialsState::OAuth2 {
                access_token,
                refresh_token,
                ..
            } => {
                if refresh_token.is_some() {
                    // Simulate OAuth2 refresh
                    let new_token = format!("refreshed_{}", uuid::Uuid::new_v4());
                    *access_token = SecureString::new(new_token.clone());

                    let token = AccessToken::bearer(new_token)
                        .with_expiration(SystemTime::now() + Duration::from_secs(3600));
                    Ok(token)
                } else {
                    Err(CredentialError::Internal(
                        "No refresh token available".to_string(),
                    ))
                }
            }
            _ => Err(CredentialError::Internal(
                "Only OAuth2 credentials support refresh".to_string(),
            )),
        }
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        match state {
            HttpServiceCredentialsState::ApiKey { key, .. } => {
                Ok(!key.with_exposed(|s| s.is_empty()))
            }
            HttpServiceCredentialsState::OAuth2 { access_token, .. } => {
                Ok(!access_token.with_exposed(|s| s.is_empty()))
            }
            HttpServiceCredentialsState::BasicAuth { password, .. } => {
                Ok(!password.with_exposed(|s| s.is_empty()))
            }
            HttpServiceCredentialsState::BearerToken { token } => {
                Ok(!token.with_exposed(|s| s.is_empty()))
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Mock HTTP Client
// ════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub base_url: String,
    pub timeout_seconds: u64,
}

#[cfg(feature = "credentials")]
#[derive(Debug)]
pub struct HttpClient {
    base_url: String,
    headers: HashMap<String, String>,
}

#[cfg(feature = "credentials")]
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

    pub async fn get(&self, path: &str) -> ResourceResult<HttpResponse> {
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

#[cfg(feature = "credentials")]
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

// ════════════════════════════════════════════════════════════════════════════
// HTTP Client Authenticator - Pattern Matches on Credential State
// ════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
pub struct HttpClientAuthenticator;

#[cfg(feature = "credentials")]
#[async_trait]
impl StatefulAuthenticator<HttpServiceCredential> for HttpClientAuthenticator {
    type Target = HttpClientConfig;
    type Output = HttpClient;

    async fn authenticate(
        &self,
        config: Self::Target,
        state: &HttpServiceCredentialsState,
    ) -> Result<Self::Output, CredentialError> {
        let mut client = HttpClient::new(config.base_url);

        // Pattern match on credential state to apply appropriate authentication
        match state {
            HttpServiceCredentialsState::ApiKey { key, header_name } => {
                println!("🔐 Configuring API Key authentication");
                let key_value = key.with_exposed(ToString::to_string);
                client = client.with_header(header_name.clone(), key_value);
            }

            HttpServiceCredentialsState::OAuth2 { access_token, .. } => {
                println!("🔐 Configuring OAuth2 authentication");
                let token_value = access_token.with_exposed(ToString::to_string);
                let auth_header = format!("Bearer {}", token_value);
                client = client.with_header("Authorization".to_string(), auth_header);
            }

            HttpServiceCredentialsState::BasicAuth { username, password } => {
                println!("🔐 Configuring Basic Auth");
                let credentials = password.with_exposed(|pwd| format!("{}:{}", username, pwd));
                let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
                let auth_header = format!("Basic {}", encoded);
                client = client.with_header("Authorization".to_string(), auth_header);
            }

            HttpServiceCredentialsState::BearerToken { token } => {
                println!("🔐 Configuring Bearer Token authentication");
                let token_value = token.with_exposed(ToString::to_string);
                let auth_header = format!("Bearer {}", token_value);
                client = client.with_header("Authorization".to_string(), auth_header);
            }
        }

        Ok(client)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// HTTP Resource Configuration - NO AuthMethod Field!
// ════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResourceConfig {
    /// Service endpoint
    pub endpoint: String,
    /// Credential ID to use (authentication method determined by credential!)
    pub credential_id: String,
    /// Request timeout
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

#[cfg(feature = "credentials")]
fn default_timeout() -> u64 {
    30
}

// ════════════════════════════════════════════════════════════════════════════
// Main Example
// ════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║  Multi-Authentication HTTP Client (nebula-resource + credential)    ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");

    // Setup credential manager
    let manager = CredentialManager::builder()
        .with_store(Arc::new(MockStateStore::new()))
        .with_lock(MockLock::new())
        .build()
        .map_err(|e| format!("Failed to build manager: {}", e))?;

    let registry = manager.registry();
    registry.register_credential(HttpServiceCredential);

    println!("✅ Credential manager initialized\n");

    // ════════════════════════════════════════════════════════════════════════
    // Scenario 1: API Key Authentication
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("Scenario 1: API Key Authentication");
    println!("{}", "═".repeat(70));

    let api_key_input = HttpServiceCredentialsInput::ApiKey {
        key: "sk_live_abc123xyz789".to_string(),
        header_name: "X-API-Key".to_string(),
    };

    let api_key_id = manager
        .create_credential("http_service", serde_json::to_value(&api_key_input)?)
        .await?;

    println!("✅ API Key credential created: {}", api_key_id);

    // Resource config - ONLY has credential_id!
    let config = HttpResourceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: api_key_id.to_string(),
        timeout_seconds: 30,
    };

    println!("📋 Config: endpoint={}, credential_id={}", config.endpoint, config.credential_id);

    // Create client config and manually create state for demo
    let client_config = HttpClientConfig {
        base_url: config.endpoint.clone(),
        timeout_seconds: config.timeout_seconds,
    };

    let api_key_state = HttpServiceCredentialsState::ApiKey {
        key: SecureString::new("sk_live_abc123xyz789".to_string()),
        header_name: "X-API-Key".to_string(),
    };

    let authenticator = HttpClientAuthenticator;
    let client = client_config
        .authenticate_with_state(&authenticator, &api_key_state)
        .await?;

    let _response = client.get("/users").await?;
    println!("✅ Request successful\n");

    // ════════════════════════════════════════════════════════════════════════
    // Scenario 2: OAuth2 Authentication
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("Scenario 2: OAuth2 Authentication");
    println!("{}", "═".repeat(70));

    let oauth2_input = HttpServiceCredentialsInput::OAuth2 {
        client_id: "my_client_id".to_string(),
        client_secret: "my_client_secret".to_string(),
        access_token: "ya29.a0AfH6SMC...".to_string(),
        refresh_token: Some("1//0gB8...".to_string()),
    };

    let oauth2_id = manager
        .create_credential("http_service", serde_json::to_value(&oauth2_input)?)
        .await?;

    println!("✅ OAuth2 credential created: {}", oauth2_id);

    let config = HttpResourceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: oauth2_id.to_string(),
        timeout_seconds: 30,
    };

    let client_config = HttpClientConfig {
        base_url: config.endpoint.clone(),
        timeout_seconds: config.timeout_seconds,
    };

    let oauth2_state = HttpServiceCredentialsState::OAuth2 {
        client_id: "my_client_id".to_string(),
        client_secret: SecureString::new("my_client_secret".to_string()),
        access_token: SecureString::new("ya29.a0AfH6SMC...".to_string()),
        refresh_token: Some(SecureString::new("1//0gB8...".to_string())),
    };

    let client = client_config
        .authenticate_with_state(&authenticator, &oauth2_state)
        .await?;

    let _response = client.get("/users").await?;
    println!("✅ Request successful\n");

    // ════════════════════════════════════════════════════════════════════════
    // Scenario 3: Basic Auth
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("Scenario 3: Basic HTTP Authentication");
    println!("{}", "═".repeat(70));

    let basic_input = HttpServiceCredentialsInput::BasicAuth {
        username: "admin".to_string(),
        password: "super_secret".to_string(),
    };

    let basic_id = manager
        .create_credential("http_service", serde_json::to_value(&basic_input)?)
        .await?;

    println!("✅ Basic Auth credential created: {}", basic_id);

    let config = HttpResourceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: basic_id.to_string(),
        timeout_seconds: 30,
    };

    let client_config = HttpClientConfig {
        base_url: config.endpoint.clone(),
        timeout_seconds: config.timeout_seconds,
    };

    let basic_state = HttpServiceCredentialsState::BasicAuth {
        username: "admin".to_string(),
        password: SecureString::new("super_secret".to_string()),
    };

    let client = client_config
        .authenticate_with_state(&authenticator, &basic_state)
        .await?;

    let _response = client.get("/users").await?;
    println!("✅ Request successful\n");

    // ════════════════════════════════════════════════════════════════════════
    // Key Takeaways
    // ════════════════════════════════════════════════════════════════════════
    println!("{}", "═".repeat(70));
    println!("🎯 Key Takeaways");
    println!("{}", "═".repeat(70));
    println!("✅ Config contains ONLY credential_id (NO AuthMethod field)");
    println!("✅ Credential enum determines authentication method");
    println!("✅ Single authenticator handles all methods via pattern matching");
    println!("✅ Type-safe access to all credential fields");
    println!("✅ Same code path for all authentication types");
    println!("✅ Easy to add new auth methods (just extend enum)");
    println!("✅ nebula-resource seamlessly integrates with nebula-credential\n");

    Ok(())
}

#[cfg(not(feature = "credentials"))]
fn main() {
    println!("This example requires the 'credentials' feature");
    println!("Run with: cargo run --example multi_auth_http_client --features credentials");
}
