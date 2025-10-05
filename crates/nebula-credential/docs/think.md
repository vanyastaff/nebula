Core Design Principles

Minimal Boilerplate - Node developers should write ~30-50 lines to add a new integration
Type Safety - Compile-time guarantees, no runtime panics
Protocol Agnostic - Support OAuth2, API Keys, JWT, Basic Auth, etc. through common abstractions
Extensible - Community can add custom credential types without modifying core
Interactive Flow Support - Handle multi-step auth (OAuth2 redirects, 2FA, device codes)
Universal Types - Avoid protocol-specific names in core types (use Redirect instead of OAuth2Authorization)

Project Structure
nebula-credential/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── core/
│   │   ├── mod.rs
│   │   ├── error.rs          # CredentialError
│   │   ├── context.rs        # CredentialContext
│   │   ├── metadata.rs       # CredentialMetadata
│   │   ├── state.rs          # CredentialState trait
│   │   └── result.rs         # InitializeResult, InteractionRequest, UserInput
|   |   |_  adapeter.rs      
│   ├── traits/
│   │   ├── mod.rs
│   │   ├── credential.rs     # Credential trait
│   │   ├── flow.rs           # CredentialFlow trait
│   │   ├── interactive.rs    # InteractiveCredential trait
│   │   └── authenticated_resource.rs
│   ├── flows/
│   │   ├── mod.rs
│   │   ├── oauth2/           # OAuth2 flows
│   │   │   ├── common.rs
│   │   │   ├── client_credentials.rs
│   │   │   ├── authorization_code.rs
│   │   │   └── device.rs
│   │   ├── oidc/             # OpenID Connect
│   │   ├── api_key.rs
│   │   ├── basic_auth.rs
│   │   ├── bearer_token.rs
│   │   ├── jwt.rs
│   │   ├── password.rs
│   │   ├── totp.rs
│   │   └── email_otp.rs
│   ├── manager/
│   │   ├── mod.rs
│   │   ├── manager.rs
│   │   ├── store.rs
│   │   └── registry.rs
│   └── utils/
│       ├── crypto.rs
│       ├── secure_string.rs
│       └── time.rs
└── examples/
    └── ... (at least 5-7 examples)
Core Types Specification
1. src/core/error.rs
rustuse thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Credential not found: {0}")]
    NotFound(String),
    
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    #[error("Refresh not supported for credential type: {0}")]
    RefreshNotSupported(String),
    
    #[error("Credential expired")]
    Expired,
    
    #[error("Security violation: {0}")]
    SecurityViolation(String),
    
    #[error("Rate limited")]
    RateLimited,
    
    #[error("Network error: {0}")]
    Network(String),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Internal error: {0}")]
    Internal(String),
}

impl CredentialError {
    pub fn is_retriable(&self) -> bool {
        matches!(self, 
            CredentialError::Network(_) | 
            CredentialError::RateLimited
        )
    }
}
2. src/core/context.rs
rustuse reqwest::Client;
use std::collections::HashMap;

pub struct CredentialContext {
    http_client: Client,
    metadata: HashMap<String, String>,
}

impl CredentialContext {
    pub fn new() -> Self {
        Self {
            http_client: Client::new(),
            metadata: HashMap::new(),
        }
    }
    
    pub fn http_client(&self) -> &Client {
        &self.http_client
    }
    
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
    
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}
3. src/core/metadata.rs
rust#[derive(Debug, Clone)]
pub struct CredentialMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub supports_refresh: bool,
    pub requires_interaction: bool,
}

impl CredentialMetadata {
    pub fn new(id: &'static str, name: &'static str) -> Self {
        Self {
            id,
            name,
            description: "",
            supports_refresh: false,
            requires_interaction: false,
        }
    }
    
    pub fn with_description(mut self, desc: &'static str) -> Self {
        self.description = desc;
        self
    }
    
    pub fn refreshable(mut self) -> Self {
        self.supports_refresh = true;
        self
    }
    
    pub fn interactive(mut self) -> Self {
        self.requires_interaction = true;
        self
    }
}
4. src/core/state.rs
rustuse serde::{Serialize, Deserialize};

