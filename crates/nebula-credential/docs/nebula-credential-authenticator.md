# Nebula Credential - ClientAuthenticator Pattern

## 1. Core Authenticator Trait - nebula-credential/src/authenticator.rs

```rust
//! Client authenticator traits for creating authenticated clients

use crate::core::{AccessToken, CredentialError};
use async_trait::async_trait;

/// Trait for creating authenticated clients from tokens
#[async_trait]
pub trait ClientAuthenticator: Send + Sync {
    /// Input type (what we start with)
    type Target;
    
    /// Output type (what we produce)
    type Output;
    
    /// Authenticate and create the client
    async fn authenticate(
        &self,
        target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError>;
}

/// Extension trait for easy use with credential manager
#[async_trait]
pub trait AuthenticateWith {
    /// Create authenticated client using the authenticator
    async fn authenticate_with<A>(
        self,
        authenticator: &A,
        token: &AccessToken,
    ) -> Result<A::Output, CredentialError>
    where
        A: ClientAuthenticator<Target = Self>,
        Self: Sized;
}

/// Implement for all types that can be targets
#[async_trait]
impl<T> AuthenticateWith for T
where
    T: Send,
{
    async fn authenticate_with<A>(
        self,
        authenticator: &A,
        token: &AccessToken,
    ) -> Result<A::Output, CredentialError>
    where
        A: ClientAuthenticator<Target = Self>,
        Self: Sized,
    {
        authenticator.authenticate(self, token).await
    }
}
```

## 2. Common Authenticators - nebula-credential/src/authenticators/mod.rs

```rust
//! Common authenticator implementations

use crate::authenticator::ClientAuthenticator;
use crate::core::{AccessToken, CredentialError, TokenType};
use async_trait::async_trait;

/// HTTP Bearer token authenticator
pub struct HttpBearer;

#[cfg(feature = "http")]
#[async_trait]
impl ClientAuthenticator for HttpBearer {
    type Target = http::request::Builder;
    type Output = http::request::Builder;
    
    async fn authenticate(
        &self,
        mut builder: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::Bearer) {
            return Err(CredentialError::Invalid);
        }
        
        let auth_value = token.token.with_exposed(|s| format!("Bearer {}", s));
        
        Ok(builder.header("Authorization", auth_value))
    }
}

/// API Key authenticator (header-based)
pub struct ApiKeyHeader {
    pub header_name: String,
}

#[cfg(feature = "http")]
#[async_trait]
impl ClientAuthenticator for ApiKeyHeader {
    type Target = http::request::Builder;
    type Output = http::request::Builder;
    
    async fn authenticate(
        &self,
        mut builder: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::ApiKey) {
            return Err(CredentialError::Invalid);
        }
        
        let key_value = token.token.with_exposed(|s| s.to_string());
        
        Ok(builder.header(&self.header_name, key_value))
    }
}
```

## 3. Updated Core lib.rs - nebula-credential/src/lib.rs

```rust
//! Nebula Credential Core

#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod core;
pub mod manager;
pub mod traits;
pub mod authenticator;
pub mod authenticators;

// Re-export essential types
pub use crate::core::{
    AccessToken,
    Credential,
    CredentialContext,
    CredentialError,
    CredentialMetadata,
    CredentialState,
    SecureString,
    TokenType,
};

pub use crate::manager::{CredentialManager, RefreshPolicy};

pub use crate::traits::{
    StateStore,
    StateVersion,
    TokenCache,
    DistributedLock,
    LockError,
};

pub use crate::authenticator::{
    ClientAuthenticator,
    AuthenticateWith,
};

/// Prelude for convenient imports
pub mod prelude {
    pub use crate::core::*;
    pub use crate::manager::*;
    pub use crate::traits::*;
    pub use crate::authenticator::*;
    pub use async_trait::async_trait;
    pub use serde::{Deserialize, Serialize};
}
```

## 4. Telegram Node with Authenticator - nebula-node-telegram/src/credential/mod.rs

