# Integration Patterns

This directory contains integration patterns and best practices for combining Nebula crates.

## Available Patterns

### [Credential-Resource Integration](./credential-resource-integration.md)

**Status:** ✅ Stable

Integration pattern for `nebula-credential` + `nebula-resource` to create secure, authenticated resources.

Supports **two patterns**:
1. **Single-Auth** - Service with ONE authentication method (simpler)
2. **Multi-Auth** - Service with MULTIPLE authentication methods (more flexible)

**Key Features:**
- ✅ Single OR multiple authentication methods
- ✅ Type-safe credential access
- ✅ Simplified configuration (NO AuthMethod field needed!)
- ✅ Pattern matching for multi-auth (optional for single-auth)
- ✅ Automatic credential refresh and rotation

**Quick Start (Single-Auth):**

```rust
// 1. Define credential struct (not enum!)
pub struct ApiKeyState {
    pub key: SecureString,
    pub header_name: String,
}

// 2. Implement Credential trait
#[async_trait]
impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type State = ApiKeyState;
    // ... implement initialize, refresh, validate
}

// 3. Simple authenticator - no pattern matching!
#[async_trait]
impl StatefulAuthenticator<ApiKeyCredential> for ApiKeyAuthenticator {
    async fn authenticate(&self, config: Config, state: &ApiKeyState) -> Result<Client> {
        // Direct access to state fields
        let client = HttpClient::new(config.base_url);
        client.header(&state.header_name, state.key.expose())
    }
}

// 4. Use it!
let config = ServiceConfig {
    endpoint: "https://api.stripe.com".into(),
    credential_id: cred_id.to_string(), // That's it!
};

let client = config
    .authenticate_with_state(&authenticator, &state)
    .await?;
```

**Quick Start (Multi-Auth):**

```rust
// 1. Define credential enum
pub enum ServiceCredentials {
    ApiKey { key: SecureString, header_name: String },
    OAuth2 { client_id: String, access_token: SecureString, ... },
    BasicAuth { username: String, password: SecureString },
}

// 2. Implement Credential trait (same as single-auth)
// 3. Authenticator with pattern matching
#[async_trait]
impl StatefulAuthenticator<ServiceCredential> for HttpClientAuthenticator {
    async fn authenticate(&self, config: Config, state: &State) -> Result<Client> {
        match state {
            State::ApiKey { key, .. } => { /* API Key logic */ },
            State::OAuth2 { token, .. } => { /* OAuth2 logic */ },
            State::BasicAuth { username, password } => { /* Basic auth */ },
        }
    }
}
```

**Examples:**
- [single_auth_service.rs](../../crates/nebula-credential/examples/single_auth_service.rs) - **Single-auth** (API Key, OAuth2 separately)
- [multi_auth_service.rs](../../crates/nebula-credential/examples/multi_auth_service.rs) - **Multi-auth** (all methods in one service)
- [multi_auth_http_client.rs](../../crates/nebula-resource/examples/multi_auth_http_client.rs) - HTTP client with multi-auth

**When to use Single-Auth:**
- Service supports ONLY ONE authentication method (e.g., Stripe API Key only)
- Simpler code, no pattern matching needed
- Can upgrade to multi-auth later if needed

**When to use Multi-Auth:**
- Service supports MULTIPLE authentication methods (e.g., AWS supports multiple auth types)
- Need flexibility for users to choose auth method
- More complex but more powerful

---

## Coming Soon

### Context Propagation Pattern
Integration of distributed tracing context with resource management.

### Resource Dependency Graph
Building complex resource dependency graphs with automatic initialization.

### Multi-Tenancy Pattern
Tenant-aware resource management and isolation.

---

## Contributing

When adding a new integration pattern:

1. Create a new markdown file in this directory
2. Follow the structure:
   - Overview and problem statement
   - Solution with code examples
   - Complete implementation guide
   - Best practices
   - Examples and references
3. Add entry to this README
4. Provide working example code in relevant crate(s)
5. Add tests

## Testing

All patterns should include:
- ✅ Complete working examples
- ✅ Unit tests
- ✅ Integration tests
- ✅ Documentation tests (doctests)

## Questions?

- Check the [Nebula documentation](../../README.md)
- Look at examples in crate directories
- Review test files for usage patterns
