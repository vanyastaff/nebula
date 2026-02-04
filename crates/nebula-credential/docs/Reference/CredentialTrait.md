---
title: CredentialTrait
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# Credential Trait Reference

Core trait that defines the interface for all credential types.

## Trait Definition

```rust
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Input parameters for this credential type
    type Input: DeserializeOwned + Serialize + Send + Sync;
    
    /// Persistent state for this credential
    type State: CredentialState;
    
    /// Metadata about this credential type
    fn metadata(&self) -> CredentialMetadata;
    
    /// Initialize the credential
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError>;
    
    /// Get current token
    async fn get_token(
        &self,
        state: &Self::State,
        context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError>;
    
    /// Refresh token if supported
    async fn refresh_token(
        &self,
        state: &mut Self::State,
        context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError>;
    
    /// Validate credential state
    fn validate_state(&self, state: &Self::State) -> Result<(), CredentialError>;
}
```

## Associated Types

### Input

The input parameters required to create this credential type.

```rust
type Input: DeserializeOwned + Serialize + Send + Sync;
```

**Requirements:**

- Must be deserializable from JSON
- Must be serializable to JSON
- Must be thread-safe (`Send + Sync`)

**Example:**

```rust
#[derive(Deserialize, Serialize)]
pub struct ApiKeyInput {
    pub api_key: String,
    pub header_name: String,
    pub prefix: Option<String>,
}
```

### State

The persistent state for this credential.

```rust
type State: CredentialState;
```

**Requirements:**

- Must implement `CredentialState` trait
- Must be serializable/deserializable
- Must track validity and expiration

**Example:**

```rust
#[derive(Serialize, Deserialize)]
pub struct ApiKeyState {
    pub api_key: String,
    pub header_name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CredentialState for ApiKeyState {
    fn is_valid(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() < exp)
            .unwrap_or(true)
    }
    
    fn needs_refresh(&self) -> bool {
        false // API keys don't refresh
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}
```

## Required Methods

### metadata

Returns metadata about the credential type.

```rust
fn metadata(&self) -> CredentialMetadata
```

**Returns:** `CredentialMetadata` containing:

- `id`: Unique identifier for this credential type
- `name`: Human-readable name
- `description`: Description of the credential type
- `supports_refresh`: Whether refresh is supported
- `requires_interaction`: Whether user interaction is needed
- `supported_clients`: List of supported client types

**Example:**

```rust
fn metadata(&self) -> CredentialMetadata {
    CredentialMetadata {
        id: "api_key",
        name: "API Key",
        description: "Simple API key authentication",
        supports_refresh: false,
        requires_interaction: false,
        supported_clients: vec!["http", "grpc"],
    }
}
```

### initialize

Initialize a new credential instance.

```rust
async fn initialize(
    &self,
    input: &Self::Input,
    context: &mut CredentialContext,
) -> Result<InitializeResult, CredentialError>
```

**Parameters:**

- `input`: The input parameters for initialization
- `context`: Mutable credential context for storing state

**Returns:** `InitializeResult` which can be:

- `Ready`: Credential is ready to use
- `RequiresInteraction`: User interaction needed (e.g., OAuth flow)

**Example:**

```rust
async fn initialize(
    &self,
    input: &Self::Input,
    context: &mut CredentialContext,
) -> Result<InitializeResult, CredentialError> {
    // Validate input
    if input.api_key.is_empty() {
        return Err(CredentialError::InvalidInput(
            "API key cannot be empty".into()
        ));
    }
    
    // Create and save state
    let state = ApiKeyState {
        api_key: input.api_key.clone(),
        header_name: input.header_name.clone(),
        created_at: Utc::now(),
        expires_at: None,
    };
    
    context.save_state(&state).await?;
    
    Ok(InitializeResult::Ready)
}
```

### get_token

Retrieve the current token from the credential state.

```rust
async fn get_token(
    &self,
    state: &Self::State,
    context: &mut CredentialContext,
) -> Result<TokenResult, CredentialError>
```

**Parameters:**

- `state`: Current credential state
- `context`: Credential context for additional operations

**Returns:** `TokenResult` which can be:

- `Token(token)`: Valid token
- `NeedsRefresh`: Token needs refresh
- `Expired`: Token has expired

**Example:**

```rust
async fn get_token(
    &self,
    state: &Self::State,
    context: &mut CredentialContext,
) -> Result<TokenResult, CredentialError> {
    if !state.is_valid() {
        return Ok(TokenResult::Expired);
    }
    
    if state.needs_refresh() {
        return Ok(TokenResult::NeedsRefresh);
    }
    
    let token = Token {
        value: SecureString::new(&state.api_key),
        token_type: TokenType::ApiKey,
        expires_at: state.expires_at,
        scopes: vec![],
        claims: HashMap::new(),
    };
    
    Ok(TokenResult::Token(token))
}
```

### refresh_token

Refresh an expiring or expired token.

```rust
async fn refresh_token(
    &self,
    state: &mut Self::State,
    context: &mut CredentialContext,
) -> Result<TokenResult, CredentialError>
```

**Parameters:**

- `state`: Mutable credential state to update
- `context`: Credential context