```rust
//! Telegram-specific credential implementation

use nebula_credential::prelude::*;
use serde::{Deserialize, Serialize};
use teloxide::Bot;

/// Telegram bot credential
pub struct TelegramBotCredential;

/// Input for Telegram bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBotInput {
    pub bot_token: String,
    pub webhook_secret: Option<String>,
}

/// State for Telegram bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBotState {
    pub bot_token: SecureString,
    pub webhook_secret: Option<SecureString>,
}

impl CredentialState for TelegramBotState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "telegram_bot";
}

#[async_trait]
impl Credential for TelegramBotCredential {
    type Input = TelegramBotInput;
    type State = TelegramBotState;
    
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "telegram_bot",
            name: "Telegram Bot",
            description: "Telegram bot token",
            supports_refresh: false,
            requires_interaction: false,
        }
    }
    
    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        let state = TelegramBotState {
            bot_token: SecureString::new(&input.bot_token),
            webhook_secret: input.webhook_secret.as_ref().map(|s| SecureString::new(s)),
        };
        
        let token = AccessToken {
            token: SecureString::new(&input.bot_token),
            token_type: TokenType::ApiKey,
            issued_at: std::time::SystemTime::now(),
            expires_at: None,
            scopes: None,
            claims: Default::default(),
        };
        
        Ok((state, Some(token)))
    }
}

/// Authenticator for creating Teloxide Bot
pub struct TeloxideBotAuthenticator;

#[async_trait]
impl ClientAuthenticator for TeloxideBotAuthenticator {
    type Target = ();  // No input needed
    type Output = Bot;
    
    async fn authenticate(
        &self,
        _target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::ApiKey) {
            return Err(CredentialError::Invalid);
        }
        
        let bot_token = token.token.with_exposed(|s| s.to_string());
        Ok(Bot::new(bot_token))
    }
}
```

## 5. OpenAI Node with Authenticator - nebula-node-openai/src/credential/mod.rs

```rust
//! OpenAI-specific credential implementation

use nebula_credential::prelude::*;
use serde::{Deserialize, Serialize};

/// OpenAI API key credential
pub struct OpenAICredential;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIInput {
    pub api_key: String,
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIState {
    pub api_key: SecureString,
    pub organization_id: Option<String>,
}

impl CredentialState for OpenAIState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "openai_api_key";
}

#[async_trait]
impl Credential for OpenAICredential {
    type Input = OpenAIInput;
    type State = OpenAIState;
    
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "openai_api_key",
            name: "OpenAI API Key",
            description: "OpenAI API authentication",
            supports_refresh: false,
            requires_interaction: false,
        }
    }
    
    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        let state = OpenAIState {
            api_key: SecureString::new(&input.api_key),
            organization_id: input.organization_id.clone(),
        };
        
        let mut claims = std::collections::HashMap::new();
        if let Some(org) = &input.organization_id {
            claims.insert("organization_id".to_string(), serde_json::json!(org));
        }
        
        let token = AccessToken {
            token: SecureString::new(&input.api_key),
            token_type: TokenType::Bearer,
            issued_at: std::time::SystemTime::now(),
            expires_at: None,
            scopes: None,
            claims,
        };
        
        Ok((state, Some(token)))
    }
}

/// Authenticator for OpenAI HTTP client
pub struct OpenAIHttpAuthenticator;

#[async_trait]
impl ClientAuthenticator for OpenAIHttpAuthenticator {
    type Target = reqwest::RequestBuilder;
    type Output = reqwest::RequestBuilder;
    
    async fn authenticate(
        &self,
        request: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::Bearer) {
            return Err(CredentialError::Invalid);
        }
        
        let auth_header = token.token.with_exposed(|s| format!("Bearer {}", s));
        
        let mut request = request.header("Authorization", auth_header);
        
        // Add organization header if present
        if let Some(org_id) = token.claims.get("organization_id") {
            if let Some(org) = org_id.as_str() {
                request = request.header("OpenAI-Organization", org);
            }
        }
        
        Ok(request)
    }
}

/// Convenience client creator
pub struct OpenAIClientAuthenticator;

#[async_trait]
impl ClientAuthenticator for OpenAIClientAuthenticator {
    type Target = ();
    type Output = OpenAIClient;  // Your OpenAI client type
    
    async fn authenticate(
        &self,
        _target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        let api_key = token.token.with_exposed(|s| s.to_string());
        let org_id = token.claims.get("organization_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        Ok(OpenAIClient::new(api_key, org_id))
    }
}
```