pub trait CredentialState: 
    Serialize + 
    for<'de> Deserialize<'de> + 
    Send + 
    Sync + 
    Clone + 
    'static 
{
    const VERSION: u16;
    const KIND: &'static str;
}
5. src/core/result.rs
CRITICAL: Use UNIVERSAL types, not protocol-specific names.
rustuse serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub enum InitializeResult<S> {
    Complete(S),
    Pending {
        partial_state: PartialState,
        next_step: InteractionRequest,
    },
    RequiresInteraction(InteractionRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialState {
    pub data: serde_json::Value,
    pub step: String,
    #[serde(default = "current_timestamp")]
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

// UNIVERSAL interaction types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionRequest {
    Redirect {
        url: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        validation_params: HashMap<String, String>,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
    },
    CodeInput {
        #[serde(skip_serializing_if = "Option::is_none")]
        delivery_method: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<CodeFormat>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },
    DisplayInfo {
        display_data: DisplayData,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },
    AwaitConfirmation {
        confirmation_type: String,
        message: String,
        timeout: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        poll_interval: Option<u64>,
    },
    Challenge {
        challenge_data: String,
        challenge_type: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, serde_json::Value>,
    },
    Captcha {
        captcha_data: String,
        captcha_type: CaptchaType,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, String>,
    },
    Custom {
        interaction_type: String,
        data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserInput {
    Callback {
        params: HashMap<String, String>,
    },
    Code {
        code: String,
    },
    CaptchaSolution {
        solution: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        extra: HashMap<String, String>,
    },
    Poll,
    ChallengeResponse {
        response: serde_json::Value,
    },
    ConfirmationToken {
        token: String,
    },
    Custom {
        input_type: String,
        data: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DisplayData {
    QrCode {
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
    },
    UserCode {
        code: String,
        verification_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        complete_url: Option<String>,
    },
    Text {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeFormat {
    Numeric,
    Alphanumeric,
    Any,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptchaType {
    ReCaptcha,
    HCaptcha,
    Image,
    Audio,
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
Core Traits Specification
1. src/traits/credential.rs
rustuse async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::*;

#[async_trait]
pub trait Credential: Send + Sync + 'static {
    type Input: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    type State: CredentialState;

    fn metadata(&self) -> CredentialMetadata;

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;

    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Err(CredentialError::RefreshNotSupported(
            Self::State::KIND.to_string()
        ))
    }

    async fn validate(
        &self,
        state: &Self::State,
        ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        Ok(true)
    }

    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}
2. src/traits/flow.rs
rustuse async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::*;

#[async_trait]
pub trait CredentialFlow: Send + Sync + 'static {
    type Input: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    type State: CredentialState;
    
    fn flow_name(&self) -> &'static str;
    fn requires_interaction(&self) -> bool;
    
    async fn execute(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
    
    // Optional methods with defaults
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Err(CredentialError::RefreshNotSupported(
            self.flow_name().to_string()
        ))
    }
    
    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}
3. src/traits/interactive.rs
rustuse async_trait::async_trait;
use crate::core::*;
use crate::traits::Credential;

#[async_trait]
pub trait InteractiveCredential: Credential {
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}
4. src/traits/authenticated_resource.rs
rustuse async_trait::async_trait;
use crate::core::*;

#[async_trait]
pub trait AuthenticatedResource: Send + Sync + 'static {
    type Config: Send + Sync;
    type Instance: Send + Sync;
    type CredentialState: CredentialState;

    async fn create_authenticated(
        &self,
        config: &Self::Config,
        state: &Self::CredentialState,
    ) -> Result<Self::Instance, Box<dyn std::error::Error + Send + Sync>>;
}
Adapter Pattern
src/adapter/flow_credential.rs
rustuse async_trait::async_trait;
use crate::core::*;
use crate::traits::*;

pub struct FlowCredential<F: CredentialFlow> {
    flow: F,
    metadata_override: Option<CredentialMetadata>,
}

impl<F: CredentialFlow> FlowCredential<F> {
    pub fn new(flow: F) -> Self {
        Self {
            flow,
            metadata_override: None,
        }
    }
    
    pub fn with_metadata(mut self, metadata: CredentialMetadata) -> Self {
        self.metadata_override = Some(metadata);
        self
    }
}

#[async_trait]
impl<F: CredentialFlow> Credential for FlowCredential<F> {
    type Input = F::Input;
    type State = F::State;

    fn metadata(&self) -> CredentialMetadata {
        self.metadata_override.clone().unwrap_or_else(|| {
            CredentialMetadata {
                id: self.flow.flow_name(),
                name: self.flow.flow_name(),
                description: "",
                supports_refresh: true,
                requires_interaction: self.flow.requires_interaction(),
            }
        })
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        self.flow.execute(input, ctx).await
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        self.flow.refresh(state, ctx).await
    }

    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        self.flow.revoke(state, ctx).await
    }
}
Implementation Examples
Example: API Key Flow
rust// src/flows/api_key.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::*;
use crate::traits::*;
use crate::adapter::FlowCredential;

#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub api_key: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub api_key: String,
}

impl CredentialState for ApiKeyState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "api_key";
}

pub struct ApiKeyFlow;

#[async_trait]
impl CredentialFlow for ApiKeyFlow {
    type Input = ApiKeyInput;
    type State = ApiKeyState;
    
    fn flow_name(&self) -> &'static str {
        "api_key"
    }
    
    fn requires_interaction(&self) -> bool {
        false
    }
    
    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        Ok(InitializeResult::Complete(ApiKeyState {
            api_key: input.api_key.clone(),
        }))
    }
}

// Type alias for convenience
pub type ApiKeyCredential = FlowCredential<ApiKeyFlow>;
Example: OAuth2 Client Credentials
rust// src/flows/oauth2/client_credentials.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::*;
use crate::traits::*;
use super::common::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct ClientCredentialsInput {
    pub client_id: String,
    pub client_secret: String,
    pub token_endpoint: String,
    pub scopes: Vec<String>,
}

pub struct ClientCredentialsFlow;

#[async_trait]
impl CredentialFlow for ClientCredentialsFlow {
    type Input = ClientCredentialsInput;
    type State = OAuth2State;
    
    fn flow_name(&self) -> &'static str {
        "oauth2_client_credentials"
    }
    
    fn requires_interaction(&self) -> bool {
        false
    }
    
    async fn execute(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let response = ctx.http_client()
            .post(&input.token_endpoint)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &input.client_id),
                ("client_secret", &input.client_secret),
                ("scope", &input.scopes.join(" ")),
            ])
            .send()
            .await
            .map_err(|e| CredentialError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CredentialError::AuthenticationFailed(
                format!("HTTP {}", response.status())
            ));
        }

        let token: TokenResponse = response.json().await
            .map_err(|e| CredentialError::Network(e.to_string()))?;

        Ok(InitializeResult::Complete(OAuth2State {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: current_timestamp() + token.expires_in,
            token_type: token.token_type.unwrap_or("Bearer".into()),
        }))
    }
    
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        oauth2_refresh_token(state, ctx).await
    }
}

