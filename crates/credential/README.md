# nebula-credential

Universal credential management system for workflow automation.

## Overview

`nebula-credential` provides a secure, extensible credential management system supporting multiple authentication protocols (OAuth2, API Keys, JWT, SAML, etc.) with interactive flows and secure storage.

## Key Features

- **Protocol-Agnostic Flows** - OAuth2, API Keys, JWT, SAML, Kerberos, mTLS
- **Type-Safe Credentials** - Compile-time verification with generic flows
- **Interactive Authentication** - Multi-step flows with user interaction
- **Secure Storage** - Zero-copy secrets with automatic zeroization
- **Minimal Boilerplate** - ~30-50 lines to add new integrations

## Quick Start

### API Key Authentication

```rust
use nebula_credential::prelude::*;

// Create API key credential
let api_key = ApiKeyCredential {
    key: SecureString::from("your-api-key"),
    header_name: "X-API-Key".to_string(),
};

// Use in HTTP request
let request = reqwest::Client::new()
    .get("https://api.example.com/data")
    .header(&api_key.header_name, api_key.key.expose());
```

### OAuth2 Flow

```rust
use nebula_credential::prelude::*;

// Initialize OAuth2 flow
let flow = AuthorizationCodeFlow::new(
    "client-id".to_string(),
    "client-secret".to_string(),
    "https://auth.example.com/authorize".to_string(),
    "https://auth.example.com/token".to_string(),
);

// Start authentication
let init_result = flow.initialize(context).await?;

match init_result {
    InitializeResult::NeedsInteraction(request) => {
        // Show authorization URL to user
        println!("Go to: {}", request.url);

        // After user completes auth, resume with code
        let code = user_provides_code();
        let credential = flow.resume(code, context).await?;

        // credential is now ready to use
    }
    InitializeResult::Complete(credential) => {
        // Already authenticated
    }
}
```

### Basic Authentication

```rust
use nebula_credential::prelude::*;

let basic_auth = BasicAuthCredential {
    username: "user".to_string(),
    password: SecureString::from("password"),
};

// Automatically handles base64 encoding
let auth_header = basic_auth.to_header_value();
```

## Built-in Flows

### Simple Flows
- **ApiKeyFlow** - API key in header/query
- **BasicAuthFlow** - HTTP Basic authentication
- **BearerTokenFlow** - Bearer token authentication
- **PasswordFlow** - Username/password

### OAuth2 Flows
- **AuthorizationCodeFlow** - OAuth2 authorization code grant
- **ClientCredentialsFlow** - OAuth2 client credentials grant
- **DeviceCodeFlow** - OAuth2 device code flow (upcoming)
- **RefreshTokenFlow** - Token refresh

## Interactive Credentials

For flows requiring user interaction (OAuth2, SAML):

```rust
use nebula_credential::prelude::*;

#[async_trait]
impl InteractiveCredential for MyFlow {
    async fn initialize(&self, ctx: &CredentialContext)
        -> Result<InitializeResult, CredentialError> {

        // Return interaction request
        Ok(InitializeResult::NeedsInteraction(InteractionRequest {
            url: "https://auth.example.com/login".to_string(),
            method: "GET".to_string(),
            display_data: DisplayData::AuthorizationUrl {
                url: auth_url,
                code_format: CodeFormat::Alphanumeric,
            },
            ..Default::default()
        }))
    }

    async fn resume(&self, input: UserInput, ctx: &CredentialContext)
        -> Result<FlowCredential, CredentialError> {
        // Handle user's input (code, token, etc.)
        // Return completed credential
    }
}
```

## Secure Storage

```rust
use nebula_credential::core::SecureString;

// Automatically zeroized on drop
let secret = SecureString::from("sensitive-data");

// Access when needed
println!("Secret: {}", secret.expose());

// Automatically cleared from memory when dropped
```

## Credential Context

```rust
use nebula_credential::core::CredentialContext;

let context = CredentialContext {
    user_id: "user-123".to_string(),
    tenant_id: Some("tenant-456".to_string()),
    workflow_id: Some("workflow-789".to_string()),
    redirect_uri: Some("https://app.example.com/callback".to_string()),
    ..Default::default()
};

let credential = flow.initialize(context).await?;
```

## State Management

```rust
#[async_trait]
impl StateStore for MyStateStore {
    async fn save(&self, key: &str, state: PartialState)
        -> Result<(), CredentialError>;

    async fn load(&self, key: &str)
        -> Result<Option<PartialState>, CredentialError>;

    async fn delete(&self, key: &str)
        -> Result<(), CredentialError>;
}
```

## Error Handling

```rust
use nebula_credential::core::CredentialError;

match credential_result {
    Err(CredentialError::InvalidCredentials) => {
        // Handle invalid credentials
    }
    Err(CredentialError::ExpiredCredentials) => {
        // Handle expired credentials - refresh needed
    }
    Err(CredentialError::AuthenticationFailed(msg)) => {
        // Handle auth failure
    }
    Ok(credential) => {
        // Use credential
    }
}
```

## Creating Custom Flows

```rust
use nebula_credential::prelude::*;

pub struct MyCustomFlow {
    // Your config
}

#[async_trait]
impl Credential for MyCustomFlow {
    type Output = MyCredential;

    async fn authenticate(&self, ctx: &CredentialContext)
        -> Result<Self::Output, CredentialError> {
        // Your authentication logic
        Ok(MyCredential { /* ... */ })
    }
}
```

## Best Practices

1. **Use SecureString** - For all sensitive data (passwords, tokens, keys)
2. **Implement refresh** - For OAuth2 and other expiring tokens
3. **Store state securely** - Use encrypted storage for partial states
4. **Validate inputs** - Check redirect URIs and state parameters
5. **Handle errors gracefully** - Provide clear feedback to users

## Architecture

```
nebula-credential/
├── core/              # Core types and errors
├── flows/             # Built-in flows
│   ├── api_key/
│   ├── basic_auth/
│   ├── bearer_token/
│   ├── oauth2/
│   └── password/
├── traits/            # Core traits
└── utils/             # Helper functions
```

## Security

- ✅ Zero-copy secret handling with zeroization
- ✅ PKCE support for OAuth2
- ✅ State parameter validation
- ✅ Redirect URI validation
- ✅ No unsafe code (`#![forbid(unsafe_code)]`)

## License

Licensed under the same terms as the Nebula project.
