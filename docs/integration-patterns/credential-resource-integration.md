# Credential-Resource Integration Pattern

This document describes the recommended patterns for integrating `nebula-credential` with `nebula-resource` to create secure, multi-authentication resources.

## Table of Contents

1. [Overview](#overview)
2. [Multi-Authentication Pattern](#multi-authentication-pattern)
3. [Stateful Authenticator Integration](#stateful-authenticator-integration)
4. [Examples](#examples)
5. [Best Practices](#best-practices)

## Overview

The integration between `nebula-credential` and `nebula-resource` enables:

- ✅ **Multiple authentication methods** in a single service (API Key, OAuth2, Basic Auth, Bearer Token)
- ✅ **Type-safe credential access** - full access to all credential fields
- ✅ **Simplified configuration** - no `AuthMethod` field needed
- ✅ **Automatic credential refresh** - handled by nebula-credential
- ✅ **Credential rotation** - seamless updates without downtime
- ✅ **Easy extensibility** - add new auth methods by extending enums

## Multi-Authentication Pattern

### Problem

Many services support multiple authentication methods (e.g., Stripe supports API Key, OAuth2, and Basic Auth). Traditional approaches require:

1. An `AuthMethod` enum in config
2. Separate logic for each auth type
3. Runtime dispatching based on config
4. Duplication of auth logic

### Solution: Enum-Based Credentials

Use a **single enum** to represent all authentication variants, eliminating the need for `AuthMethod` in config.

#### 1. Define Credential Enums

```rust
use nebula_credential::core::{CredentialState, SecureString};
use serde::{Deserialize, Serialize};

/// Input for credential initialization (plain strings)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsInput {
    ApiKey {
        key: String,
        header_name: String,
    },
    OAuth2 {
        client_id: String,
        client_secret: String,
        access_token: String,
        refresh_token: Option<String>,
    },
    BasicAuth {
        username: String,
        password: String,
    },
    BearerToken {
        token: String,
    },
}

/// State for secure storage (SecureString for sensitive data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsState {
    ApiKey {
        key: SecureString,
        header_name: String,
    },
    OAuth2 {
        client_id: String,
        client_secret: SecureString,
        access_token: SecureString,
        refresh_token: Option<SecureString>,
    },
    BasicAuth {
        username: String,
        password: SecureString,
    },
    BearerToken {
        token: SecureString,
    },
}

impl CredentialState for ServiceCredentialsState {
    const KIND: &'static str = "service";
    const VERSION: u16 = 1;
}
```

#### 2. Implement Credential Trait

```rust
use nebula_credential::{
    core::{AccessToken, CredentialContext, CredentialError, CredentialMetadata},
    traits::Credential,
};
use async_trait::async_trait;

pub struct ServiceCredential;

#[async_trait]
impl Credential for ServiceCredential {
    type Input = ServiceCredentialsInput;
    type State = ServiceCredentialsState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "service",
            name: "Service Credentials",
            description: "Multi-method service authentication",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        // Convert plain input to secure state
        let (state, token_value) = match input {
            ServiceCredentialsInput::ApiKey { key, header_name } => {
                let state = ServiceCredentialsState::ApiKey {
                    key: SecureString::new(key.clone()),
                    header_name: header_name.clone(),
                };
                (state, key.clone())
            }
            ServiceCredentialsInput::OAuth2 { access_token, .. } => {
                // ... convert OAuth2
                (state, access_token.clone())
            }
            // ... other variants
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
        // Only OAuth2 can be refreshed
        match state {
            ServiceCredentialsState::OAuth2 { access_token, refresh_token, .. } => {
                if refresh_token.is_some() {
                    // Perform OAuth2 refresh
                    // Update access_token
                    Ok(new_token)
                } else {
                    Err(CredentialError::Internal("No refresh token".into()))
                }
            }
            _ => Err(CredentialError::Internal("Not refreshable".into())),
        }
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        // Validate based on variant
        Ok(true)
    }
}
```

#### 3. Create Authenticator with Pattern Matching

```rust
use nebula_credential::authenticator::StatefulAuthenticator;

pub struct HttpClientAuthenticator;

#[async_trait]
impl StatefulAuthenticator<ServiceCredential> for HttpClientAuthenticator {
    type Target = HttpClientConfig;
    type Output = HttpClient;

    async fn authenticate(
        &self,
        config: Self::Target,
        state: &ServiceCredentialsState,
    ) -> Result<Self::Output, CredentialError> {
        let mut client = HttpClient::new(config.base_url);

        // Pattern match on credential state
        match state {
            ServiceCredentialsState::ApiKey { key, header_name } => {
                println!("🔐 Using API Key authentication");
                let key_value = key.with_exposed(ToString::to_string);
                client = client.with_header(header_name.clone(), key_value);
            }

            ServiceCredentialsState::OAuth2 { access_token, .. } => {
                println!("🔐 Using OAuth2 authentication");
                let token = access_token.with_exposed(ToString::to_string);
                client = client.with_header(
                    "Authorization".into(),
                    format!("Bearer {}", token)
                );
            }

            ServiceCredentialsState::BasicAuth { username, password } => {
                println!("🔐 Using Basic authentication");
                let credentials = password.with_exposed(|pwd| {
                    format!("{}:{}", username, pwd)
                });
                let encoded = base64::encode(credentials);
                client = client.with_header(
                    "Authorization".into(),
                    format!("Basic {}", encoded)
                );
            }

            ServiceCredentialsState::BearerToken { token } => {
                println!("🔐 Using Bearer Token authentication");
                let token_value = token.with_exposed(ToString::to_string);
                client = client.with_header(
                    "Authorization".into(),
                    format!("Bearer {}", token_value)
                );
            }
        }

        Ok(client)
    }
}
```

#### 4. Simplified Resource Configuration

**Key point**: Config contains **ONLY** `credential_id` - NO `AuthMethod` field!

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Service endpoint
    pub endpoint: String,

    /// Credential ID to use
    /// Authentication method is determined by the credential itself!
    pub credential_id: String,

    /// Optional timeout
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
```

#### 5. Usage

```rust
use nebula_credential::authenticator::AuthenticateWithState;

// Setup
let manager = CredentialManager::builder()
    .with_store(Arc::new(store))
    .with_lock(lock)
    .build()?;

manager.registry().register_credential(ServiceCredential);

// Create API Key credential
let api_key_input = ServiceCredentialsInput::ApiKey {
    key: "sk_live_abc123".to_string(),
    header_name: "X-API-Key".to_string(),
};

let cred_id = manager
    .create_credential("service", serde_json::to_value(&api_key_input)?)
    .await?;

// Resource config - ONLY has credential_id!
let config = ServiceConfig {
    endpoint: "https://api.example.com".to_string(),
    credential_id: cred_id.to_string(),
    timeout_seconds: 30,
};

// Authenticate and create client
let client_config = HttpClientConfig {
    base_url: config.endpoint,
    timeout_seconds: config.timeout_seconds,
};

let state = /* get state from manager or construct manually */;

let authenticator = HttpClientAuthenticator;
let client = client_config
    .authenticate_with_state(&authenticator, &state)
    .await?;

// Use authenticated client
let response = client.get("/users").await?;
```

## Stateful Authenticator Integration

### nebula-resource Integration

The `nebula-resource` crate provides integration helpers in `credentials::stateful`:

```rust
use nebula_resource::credentials::stateful::{
    StatefulResourceAuthenticator,
    StatefulCredentialProvider,
    AuthenticateWithStateful,
};

// Implement StatefulResourceAuthenticator
#[async_trait]
impl StatefulResourceAuthenticator<ServiceCredential> for MyResource {
    type Target = Config;
    type Output = Client;

    async fn authenticate(
        &self,
        config: Config,
        state: &ServiceCredentialsState,
    ) -> ResourceResult<Client> {
        // Access all credential fields
        match state {
            ServiceCredentialsState::ApiKey { key, header_name } => {
                // Full access to key and header_name
            }
            // ... other variants
        }
        Ok(client)
    }
}
```

### Provider Pattern

For accessing credential state from resources:

```rust
use nebula_resource::credentials::stateful::StatefulCredentialProvider;

let provider = StatefulCredentialProvider::new(
    Arc::clone(&manager),
    credential_id,
);

// Get full credential state
let state: ServiceCredentialsState = provider
    .get_state::<ServiceCredential>()
    .await?;

// Access all fields
println!("Using credentials for: {}", state.username);
```

## Examples

### Complete Multi-Auth HTTP Client

See [crates/nebula-resource/examples/multi_auth_http_client.rs](../../crates/nebula-resource/examples/multi_auth_http_client.rs) for a complete example showing:

- ✅ API Key authentication
- ✅ OAuth2 authentication with refresh
- ✅ Basic HTTP authentication
- ✅ Bearer Token authentication
- ✅ Single authenticator handling all methods
- ✅ Simplified config (only credential_id)

### Multi-Auth Service Credential

See [crates/nebula-credential/examples/multi_auth_service.rs](../../crates/nebula-credential/examples/multi_auth_service.rs) for credential-focused example.

## Best Practices

### 1. Use Enums for Multi-Auth Services

**DO:**
```rust
pub enum ServiceCredentials {
    ApiKey { key: SecureString },
    OAuth2 { client_id: String, access_token: SecureString },
    BasicAuth { username: String, password: SecureString },
}
```

**DON'T:**
```rust
pub struct ServiceConfig {
    auth_method: AuthMethod,  // ❌ Redundant!
    credential_id: String,
}
```

### 2. Separate Input and State

- **Input**: Plain strings, easy to deserialize from config
- **State**: `SecureString` for sensitive data, stored encrypted

```rust
// Input (from user config)
pub enum CredentialsInput {
    ApiKey { key: String },  // Plain string
}

// State (stored securely)
pub enum CredentialsState {
    ApiKey { key: SecureString },  // Encrypted in memory
}
```

### 3. Pattern Matching in Authenticator

Let the **authenticator** handle all variants - no need for separate authenticators per method:

```rust
async fn authenticate(&self, config: Config, state: &State) -> Result<Client> {
    match state {
        State::ApiKey { key } => { /* API Key logic */ },
        State::OAuth2 { token } => { /* OAuth2 logic */ },
        State::BasicAuth { username, password } => { /* Basic auth logic */ },
    }
}
```

### 4. Implement Refresh Only for Refreshable Credentials

```rust
async fn refresh(&self, state: &mut State, ctx: &mut Context) -> Result<Token> {
    match state {
        State::OAuth2 { refresh_token, .. } if refresh_token.is_some() => {
            // Refresh OAuth2 token
        }
        _ => Err(CredentialError::Internal("Not refreshable".into())),
    }
}
```

### 5. Use Extension Traits for Fluent API

```rust
use nebula_credential::authenticator::AuthenticateWithState;

// Fluent API
let client = config
    .authenticate_with_state(&authenticator, &state)
    .await?;
```

### 6. Config Simplicity

Keep resource config simple - let credentials handle complexity:

```rust
#[derive(Serialize, Deserialize)]
pub struct ResourceConfig {
    pub endpoint: String,
    pub credential_id: String,  // That's it!
}
```

### 7. Test All Authentication Variants

```rust
#[tokio::test]
async fn test_api_key_auth() {
    let input = CredentialsInput::ApiKey { key: "test".into() };
    // Test API Key flow
}

#[tokio::test]
async fn test_oauth2_auth() {
    let input = CredentialsInput::OAuth2 { /* ... */ };
    // Test OAuth2 flow
}
```

## Summary

The credential-resource integration pattern provides:

| Feature | Traditional Approach | nebula Pattern |
|---------|---------------------|----------------|
| Config complexity | `endpoint`, `credential_id`, `auth_method` | `endpoint`, `credential_id` only |
| Auth logic | Scattered across codebase | Centralized in authenticator |
| Type safety | Runtime checks | Compile-time guarantees |
| Extensibility | Modify multiple files | Extend enum, add match arm |
| Credential access | Token string only | Full state access |
| Multi-auth | Complex dispatching | Simple pattern matching |

**Result**: Clean, type-safe, extensible authentication with minimal boilerplate! 🎉
