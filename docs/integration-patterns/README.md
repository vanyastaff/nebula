# Integration Patterns

This directory contains integration patterns and best practices for combining Nebula crates.

## Available Patterns

### [Credential-Resource Integration](./credential-resource-integration.md)

**Status:** ✅ Stable

Integration pattern for `nebula-credential` + `nebula-resource` to create secure, multi-authentication resources.

**Key Features:**
- ✅ Multiple authentication methods (API Key, OAuth2, Basic Auth, Bearer Token)
- ✅ Type-safe credential access
- ✅ Simplified configuration (NO AuthMethod field needed!)
- ✅ Pattern matching for all auth variants
- ✅ Automatic credential refresh and rotation

**Quick Start:**

```rust
// 1. Define credential enum
pub enum ServiceCredentials {
    ApiKey { key: SecureString, header_name: String },
    OAuth2 { client_id: String, access_token: SecureString, ... },
    BasicAuth { username: String, password: SecureString },
}

// 2. Implement Credential trait
#[async_trait]
impl Credential for ServiceCredential {
    type Input = ServiceCredentialsInput;
    type State = ServiceCredentialsState;
    // ... implement initialize, refresh, validate
}

// 3. Create authenticator with pattern matching
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

// 4. Use it!
let config = ServiceConfig {
    endpoint: "https://api.example.com".into(),
    credential_id: cred_id.to_string(), // That's it!
};

let client = config
    .authenticate_with_state(&authenticator, &state)
    .await?;
```

**Examples:**
- [multi_auth_http_client.rs](../../crates/nebula-resource/examples/multi_auth_http_client.rs) - HTTP client with multi-auth
- [multi_auth_service.rs](../../crates/nebula-credential/examples/multi_auth_service.rs) - Service credentials

**When to use:**
- Building resources that need authentication
- Supporting multiple authentication methods
- Need type-safe access to credential fields (not just tokens)
- Want automatic credential refresh/rotation

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