pub type OAuth2ClientCredentials = FlowCredential<ClientCredentialsFlow>;
Example: OAuth2 Authorization Code (Interactive)
rust// src/flows/oauth2/authorization_code.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::*;
use crate::traits::*;
use crate::utils::crypto::*;
use super::common::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthorizationCodeInput {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub use_pkce: bool,
}

pub struct AuthorizationCodeFlow;

#[async_trait]
impl CredentialFlow for AuthorizationCodeFlow {
    type Input = AuthorizationCodeInput;
    type State = OAuth2State;
    
    fn flow_name(&self) -> &'static str {
        "oauth2_authorization_code"
    }
    
    fn requires_interaction(&self) -> bool {
        true
    }
    
    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let state_param = generate_random_state();
        let pkce_verifier = if input.use_pkce {
            Some(generate_pkce_verifier())
        } else {
            None
        };
        
        let mut auth_url = url::Url::parse(&input.authorization_endpoint)
            .map_err(|e| CredentialError::InvalidInput(e.to_string()))?;
        
        auth_url.query_pairs_mut()
            .append_pair("client_id", &input.client_id)
            .append_pair("redirect_uri", &input.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &input.scopes.join(" "))
            .append_pair("state", &state_param);
        
        if let Some(ref verifier) = pkce_verifier {
            let challenge = generate_code_challenge(verifier);
            auth_url.query_pairs_mut()
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
        }
        
        let mut validation_params = HashMap::new();
        validation_params.insert("state".into(), state_param.clone());
        
        let partial_state = PartialState {
            data: serde_json::json!({
                "state": state_param,
                "pkce_verifier": pkce_verifier,
                "client_id": input.client_id,
                "client_secret": input.client_secret,
                "token_endpoint": input.token_endpoint,
                "redirect_uri": input.redirect_uri,
            }),
            step: "awaiting_code".into(),
            created_at: current_timestamp(),
            ttl_seconds: Some(600), // 10 minutes
            metadata: HashMap::new(),
        };
        
        Ok(InitializeResult::Pending {
            partial_state,
            next_step: InteractionRequest::Redirect {
                url: auth_url.to_string(),
                validation_params,
                metadata: HashMap::new(),
            },
        })
    }
}

