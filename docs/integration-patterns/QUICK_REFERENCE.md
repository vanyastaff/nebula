# Credential Authentication Quick Reference

Quick reference guide for choosing and implementing authentication patterns in Nebula.

## Decision Tree: Which Pattern to Use?

```
Does your service support multiple authentication methods?
│
├─ NO (only one auth method)
│  └─ Use SINGLE-AUTH PATTERN
│     - Simple struct for State
│     - No pattern matching
│     - Example: Stripe (API Key only)
│
└─ YES (multiple auth methods)
   └─ Use MULTI-AUTH PATTERN
      - Enum for State variants
      - Pattern matching in authenticator
      - Example: AWS (IAM, STS, Keys, etc.)
```

---

## Pattern Comparison

| Aspect | Single-Auth | Multi-Auth |
|--------|-------------|------------|
| **Use Case** | One auth method only | Multiple auth methods |
| **State Type** | `struct` | `enum` |
| **Authenticator** | Direct field access | Pattern matching |
| **Complexity** | ⭐ Simple | ⭐⭐ More complex |
| **Flexibility** | ⭐ Limited | ⭐⭐⭐ Very flexible |
| **Config** | `credential_id` only | `credential_id` only |
| **Upgrade Path** | Easy → Multi-Auth | Already multi |

---

## Quick Implementation Guides

### Single-Auth Pattern (3 Steps)

**When:** Service supports ONLY ONE authentication method (e.g., Stripe, SendGrid)

**Step 1:** Define credential struct
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub key: String,
    pub header_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub key: SecureString,
    pub header_name: String,
}

impl CredentialState for ApiKeyState {
    const KIND: &'static str = "api_key";
    const VERSION: u16 = 1;
}
```

**Step 2:** Implement Credential
```rust
#[async_trait]
impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type State = ApiKeyState;

    async fn initialize(&self, input: &Input, _ctx: &mut Context)
        -> Result<(State, Option<AccessToken>)> {
        let state = ApiKeyState {
            key: SecureString::new(input.key.clone()),
            header_name: input.header_name.clone(),
        };
        let token = AccessToken::bearer(input.key.clone());
        Ok((state, Some(token)))
    }
    // ... refresh, validate
}
```

**Step 3:** Simple authenticator
```rust
#[async_trait]
impl StatefulAuthenticator<ApiKeyCredential> for MyAuthenticator {
    type Target = HttpConfig;
    type Output = HttpClient;

    async fn authenticate(&self, config: HttpConfig, state: &ApiKeyState)
        -> Result<HttpClient> {
        // Direct access to state fields - no pattern matching!
        let client = HttpClient::new(config.base_url);
        client.header(&state.header_name, state.key.expose())
    }
}
```

**Usage:**
```rust
let config = ServiceConfig {
    endpoint: "https://api.stripe.com".into(),
    credential_id: cred_id.to_string(), // That's it!
};

let client = config
    .authenticate_with_state(&authenticator, &state)
    .await?;
```

---

### Multi-Auth Pattern (4 Steps)

**When:** Service supports MULTIPLE authentication methods (e.g., AWS, Azure)

**Step 1:** Define credential enum
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsInput {
    ApiKey { key: String },
    OAuth2 { client_id: String, client_secret: String, access_token: String },
    BasicAuth { username: String, password: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceCredentialsState {
    ApiKey { key: SecureString },
    OAuth2 { client_id: String, client_secret: SecureString, access_token: SecureString },
    BasicAuth { username: String, password: SecureString },
}

impl CredentialState for ServiceCredentialsState {
    const KIND: &'static str = "service";
    const VERSION: u16 = 1;
}
```

**Step 2:** Implement Credential
```rust
#[async_trait]
impl Credential for ServiceCredential {
    type Input = ServiceCredentialsInput;
    type State = ServiceCredentialsState;

    async fn initialize(&self, input: &Input, _ctx: &mut Context)
        -> Result<(State, Option<AccessToken>)> {
        let (state, token_value) = match input {
            ServiceCredentialsInput::ApiKey { key } => {
                let state = ServiceCredentialsState::ApiKey {
                    key: SecureString::new(key.clone())
                };
                (state, key.clone())
            }
            ServiceCredentialsInput::OAuth2 { access_token, .. } => {
                // ... convert OAuth2
            }
            // ... other variants
        };

        let token = AccessToken::bearer(token_value);
        Ok((state, Some(token)))
    }
    // ... refresh, validate
}
```

**Step 3:** Authenticator with pattern matching
```rust
#[async_trait]
impl StatefulAuthenticator<ServiceCredential> for MyAuthenticator {
    type Target = HttpConfig;
    type Output = HttpClient;

    async fn authenticate(&self, config: HttpConfig, state: &ServiceCredentialsState)
        -> Result<HttpClient> {
        let client = HttpClient::new(config.base_url);

        // Pattern match on credential variant
        match state {
            ServiceCredentialsState::ApiKey { key } => {
                client.header("X-API-Key", key.expose())
            }
            ServiceCredentialsState::OAuth2 { access_token, .. } => {
                client.header("Authorization", format!("Bearer {}", access_token.expose()))
            }
            ServiceCredentialsState::BasicAuth { username, password } => {
                let creds = format!("{}:{}", username, password.expose());
                let encoded = base64::encode(creds);
                client.header("Authorization", format!("Basic {}", encoded))
            }
        }
    }
}
```

