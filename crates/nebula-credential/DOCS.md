# nebula-credential Documentation

## For AI Agents

## Crate Purpose

`nebula-credential` is the **universal credential management system** for Nebula workflows. Handles authentication for external services with multiple protocols.

## Core Concepts

### Credential
Authentication data for accessing external services. Can be simple (API key) or complex (OAuth2 with refresh).

### Flow
Authentication protocol implementation (OAuth2, API Key, JWT, etc.). Defines how to obtain credentials.

### InteractiveCredential
Multi-step authentication requiring user interaction (e.g., OAuth2 authorization code).

### SecureString
Zero-copy string that zeroizes memory on drop. Used for passwords, tokens, keys.

## Key Traits

```rust
#[async_trait]
pub trait Credential {
    type Output;
    async fn authenticate(&self, ctx: &CredentialContext)
        -> Result<Self::Output, CredentialError>;
}

#[async_trait]
pub trait InteractiveCredential {
    async fn initialize(&self, ctx: &CredentialContext)
        -> Result<InitializeResult, CredentialError>;

    async fn resume(&self, input: UserInput, ctx: &CredentialContext)
        -> Result<FlowCredential, CredentialError>;
}

pub trait StateStore {
    async fn save(&self, key: &str, state: PartialState) -> Result<(), CredentialError>;
    async fn load(&self, key: &str) -> Result<Option<PartialState>, CredentialError>;
    async fn delete(&self, key: &str) -> Result<(), CredentialError>;
}
```

## Built-in Flows

### Simple (Non-Interactive)
- `ApiKeyFlow` - API key in header/query
- `BasicAuthFlow` - HTTP Basic auth
- `BearerTokenFlow` - Bearer token
- `PasswordFlow` - Username/password

### OAuth2 (Interactive)
- `AuthorizationCodeFlow` - Standard OAuth2 flow
- `ClientCredentialsFlow` - Machine-to-machine
- Refresh token handling

## Module Structure

```
nebula-credential/
├── core/
│   ├── error.rs           # CredentialError types
│   ├── types.rs           # SecureString, CredentialId
│   ├── context.rs         # CredentialContext
│   ├── adapter.rs         # FlowCredential wrapper
│   └── result.rs          # InitializeResult, InteractionRequest
├── flows/
│   ├── api_key/           # API key flow
│   ├── basic_auth/        # Basic auth flow
│   ├── bearer_token/      # Bearer token flow
│   ├── oauth2/            # OAuth2 flows
│   │   ├── authorization_code.rs
│   │   ├── client_credentials.rs
│   │   └── common.rs
│   └── password/          # Password flow
├── traits/
│   ├── credential.rs      # Credential trait
│   ├── interactive.rs     # InteractiveCredential trait
│   └── storage.rs         # StateStore trait
└── utils/
    ├── crypto.rs          # PKCE, hashing
    └── time.rs            # Expiration handling
```

## Integration Points

**Used By**: All workflow nodes/actions accessing external APIs
**Uses**: `nebula-core`, `nebula-error`, `reqwest`, `base64`

## When to Use

✅ Authenticating to external APIs
✅ OAuth2 flows (Google, GitHub, Slack, etc.)
✅ API key management
✅ Multi-step authentication
✅ Secure credential storage

## Common Patterns

### Simple API Key
```rust
let cred = ApiKeyCredential { key: SecureString::from("key"), .. };
request.header(&cred.header_name, cred.key.expose());
```

### OAuth2 Authorization Code
```rust
let flow = AuthorizationCodeFlow::new(..);
let init = flow.initialize(ctx).await?;
// Get authorization URL, redirect user
let code = get_code_from_callback();
let cred = flow.resume(UserInput::Code(code), ctx).await?;
```

### Token Refresh
```rust
if cred.is_expired() {
    cred = flow.refresh(&cred.refresh_token, ctx).await?;
}
```

## Security Features

- **SecureString**: Zeroizes memory on drop
- **PKCE**: Code challenge for OAuth2
- **State validation**: CSRF protection
- **No unsafe code**: `#![forbid(unsafe_code)]`
- **Redirect URI validation**: Prevents token theft

## Error Types

```rust
pub enum CredentialError {
    InvalidCredentials,
    ExpiredCredentials,
    AuthenticationFailed(String),
    MissingRequiredField(String),
    InvalidState,
    NetworkError(reqwest::Error),
    StorageError(String),
}
```

## Performance

- Minimal overhead for simple flows (<1ms)
- OAuth2 flows: Network-bound (100-500ms)
- SecureString: Zero-copy (no heap allocation for expose)

## Thread Safety

- All credential types are `Send + Sync`
- StateStore implementations must be thread-safe
- SecureString is NOT `Clone` (security by design)

## Testing

```bash
cargo test -p nebula-credential
cargo test -p nebula-credential --all-features
```

## Version

See [Cargo.toml](./Cargo.toml)