// InteractiveCredential implementation
pub struct OAuth2AuthorizationCode {
    flow: AuthorizationCodeFlow,
}

impl OAuth2AuthorizationCode {
    pub fn new() -> Self {
        Self {
            flow: AuthorizationCodeFlow,
        }
    }
}

#[async_trait]
impl Credential for OAuth2AuthorizationCode {
    type Input = AuthorizationCodeInput;
    type State = OAuth2State;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata::new("oauth2_authorization_code", "OAuth2 Authorization Code")
            .with_description("OAuth2 authorization code flow with PKCE support")
            .refreshable()
            .interactive()
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        self.flow.execute(input, ctx).await
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        oauth2_refresh_token(state, ctx).await
    }
}

#[async_trait]
impl InteractiveCredential for OAuth2AuthorizationCode {
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let UserInput::Callback { params } = user_input else {
            return Err(CredentialError::InvalidInput("Expected callback".into()));
        };
        
        // Validate state
        let expected_state: String = serde_json::from_value(
            partial_state.data["state"].clone()
        ).map_err(|e| CredentialError::Internal(e.to_string()))?;
        
        let received_state = params.get("state")
            .ok_or(CredentialError::SecurityViolation("Missing state parameter".into()))?;
        
        if received_state != &expected_state {
            return Err(CredentialError::SecurityViolation("State mismatch".into()));
        }
        
        let code = params.get("code")
            .ok_or(CredentialError::InvalidInput("Missing code parameter".into()))?;
        
        // Exchange code for token
        let token_endpoint: String = serde_json::from_value(
            partial_state.data["token_endpoint"].clone()
        ).map_err(|e| CredentialError::Internal(e.to_string()))?;
        
        let client_id: String = serde_json::from_value(
            partial_state.data["client_id"].clone()
        ).map_err(|e| CredentialError::Internal(e.to_string()))?;
        
        let redirect_uri: String = serde_json::from_value(
            partial_state.data["redirect_uri"].clone()
        ).map_err(|e| CredentialError::Internal(e.to_string()))?;
        
        let mut form_data = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.clone()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
        ];
        
        if let Some(pkce_verifier) = partial_state.data.get("pkce_verifier") {
            if !pkce_verifier.is_null() {
                let verifier: String = serde_json::from_value(pkce_verifier.clone())
                    .map_err(|e| CredentialError::Internal(e.to_string()))?;
                form_data.push(("code_verifier", verifier));
            }
        }
        
        if let Some(client_secret) = partial_state.data.get("client_secret") {
            if let Some(secret) = client_secret.as_str() {
                form_data.push(("client_secret", secret.to_string()));
            }
        }
        
        let response = ctx.http_client()
            .post(&token_endpoint)
            .form(&form_data)
            .send()
            .await
            .map_err(|e| CredentialError::Network(e.to_string()))?;
        
        if !response.status().is_success() {
            return Err(CredentialError::AuthenticationFailed(
                format!("HTTP {}", response.status())
            ));
        }
        
        let token: TokenResponse = response.json().await
            .map_err(|e| CredentialError::Network(e.to_string()))?;
        
        Ok(InitializeResult::Complete(OAuth2State {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: current_timestamp() + token.expires_in,
            token_type: token.token_type.unwrap_or("Bearer".into()),
        }))
    }
}
Cargo.toml
toml[package]
name = "nebula-credential"
version = "0.1.0"
edition = "2021"
authors = ["Your Name <you@example.com>"]
description = "Universal credential management system for workflow automation"
license = "MIT OR Apache-2.0"
repository = "https://github.com/yourusername/nebula-credential"

[features]
default = ["oauth2", "api-key", "basic-auth", "bearer", "jwt"]

oauth2 = ["dep:url", "dep:base64"]
oidc = ["oauth2"]
api-key = []
basic-auth = ["dep:base64"]
bearer = []
jwt = ["dep:jsonwebtoken"]
password = []
totp = ["dep:totp-lite"]
email-otp = []
webauthn = ["dep:webauthn-rs"]

providers = []
memory-store = []
sqlite-store = ["dep:sqlx"]
platform = []