## 6. AWS Node with Authenticator - nebula-node-aws/src/credential/mod.rs

```rust
//! AWS credential implementation

use nebula_credential::prelude::*;
use aws_sigv4::http_request::{SigningParams, SigningSettings};

/// AWS SigV4 authenticator
pub struct AwsSigV4Authenticator {
    pub region: String,
    pub service: String,
}

#[async_trait]
impl ClientAuthenticator for AwsSigV4Authenticator {
    type Target = http::Request<Vec<u8>>;
    type Output = http::Request<Vec<u8>>;
    
    async fn authenticate(
        &self,
        mut request: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::AwsSigV4) {
            return Err(CredentialError::Invalid);
        }
        
        // Extract AWS credentials from token
        let (access_key, secret_key, session_token) = 
            parse_aws_credentials(token)?;
        
        // Sign the request
        let params = SigningParams::builder()
            .access_key(&access_key)
            .secret_key(&secret_key)
            .session_token(session_token.as_deref())
            .region(&self.region)
            .service(&self.service)
            .time(std::time::SystemTime::now())
            .settings(SigningSettings::default())
            .build()
            .map_err(|e| CredentialError::Unknown(e.to_string()))?;
        
        aws_sigv4::http_request::sign(&mut request, &params)
            .map_err(|e| CredentialError::Unknown(e.to_string()))?;
        
        Ok(request)
    }
}
```

## 7. Usage Examples

```rust
// Using Telegram authenticator
use nebula_node_telegram::credential::{TeloxideBotAuthenticator, TelegramBotCredential};

async fn create_telegram_bot(
    manager: &CredentialManager,
    cred_id: &str,
) -> Result<teloxide::Bot, Box<dyn std::error::Error>> {
    // Get token from manager
    let token = manager.get_token(cred_id).await?;
    
    // Create bot using authenticator
    let authenticator = TeloxideBotAuthenticator;
    let bot = ().authenticate_with(&authenticator, &token).await?;
    
    Ok(bot)
}

// Using OpenAI authenticator
use nebula_node_openai::credential::OpenAIHttpAuthenticator;

async fn make_openai_request(
    manager: &CredentialManager,
    cred_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let token = manager.get_token(cred_id).await?;
    let authenticator = OpenAIHttpAuthenticator;
    
    let client = reqwest::Client::new();
    let request = client
        .post("https://api.openai.com/v1/chat/completions")
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hello!"}]
        }));
    
    // Authenticate the request
    let authenticated_request = request
        .authenticate_with(&authenticator, &token)
        .await?;
    
    let response = authenticated_request.send().await?;
    Ok(response.text().await?)
}

// Generic helper in action handlers
async fn with_authenticated_client<A>(
    manager: &CredentialManager,
    cred_id: &str,
    authenticator: A,
) -> Result<A::Output, CredentialError>
where
    A: ClientAuthenticator<Target = ()>,
{
    let token = manager.get_token(cred_id).await?;
    ().authenticate_with(&authenticator, &token).await
}
```

## 8. Advanced: Composable Authenticators

```rust
/// Chain multiple authenticators
pub struct ChainAuthenticator<A, B> {
    first: A,
    second: B,
}

#[async_trait]
impl<A, B> ClientAuthenticator for ChainAuthenticator<A, B>
where
    A: ClientAuthenticator,
    B: ClientAuthenticator<Target = A::Output>,
    A::Output: Send,
{
    type Target = A::Target;
    type Output = B::Output;
    
    async fn authenticate(
        &self,
        target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        let intermediate = self.first.authenticate(target, token).await?;
        self.second.authenticate(intermediate, token).await
    }
}

// Usage: Add both auth header and rate limiting
let authenticator = ChainAuthenticator {
    first: HttpBearer,
    second: RateLimiter::new(100),
};
```

## Benefits

1. **Type-Safe** - Compile-time verification of authentication
2. **Composable** - Chain authenticators together
3. **Flexible** - Works with any client type
4. **Node-Specific** - Each node defines its own authenticators
5. **Testable** - Easy to mock authenticators
6. **Reusable** - Common patterns in core, specifics in nodes

This pattern gives us maximum flexibility while keeping the core clean!