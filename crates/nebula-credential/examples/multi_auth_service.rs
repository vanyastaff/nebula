//! Multi-Authentication Service Example
//!
//! This example demonstrates the recommended pattern for services that support
//! multiple authentication methods (API Key, OAuth2, Basic Auth, etc.).
//!
//! Key Design Principles:
//! 1. Use enum to represent all authentication variants
//! 2. Config contains only credential_id, NO AuthMethod field
//! 3. Single authenticator uses pattern matching on credential enum
//! 4. Type-safe access to credential fields through State

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use nebula_credential::authenticator::{AuthenticateWithState, StatefulAuthenticator};
use nebula_credential::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
    SecureString,
};
use nebula_credential::testing::{MockLock, MockStateStore};
use nebula_credential::traits::Credential;
use nebula_credential::CredentialManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// Use our own Result type to avoid conflict
type Result<T> = std::result::Result<T, CredentialError>;

// ============================================================================
// Credential Enum - All Authentication Variants
// ============================================================================

/// Input for credential initialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsInput {
    /// API Key authentication
    ApiKey {
        key: String,
        #[serde(default)]
        key_prefix: Option<String>,
    },
    /// OAuth2 authentication
    OAuth2 {
        client_id: String,
        client_secret: String,
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Basic authentication (username + password)
    BasicAuth { username: String, password: String },
    /// Bearer token authentication
    BearerToken { token: String },
}