**Step 4:** Usage (same as single-auth!)
```rust
let config = ServiceConfig {
    endpoint: "https://api.example.com".into(),
    credential_id: cred_id.to_string(), // Same simple config!
};

let client = config
    .authenticate_with_state(&authenticator, &state)
    .await?;
```

---

## Common Scenarios

### Scenario 1: Stripe-like Service (API Key only)
**Pattern:** Single-Auth
**Reason:** Stripe only uses API Keys
**Example:** [single_auth_service.rs](../../crates/nebula-credential/examples/single_auth_service.rs)

### Scenario 2: Google APIs (OAuth2 only)
**Pattern:** Single-Auth
**Reason:** Google APIs primarily use OAuth2
**Example:** [single_auth_service.rs](../../crates/nebula-credential/examples/single_auth_service.rs)

### Scenario 3: Custom API (API Key OR OAuth2)
**Pattern:** Multi-Auth
**Reason:** Users can choose authentication method
**Example:** [multi_auth_service.rs](../../crates/nebula-credential/examples/multi_auth_service.rs)

### Scenario 4: AWS (Multiple auth types)
**Pattern:** Multi-Auth
**Reason:** AWS supports IAM, STS, Access Keys, etc.
**Example:** [multi_auth_http_client.rs](../../crates/nebula-resource/examples/multi_auth_http_client.rs)

### Scenario 5: Database (Username + Password)
**Pattern:** Single-Auth
**Reason:** Databases typically use one auth method
**Example:** [stateful_authenticator.rs](../../crates/nebula-credential/examples/stateful_authenticator.rs)

---

## Key Principles (Both Patterns)

✅ **Always:**
1. Config contains ONLY `credential_id` (NO `AuthMethod` field!)
2. Use `SecureString` for sensitive data in State
3. Separate Input (plain strings) from State (secure)
4. Implement `validate()` to check state validity
5. Implement `refresh()` only for refreshable credentials

❌ **Never:**
1. Put `AuthMethod` in config (redundant!)
2. Store plain passwords in State (use `SecureString`)
3. Mix Input and State types
4. Skip validation
5. Implement refresh for non-refreshable credentials

---

## Cheat Sheet: Type Signatures

### Single-Auth
```rust
// Credential
type Input = MyInput;              // struct
type State = MyState;              // struct

// Authenticator
impl StatefulAuthenticator<MyCredential> for MyAuthenticator {
    type Target = Config;
    type Output = Client;

    async fn authenticate(
        &self,
        config: Config,
        state: &MyState,           // Direct struct reference
    ) -> Result<Client>
}
```

### Multi-Auth
```rust
// Credential
type Input = MyCredentialsInput;   // enum
type State = MyCredentialsState;   // enum

// Authenticator
impl StatefulAuthenticator<MyCredential> for MyAuthenticator {
    type Target = Config;
    type Output = Client;

    async fn authenticate(
        &self,
        config: Config,
        state: &MyCredentialsState, // Enum reference - pattern match!
    ) -> Result<Client>
}
```

---

## Upgrade Path: Single-Auth → Multi-Auth

If you start with Single-Auth and later need to support multiple methods:

**Step 1:** Convert struct to enum
```rust
// Before (Single-Auth)
pub struct ApiKeyState {
    pub key: SecureString,
}

// After (Multi-Auth)
pub enum CredentialsState {
    ApiKey { key: SecureString },
    OAuth2 { /* ... */ },
}
```

**Step 2:** Update Credential implementation
```rust
// Update initialize, refresh to handle enum variants
async fn initialize(&self, input: &Input, ctx: &mut Context)
    -> Result<(State, Option<Token>)> {
    match input {
        Input::ApiKey { .. } => { /* ... */ },
        Input::OAuth2 { .. } => { /* ... */ },
    }
}
```

**Step 3:** Add pattern matching to authenticator
```rust
// Before (Single-Auth)
async fn authenticate(&self, config: Config, state: &ApiKeyState) -> Result<Client> {
    client.header("X-API-Key", state.key.expose())
}

// After (Multi-Auth)
async fn authenticate(&self, config: Config, state: &CredentialsState) -> Result<Client> {
    match state {
        CredentialsState::ApiKey { key } => {
            client.header("X-API-Key", key.expose())
        }
        CredentialsState::OAuth2 { token, .. } => {
            client.header("Authorization", format!("Bearer {}", token.expose()))
        }
    }
}
```

**Config stays the same!** ✅

---

## Examples

| Example | Pattern | Auth Methods | Link |
|---------|---------|--------------|------|
| single_auth_service.rs | Single-Auth | API Key, OAuth2 (separate) | [View](../../crates/nebula-credential/examples/single_auth_service.rs) |
| multi_auth_service.rs | Multi-Auth | API Key, OAuth2, Basic Auth, Bearer | [View](../../crates/nebula-credential/examples/multi_auth_service.rs) |
| multi_auth_http_client.rs | Multi-Auth | 4 methods in one service | [View](../../crates/nebula-resource/examples/multi_auth_http_client.rs) |
| stateful_authenticator.rs | Single-Auth | Database, Service Account | [View](../../crates/nebula-credential/examples/stateful_authenticator.rs) |

---

## Further Reading

- [Full Integration Guide](./credential-resource-integration.md) - Detailed documentation
- [Integration Patterns Index](./README.md) - All available patterns
- [Integration Summary](../INTEGRATION_SUMMARY.md) - Technical overview

---

**Pro Tip:** Start with Single-Auth if you're unsure. It's simpler and you can always upgrade to Multi-Auth later without changing your config structure!