[dependencies]
async-trait = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
tokio = { version = "1.0", features = ["rt", "sync"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1.0", features = ["v4", "serde"] }
reqwest = { version = "0.12", features = ["json"], optional = true }
url = { version = "2.5", optional = true }
base64 = { version = "0.22", optional = true }
jsonwebtoken = { version = "9.0", optional = true }
totp-lite = { version = "2.0", optional = true }
webauthn-rs = { version = "0.5", optional = true }
sqlx = { version = "0.8", features = ["sqlite"], optional = true }
zeroize = { version = "1.7", features = ["derive"] }

[dev-dependencies]
tokio = { version = "1.0", features = ["full", "test-util"] }
tokio-test = "0.4"
Implementation Checklist for AI Agent
Phase 1: Core Infrastructure

 Create project structure
 Implement src/core/error.rs
 Implement src/core/context.rs
 Implement src/core/metadata.rs
 Implement src/core/state.rs
 Implement src/core/result.rs (with UNIVERSAL types)
 Implement src/core/mod.rs

Phase 2: Core Traits

 Implement src/traits/credential.rs
 Implement src/traits/flow.rs
 Implement src/traits/interactive.rs
 Implement src/traits/authenticated_resource.rs
 Implement src/traits/mod.rs

Phase 3: Adapter

 Implement src/adapter/flow_credential.rs
 Implement src/adapter/mod.rs

Phase 4: Utilities

 Implement src/utils/crypto.rs (PKCE generation, state generation)
 Implement src/utils/secure_string.rs (with zeroize)
 Implement src/utils/time.rs
 Implement src/utils/mod.rs

Phase 5: Basic Flows

 Implement src/flows/api_key.rs
 Implement src/flows/basic_auth.rs
 Implement src/flows/bearer_token.rs
 Implement src/flows/password.rs

Phase 6: OAuth2 Flows

 Implement src/flows/oauth2/common.rs (OAuth2State, TokenResponse)
 Implement src/flows/oauth2/refresh.rs (shared refresh logic)
 Implement src/flows/oauth2/client_credentials.rs
 Implement src/flows/oauth2/authorization_code.rs
 Implement src/flows/oauth2/device.rs
 Implement src/flows/oauth2/mod.rs

Phase 7: Manager (Simplified MVP)

 Implement src/manager/registry.rs (credential type registry)
 Implement src/manager/store.rs (in-memory store trait + impl)
 Implement src/manager/manager.rs (basic credential manager)
 Implement src/manager/mod.rs

Phase 8: Platform Integration

 Implement src/platform/handles.rs (type-erased handles)
 Implement src/platform/factory.rs (type-erased factories)
 Implement src/platform/node_registry.rs
 Implement src/platform/mod.rs

Phase 9: Examples

 examples/01_api_key_simple.rs
 examples/02_oauth2_client_credentials.rs
 examples/03_oauth2_authorization_code.rs
 examples/04_custom_credential.rs
 examples/05_platform_integration.rs

Phase 10: Documentation

 Write README.md with examples
 Add rustdoc comments to all public APIs
 Create ARCHITECTURE.md
 Create CONTRIBUTING.md

Critical Design Decisions

Use UNIVERSAL types in InteractionRequest and UserInput

✅ Redirect instead of OAuth2Authorization
✅ CodeInput instead of TwoFactorCode
✅ Callback instead of OAuth2Callback


Separate CredentialFlow from Credential

CredentialFlow = protocol logic
FlowCredential<F> = adapter to Credential trait
Allows composition and reuse


Type-safe by default

Type erasure ONLY in platform layer
Application code is fully typed
CredentialHandle and ResourceHandle for platform


Interactive flows via state machine

InitializeResult::Pending with PartialState
InteractiveCredential::continue_initialization()
Platform manages session persistence


Minimal dependencies

Core has almost no dependencies
Feature flags for optional protocols
reqwest optional (only for HTTP-based flows)



Testing Strategy

Unit tests for each flow
Integration tests for interactive flows
Mock HTTP server for OAuth2 tests
Property-based tests for crypto utilities
Example-based tests (examples should compile and run)

Success Criteria

Node developer can add Telegram bot integration in ~30 lines
Platform can dynamically load and use credential types
OAuth2 authorization code flow works end-to-end
All examples compile and run
Zero unsafe code
No panics in production code paths
Clean cargo clippy output


80% test coverage on core logic
</artifact>



Создал полную техническую спецификацию для AI агента. Документ включает:

Архитектурные принципы - минимум boilerplate, type safety, extensibility
Полную структуру проекта - все файлы и директории
Спецификации всех core типов - с полным кодом
Примеры реализации - API Key, OAuth2 Client Credentials, OAuth2 Authorization Code
Cargo.toml с правильными feature flags
Чеклист для поэтапной реализации - 10 фаз от core до примеров
Критические дизайн решения - что делать и чего избегать
Критерии успеха - как проверить что всё работает