/// State representation - stored securely
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsState {
    /// API Key authentication
    ApiKey {
        key: SecureString,
        #[serde(default)]
        key_prefix: Option<String>,
    },
    /// OAuth2 authentication
    OAuth2 {
        client_id: String,
        client_secret: SecureString,
        access_token: SecureString,
        #[serde(default)]
        refresh_token: Option<SecureString>,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Basic authentication (username + password)
    BasicAuth {
        username: String,
        password: SecureString,
    },
    /// Bearer token authentication
    BearerToken { token: SecureString },
}

impl CredentialState for ServiceCredentialsState {
    const KIND: &'static str = "service";
    const VERSION: u16 = 1;
}

// ============================================================================
// Credential Implementation
// ============================================================================

#[derive(Debug, Clone)]
pub struct ServiceCredential;

impl ServiceCredential {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Credential for ServiceCredential {
    type Input = ServiceCredentialsInput;
    type State = ServiceCredentialsState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "service",
            name: "Service Credentials",
            description: "Multi-method service authentication (API Key, OAuth2, Basic Auth, Bearer Token)",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        // Convert input to state and create token
        let (state, token_value) = match input {
            ServiceCredentialsInput::ApiKey { key, key_prefix } => {
                if key.is_empty() {
                    return Err(CredentialError::Internal(
                        "API key cannot be empty".to_string(),
                    ));
                }
                let state = ServiceCredentialsState::ApiKey {
                    key: SecureString::new(key.clone()),
                    key_prefix: key_prefix.clone(),
                };
                (state, key.clone())
            }
            ServiceCredentialsInput::OAuth2 {
                client_id,
                client_secret,
                access_token,
                refresh_token,
                scopes,
            } => {
                if client_id.is_empty()
                    || client_secret.is_empty()
                    || access_token.is_empty()
                {
                    return Err(CredentialError::Internal(
                        "OAuth2 fields cannot be empty".to_string(),
                    ));
                }
                let state = ServiceCredentialsState::OAuth2 {
                    client_id: client_id.clone(),
                    client_secret: SecureString::new(client_secret.clone()),
                    access_token: SecureString::new(access_token.clone()),
                    refresh_token: refresh_token.as_ref().map(|t| SecureString::new(t.clone())),
                    scopes: scopes.clone(),
                };
                (state, access_token.clone())
            }
            ServiceCredentialsInput::BasicAuth { username, password } => {
                if username.is_empty() || password.is_empty() {
                    return Err(CredentialError::Internal(
                        "Username and password cannot be empty".to_string(),
                    ));
                }
                let state = ServiceCredentialsState::BasicAuth {
                    username: username.clone(),
                    password: SecureString::new(password.clone()),
                };
                (state, password.clone())
            }
            ServiceCredentialsInput::BearerToken { token } => {
                if token.is_empty() {
                    return Err(CredentialError::Internal(
                        "Bearer token cannot be empty".to_string(),
                    ));
                }
                let state = ServiceCredentialsState::BearerToken {
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
    ) -> Result<AccessToken> {
        // Only OAuth2 can be refreshed
        match state {
            ServiceCredentialsState::OAuth2 {
                refresh_token,
                access_token,
                ..
            } => {
                if refresh_token.is_some() {
                    // Simulate OAuth2 token refresh
                    let new_token_value = format!("refreshed_token_{}", uuid::Uuid::new_v4());
                    *access_token = SecureString::new(new_token_value.clone());

                    let token = AccessToken::bearer(new_token_value)
                        .with_expiration(SystemTime::now() + Duration::from_secs(3600));

                    Ok(token)
                } else {
                    Err(CredentialError::Internal(
                        "No refresh token available".to_string(),
                    ))
                }
            }
            _ => Err(CredentialError::Internal(
                "Only OAuth2 credentials can be refreshed".to_string(),
            )),
        }
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool> {
        match state {
            ServiceCredentialsState::ApiKey { key, .. } => {
                if key.expose().is_empty() {
                    return Err(CredentialError::Internal("API key is empty".to_string()));
                }
            }
            ServiceCredentialsState::OAuth2 { access_token, .. } => {
                if access_token.expose().is_empty() {
                    return Err(CredentialError::Internal(
                        "Access token is empty".to_string(),
                    ));
                }
            }
            ServiceCredentialsState::BasicAuth { username, password } => {
                if username.is_empty() || password.expose().is_empty() {
                    return Err(CredentialError::Internal(
                        "Username or password is empty".to_string(),
                    ));
                }
            }
            ServiceCredentialsState::BearerToken { token } => {
                if token.expose().is_empty() {
                    return Err(CredentialError::Internal(
                        "Bearer token is empty".to_string(),
                    ));
                }
            }
        }
        Ok(true)
    }
}

// ============================================================================
// Mock HTTP Request Builder
// ============================================================================

#[derive(Debug, Clone)]
pub struct HttpRequestBuilder {
    pub url: String,
    pub headers: HashMap<String, String>,
}

impl HttpRequestBuilder {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HashMap::new(),
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub async fn send(self) -> Result<HttpResponse> {
        println!("📤 Sending request to: {}", self.url);
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

// ============================================================================
// Authenticator - Pattern Matches on Credential Enum
// ============================================================================

pub struct ServiceAuthenticator;

#[async_trait]
impl StatefulAuthenticator<ServiceCredential> for ServiceAuthenticator {
    type Target = HttpRequestBuilder;
    type Output = HttpRequestBuilder;

    async fn authenticate(
        &self,
        builder: Self::Target,
        state: &ServiceCredentialsState,
    ) -> Result<Self::Output> {
        // Pattern match on credential variant to apply appropriate authentication
        match state {
            ServiceCredentialsState::ApiKey { key, key_prefix } => {
                println!("🔑 Authenticating with API Key");
                let header_value = if let Some(prefix) = key_prefix {
                    format!("{} {}", prefix, key.expose())
                } else {
                    key.expose().to_string()
                };
                Ok(builder.header("X-API-Key", header_value))
            }

            ServiceCredentialsState::OAuth2 { access_token, .. } => {
                println!("🔑 Authenticating with OAuth2");
                let auth_value = format!("Bearer {}", access_token.expose());
                Ok(builder.header("Authorization", auth_value))
            }

            ServiceCredentialsState::BasicAuth { username, password } => {
                println!("🔑 Authenticating with Basic Auth");
                let credentials = format!("{}:{}", username, password.expose());
                let encoded = general_purpose::STANDARD.encode(credentials);
                let auth_value = format!("Basic {}", encoded);
                Ok(builder.header("Authorization", auth_value))
            }

            ServiceCredentialsState::BearerToken { token } => {
                println!("🔑 Authenticating with Bearer Token");
                let auth_value = format!("Bearer {}", token.expose());
                Ok(builder.header("Authorization", auth_value))
            }
        }
    }
}

// ============================================================================
// Service Configuration - NO AuthMethod field!
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Service endpoint URL
    pub endpoint: String,
    /// Credential ID to use (no need to specify auth method!)
    pub credential_id: String,
    /// Optional timeout
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    30
}

// ============================================================================
// Usage Example
// ============================================================================

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Multi-Authentication Service Example\n");
    println!("This demonstrates the recommended pattern for services with multiple auth methods.");
    println!("Key: Enum credentials + Pattern matching, NO AuthMethod in config!\n");

    // Create credential manager
    let manager = CredentialManager::builder()
        .with_store(Arc::new(MockStateStore::new()))
        .with_lock(MockLock::new())
        .build()
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())) })?;

    // Register credential type
    let registry = manager.registry();
    registry.register_credential(ServiceCredential::new());

    println!("{}", "=".repeat(70));
    println!("Scenario 1: API Key Authentication");
    println!("{}", "=".repeat(70));

    // User provides API key credentials
    let api_key_input = ServiceCredentialsInput::ApiKey {
        key: "sk_live_abc123xyz".to_string(),
        key_prefix: Some("Bearer".to_string()),
    };

    let api_key_id = manager
        .create_credential("service", serde_json::to_value(&api_key_input)?)
        .await?;

    // Config only has credential_id, no auth method!
    let config = ServiceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: api_key_id.to_string(),
        timeout_seconds: 30,
    };

    // For demonstration, manually create state (in real usage, would retrieve from manager)
    let api_key_state = ServiceCredentialsState::ApiKey {
        key: SecureString::new("sk_live_abc123xyz".to_string()),
        key_prefix: Some("Bearer".to_string()),
    };

    let authenticator = ServiceAuthenticator;
    let request = HttpRequestBuilder::new(&config.endpoint);
    let authenticated = request
        .authenticate_with_state(&authenticator, &api_key_state)
        .await?;
    let _response = authenticated.send().await?;
    println!("✅ API Key authentication successful!\n");

    println!("{}", "=".repeat(70));
    println!("Scenario 2: OAuth2 Authentication");
    println!("{}", "=".repeat(70));

    // User provides OAuth2 credentials
    let oauth2_input = ServiceCredentialsInput::OAuth2 {
        client_id: "my_client_id".to_string(),
        client_secret: "my_client_secret".to_string(),
        access_token: "ya29.a0AfH6SMC...".to_string(),
        refresh_token: Some("1//0gB8...".to_string()),
        scopes: vec!["read".to_string(), "write".to_string()],
    };

    let oauth2_id = manager
        .create_credential("service", serde_json::to_value(&oauth2_input)?)
        .await?;

    // Same config structure, different credential!
    let config = ServiceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: oauth2_id.to_string(),
        timeout_seconds: 30,
    };

    // For demonstration, manually create state
    let oauth2_state = ServiceCredentialsState::OAuth2 {
        client_id: "my_client_id".to_string(),
        client_secret: SecureString::new("my_client_secret".to_string()),
        access_token: SecureString::new("ya29.a0AfH6SMC...".to_string()),
        refresh_token: Some(SecureString::new("1//0gB8...".to_string())),
        scopes: vec!["read".to_string(), "write".to_string()],
    };

    let request = HttpRequestBuilder::new(&config.endpoint);
    let authenticated = request
        .authenticate_with_state(&authenticator, &oauth2_state)
        .await?;
    let _response = authenticated.send().await?;
    println!("✅ OAuth2 authentication successful!\n");

    // Demonstrate that refresh is supported (would use manager in real usage)
    println!("🔄 OAuth2 credentials support token refresh");

    println!("{}", "=".repeat(70));
    println!("Scenario 3: Basic Auth");
    println!("{}", "=".repeat(70));

    let basic_auth_input = ServiceCredentialsInput::BasicAuth {
        username: "admin".to_string(),
        password: "super_secret_password".to_string(),
    };

    let basic_id = manager
        .create_credential("service", serde_json::to_value(&basic_auth_input)?)
        .await?;

    let config = ServiceConfig {
        endpoint: "https://api.example.com".to_string(),
        credential_id: basic_id.to_string(),
        timeout_seconds: 30,
    };

    // For demonstration, manually create state
    let basic_auth_state = ServiceCredentialsState::BasicAuth {
        username: "admin".to_string(),
        password: SecureString::new("super_secret_password".to_string()),
    };

    let request = HttpRequestBuilder::new(&config.endpoint);
    let authenticated = request
        .authenticate_with_state(&authenticator, &basic_auth_state)
        .await?;
    let _response = authenticated.send().await?;
    println!("✅ Basic Auth successful!\n");

    println!("{}", "=".repeat(70));
    println!("Key Takeaways");
    println!("{}", "=".repeat(70));
    println!("✅ Config only has credential_id (NO AuthMethod field)");
    println!("✅ Enum represents all authentication variants");
    println!("✅ Single authenticator handles all variants via pattern matching");
    println!("✅ Type-safe access to all credential fields");
    println!("✅ Same code structure for all authentication methods");
    println!("✅ Easy to add new authentication methods (just add enum variant)");

    Ok(())
}