**Returns:** `TokenResult` with new token or error

**Default Implementation:** Returns `CredentialError::RefreshNotSupported`

**Example:**

```rust
async fn refresh_token(
    &self,
    state: &mut Self::State,
    context: &mut CredentialContext,
) -> Result<TokenResult, CredentialError> {
    let refresh_token = state.refresh_token.as_ref()
        .ok_or(CredentialError::NoRefreshToken)?;
    
    // Make refresh request
    let response = self.refresh_oauth_token(refresh_token).await?;
    
    // Update state
    state.access_token = response.access_token;
    state.expires_at = Utc::now() + Duration::seconds(response.expires_in);
    
    if let Some(new_refresh) = response.refresh_token {
        state.refresh_token = Some(new_refresh);
    }
    
    // Return new token
    Ok(TokenResult::Token(Token {
        value: SecureString::new(&state.access_token),
        token_type: TokenType::Bearer,
        expires_at: Some(state.expires_at),
        scopes: state.scopes.clone(),
        claims: HashMap::new(),
    }))
}
```

## CredentialState Trait

```rust
pub trait CredentialState: Serialize + DeserializeOwned + Send + Sync {
    /// Check if state is still valid
    fn is_valid(&self) -> bool;
    
    /// Check if refresh is needed
    fn needs_refresh(&self) -> bool;
    
    /// Get expiration time if any
    fn expires_at(&self) -> Option<DateTime<Utc>>;
}
```

## InitializeResult

```rust
pub enum InitializeResult {
    /// Credential is ready to use
    Ready,
    
    /// User interaction required
    RequiresInteraction {
        interaction_type: InteractionType,
        state_token: String,
        expires_in: Duration,
    },
}
```

## TokenResult

```rust
pub enum TokenResult {
    /// Valid token
    Token(Token),
    
    /// Token needs refresh
    NeedsRefresh,
    
    /// Token has expired
    Expired,
}
```

## CredentialMetadata

```rust
pub struct CredentialMetadata {
    /// Unique identifier
    pub id: &'static str,
    
    /// Human-readable name
    pub name: &'static str,
    
    /// Description
    pub description: &'static str,
    
    /// Supports token refresh
    pub supports_refresh: bool,
    
    /// Requires user interaction
    pub requires_interaction: bool,
    
    /// Supported client types
    pub supported_clients: Vec<&'static str>,
}
```

## Implementation Example

Complete implementation of a custom credential:

```rust
use nebula_credential::prelude::*;

pub struct CustomApiCredential;

#[derive(Deserialize, Serialize)]
pub struct CustomApiInput {
    pub api_key: String,
    pub api_secret: String,
    pub endpoint: String,
}

#[derive(Serialize, Deserialize)]
pub struct CustomApiState {
    pub api_key: String,
    pub api_secret: String,
    pub endpoint: String,
    pub session_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CredentialState for CustomApiState {
    fn is_valid(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() < exp)
            .unwrap_or(true)
    }
    
    fn needs_refresh(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() + Duration::minutes(5) > exp)
            .unwrap_or(false)
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

#[async_trait]
impl Credential for CustomApiCredential {
    type Input = CustomApiInput;
    type State = CustomApiState;
    
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "custom_api",
            name: "Custom API",
            description: "Custom API with session tokens",
            supports_refresh: true,
            requires_interaction: false,
            supported_clients: vec!["http"],
        }
    }
    
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError> {
        // Create session
        let session = self.create_session(
            &input.api_key,
            &input.api_secret,
            &input.endpoint
        ).await?;
        
        let state = CustomApiState {
            api_key: input.api_key.clone(),
            api_secret: input.api_secret.clone(),
            endpoint: input.endpoint.clone(),
            session_token: Some(session.token),
            expires_at: Some(session.expires_at),
        };
        
        context.save_state(&state).await?;
        Ok(InitializeResult::Ready)
    }
    
    async fn get_token(
        &self,
        state: &Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        if !state.is_valid() {
            return Ok(TokenResult::Expired);
        }
        
        if state.needs_refresh() {
            return Ok(TokenResult::NeedsRefresh);
        }
        
        let token = Token {
            value: SecureString::new(
                state.session_token.as_ref()
                    .ok_or(CredentialError::NoToken)?
            ),
            token_type: TokenType::Custom("session".into()),
            expires_at: state.expires_at,
            scopes: vec![],
            claims: hashmap! {
                "endpoint".to_string() => Value::String(state.endpoint.clone()),
            },
        };
        
        Ok(TokenResult::Token(token))
    }
    
    async fn refresh_token(
        &self,
        state: &mut Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        // Refresh session
        let new_session = self.refresh_session(
            &state.api_key,
            &state.api_secret,
            state.session_token.as_ref()
                .ok_or(CredentialError::NoToken)?
        ).await?;
        
        state.session_token = Some(new_session.token);
        state.expires_at = Some(new_session.expires_at);
        
        self.get_token(state, _context).await
    }
}
```

## Related

- [Credential Types](https://claude.ai/chat/CredentialTypes.md)
- [Credential Manager](https://claude.ai/chat/CredentialManager.md)
- [Error Types](https://claude.ai/chat/ErrorTypes.md)