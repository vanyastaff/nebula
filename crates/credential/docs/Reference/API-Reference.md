---
title: API Reference
tags: [reference, api, traits, types, p2, priority-2]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: intermediate
---

# nebula-credential API Reference

> [!NOTE] Complete API Documentation
> This reference documents all public traits, types, and functions in nebula-credential. See [[Architecture|Architecture]] for design overview and [[Getting-Started/Core-Concepts|Core Concepts]] for usage patterns.

## Table of Contents

1. [Core Traits](#core-traits)
2. [Credential Types](#credential-types)
3. [Storage Providers](#storage-providers)
4. [Configuration Types](#configuration-types)
5. [Error Types](#error-types)
6. [State Types](#state-types)
7. [Utility Types](#utility-types)

---

## Core Traits

### Credential

Base trait for all credential types.

```rust
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// The authenticated credential output type
    type Output: Send + Sync;
    
    /// Error type for this credential
    type Error: std::error::Error + Send + Sync + 'static;
    
    /// Authenticate and return credential
    async fn authenticate(
        &self,
        ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error>;
    
    /// Check if credential is still valid
    async fn validate(&self, credential: &Self::Output) -> Result<bool, Self::Error>;
    
    /// Refresh expired credential
    async fn refresh(&self, credential: &Self::Output) 
        -> Result<Self::Output, Self::Error>;
}
```

**Associated Types**:
- `Output`: The credential data returned after authentication (e.g., `OAuth2Token`, `ApiKeyCredential`)
- `Error`: Protocol-specific error type (e.g., `OAuth2Error`, `ApiKeyError`)

**Methods**:
- `authenticate()`: Perform initial authentication, returns credential output
- `validate()`: Check if credential is still valid (not expired, not revoked)
- `refresh()`: Attempt to refresh expired credential (returns new credential)

**Example**:
```rust
use nebula_credential::prelude::*;

let api_key = ApiKeyFlow::new(config);
let credential = api_key.authenticate(&ctx).await?;

if !api_key.validate(&credential).await? {
    // Credential expired or invalid
    let refreshed = api_key.refresh(&credential).await?;
}
```

---

### InteractiveCredential

Extends `Credential` for flows requiring user interaction.

```rust
#[async_trait]
pub trait InteractiveCredential: Credential {
    /// Interaction request type
    type Request: InteractionRequest;
    
    /// Initialize authentication flow
    async fn initialize(
        &self,
        ctx: &CredentialContext,
    ) -> Result<FlowState<Self::Request, Self::Output>, Self::Error>;
    
    /// Resume flow after user interaction
    async fn resume(
        &self,
        input: UserInput,
        ctx: &CredentialContext,
    ) -> Result<FlowState<Self::Request, Self::Output>, Self::Error>;
}
```

**Associated Types**:
- `Request`: Type of user interaction needed (e.g., `OAuth2AuthorizationRequest` with URL)

**Methods**:
- `initialize()`: Start flow, returns either `NeedsInteraction(request)` or `Complete(output)`
- `resume()`: Continue flow with user input, returns next state

**Example**:
```rust
// OAuth2 with user authorization
let oauth2 = OAuth2Flow::new(config);

match oauth2.initialize(&ctx).await? {
    FlowState::NeedsInteraction(request) => {
        println!("Visit: {}", request.authorization_url());
        // Wait for user to authorize and get code
        let code = wait_for_callback().await;
        
        match oauth2.resume(UserInput::Code(code), &ctx).await? {
            FlowState::Complete(token) => {
                // OAuth2 token ready
            }
            _ => panic!("unexpected state"),
        }
    }
    FlowState::Complete(token) => {
        // Already authenticated
    }
}
```

---

### RotatableCredential

Extends `Credential` for credentials supporting rotation.

```rust
#[async_trait]
pub trait RotatableCredential: Credential {
    /// Rotation policy configuration
    type Policy: RotationPolicy;
    
    /// Create new credential version
    async fn rotate(
        &self,
        current: &Self::Output,
        policy: &Self::Policy,
    ) -> Result<Self::Output, Self::Error>;
    
    /// Validate old credential during grace period
    async fn validate_rotated(
        &self,
        old: &Self::Output,
        new: &Self::Output,
    ) -> Result<bool, Self::Error>;
}
```

**Associated Types**:
- `Policy`: Rotation policy type (e.g., `PeriodicRotation`, `BeforeExpiryRotation`)

**Methods**:
- `rotate()`: Generate new credential, returns new version
- `validate_rotated()`: Check if old credential still valid during grace period

**Example**:
```rust
let api_key = ApiKeyFlow::new(config);
let current = manager.get("api_key").await?;

// Rotate with 90-day periodic policy
let policy = PeriodicRotation::new(Duration::from_days(90));
let new_credential = api_key.rotate(&current, &policy).await?;

// Both credentials valid during grace period
assert!(api_key.validate_rotated(&current, &new_credential).await?);
```

---

### StorageProvider

Trait for credential storage backends.

```rust
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential
    async fn store(
        &self,
        id: CredentialId,
        data: EncryptedCredential,
    ) -> Result<(), StorageError>;
    
    /// Retrieve encrypted credential
    async fn retrieve(
        &self,
        id: CredentialId,
    ) -> Result<Option<EncryptedCredential>, StorageError>;
    
    /// Delete credential
    async fn delete(&self, id: CredentialId) -> Result<(), StorageError>;
    
    /// Batch store
    async fn store_batch(
        &self,
        batch: Vec<(CredentialId, EncryptedCredential)>,
    ) -> Result<(), StorageError>;
    
    /// Retrieve batch
    async fn retrieve_batch(
        &self,
        ids: Vec<CredentialId>,
    ) -> Result<Vec<Option<EncryptedCredential>>, StorageError>;
    
    /// Delete batch
    async fn delete_batch(&self, ids: Vec<CredentialId>) -> Result<(), StorageError>;
    
    /// List credentials by scope
    async fn list_by_scope(
        &self,
        scope: &CredentialScope,
    ) -> Result<Vec<CredentialId>, StorageError>;
}
```

**Implementations**:
- `LocalStorage`: SQLite-backed local storage
- `AwsSecretsManager`: AWS Secrets Manager
- `VaultStorage`: HashiCorp Vault
- `AzureKeyVault`: Azure Key Vault
- `KubernetesSecrets`: Kubernetes Secrets

**Example**:
```rust
// Custom storage provider
struct RedisStorage {
    client: redis::Client,
}

#[async_trait]
impl StorageProvider for RedisStorage {
    async fn store(&self, id: CredentialId, data: EncryptedCredential) 
        -> Result<(), StorageError> {
        let key = format!("credential:{}", id);
        self.client.set(&key, &data.ciphertext).await?;
        Ok(())
    }
    
    async fn retrieve(&self, id: CredentialId) 
        -> Result<Option<EncryptedCredential>, StorageError> {
        let key = format!("credential:{}", id);
        let data = self.client.get(&key).await?;
        Ok(data)
    }
    
    // ... implement other methods
}
```

---

### CredentialTest

Trait for testing credential validity.

```rust
#[async_trait]
pub trait CredentialTest: Send + Sync {
    /// Test credential by making real authentication attempt
    async fn test(&self) -> Result<TestResult, CredentialError>;
    
    /// Description of what the test does
    fn test_description(&self) -> &str {
        "Testing credential validity"
    }
}

pub enum TestResult {
    Success,
    Failure(String),
    PartialSuccess(Vec<String>), // Warnings
}
```

**Example**:
```rust
// Test OAuth2 credential
let oauth2 = OAuth2Flow::new(config);

match oauth2.test().await? {
    TestResult::Success => println!("✅ Credential valid"),
    TestResult::Failure(err) => println!("❌ Failed: {}", err),
    TestResult::PartialSuccess(warnings) => {
        println!("⚠️  Valid but with warnings:");
        for warning in warnings {
            println!("  - {}", warning);
        }
    }
}
```

---

## Credential Types

### OAuth2Token

OAuth 2.0 access and refresh tokens.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub token_type: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl OAuth2Token {
    pub fn is_expired(&self) -> bool {
        self.expires_at.map_or(false, |exp| Utc::now() >= exp)
    }
    
    pub fn expires_in(&self) -> Option<Duration> {
        self.expires_at.map(|exp| (exp - Utc::now()).to_std().ok()).flatten()
    }
}
```

**Fields**:
- `access_token`: Short-lived token for API access (typically 1-24h)
- `refresh_token`: Long-lived token for obtaining new access tokens
- `token_type`: Token type (usually "Bearer")
- `expires_at`: When access token expires
- `scopes`: Granted permissions

---

### ApiKeyCredential

Simple API key authentication.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub key: SecretString,
    pub key_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: HashMap<String, String>,
}
```

**Fields**:
- `key`: The secret API key (auto-redacted in logs)
- `key_id`: Public identifier for the key
- `created_at`: Creation timestamp
- `expires_at`: Optional expiration time
- `metadata`: Custom key-value pairs

---

### DatabaseCredential

Database connection credentials.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseCredential {
    pub username: String,
    pub password: SecretString,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub ssl_mode: SslMode,
}

pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyCA,
    VerifyFull,
}
```

---

### SAMLAssertion

SAML 2.0 authentication assertion.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SAMLAssertion {
    pub assertion_xml: String,
    pub subject: String,
    pub issuer: String,
    pub not_before: DateTime<Utc>,
    pub not_on_or_after: DateTime<Utc>,
    pub attributes: HashMap<String, Vec<String>>,
}
```

---

### JWTToken

JSON Web Token.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWTToken {
    pub token: SecretString,
    pub algorithm: JWTAlgorithm,
    pub claims: JWTClaims,
}

pub enum JWTAlgorithm {
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    ES256,
    ES384,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWTClaims {
    pub iss: Option<String>,  // Issuer
    pub sub: Option<String>,  // Subject
    pub aud: Option<Vec<String>>,  // Audience
    pub exp: Option<i64>,  // Expiration
    pub nbf: Option<i64>,  // Not before
    pub iat: Option<i64>,  // Issued at
    pub jti: Option<String>,  // JWT ID
    #[serde(flatten)]
    pub custom: HashMap<String, serde_json::Value>,
}
```

---

## Configuration Types

### OAuth2Config

Configuration for OAuth 2.0 flows.

```rust
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: SecretString,
    pub auth_url: Url,
    pub token_url: Url,
    pub redirect_url: Url,
    pub scopes: Vec<String>,
    pub pkce: bool,  // Use PKCE for security
}

impl OAuth2Config {
    pub fn builder() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new()
    }
}

// Builder pattern
pub struct OAuth2ConfigBuilder { /* ... */ }

impl OAuth2ConfigBuilder {
    pub fn client_id(mut self, id: impl Into<String>) -> Self { /* ... */ }
    pub fn client_secret(mut self, secret: impl Into<SecretString>) -> Self { /* ... */ }
    pub fn auth_url(mut self, url: Url) -> Self { /* ... */ }
    pub fn token_url(mut self, url: Url) -> Self { /* ... */ }
    pub fn redirect_url(mut self, url: Url) -> Self { /* ... */ }
    pub fn scopes(mut self, scopes: Vec<String>) -> Self { /* ... */ }
    pub fn enable_pkce(mut self) -> Self { /* ... */ }
    pub fn build(self) -> Result<OAuth2Config, ConfigError> { /* ... */ }
}
```

**Example**:
```rust
let config = OAuth2Config::builder()
    .client_id("my_client_id")
    .client_secret(SecretString::new("secret"))
    .auth_url("https://provider.com/oauth/authorize".parse()?)
    .token_url("https://provider.com/oauth/token".parse()?)
    .redirect_url("http://localhost:8080/callback".parse()?)
    .scopes(vec!["read".into(), "write".into()])
    .enable_pkce()
    .build()?;
```

---

### CredentialManagerConfig

Main configuration for CredentialManager.

```rust
#[derive(Debug, Clone)]
pub struct CredentialManagerConfig {
    pub storage: Arc<dyn StorageProvider>,
    pub encryption: EncryptionConfig,
    pub cache: CacheConfig,
    pub rotation: RotationConfig,
    pub audit: AuditConfig,
}

impl CredentialManagerConfig {
    pub fn builder() -> CredentialManagerConfigBuilder {
        CredentialManagerConfigBuilder::new()
    }
}
```

**Example**:
```rust
let config = CredentialManagerConfig::builder()
    .storage(LocalStorage::new("./secrets"))
    .encryption(EncryptionConfig {
        algorithm: EncryptionAlgorithm::Aes256Gcm,
        key_derivation: KeyDerivation::Argon2id {
            memory_cost: 19456, // 19 MiB
            time_cost: 2,
            parallelism: 1,
        },
    })
    .cache(CacheConfig {
        enabled: true,
        ttl: Duration::from_secs(300),
        max_size: 1_000_000_000, // 1 GB
    })
    .build()?;
```

---

## Error Types

### CredentialError

Top-level error enum.

```rust
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    
    #[error("Encryption error: {0}")]
    Encryption(#[from] EncryptionError),
    
    #[error("OAuth2 error: {0}")]
    OAuth2(#[from] OAuth2Error),
    
    #[error("SAML error: {0}")]
    Saml(#[from] SamlError),
    
    #[error("LDAP error: {0}")]
    Ldap(#[from] LdapError),
    
    #[error("JWT error: {0}")]
    Jwt(#[from] JwtError),
    
    #[error("Credential not found: {0}")]
    NotFound(String),
    
    #[error("Credential expired")]
    Expired,
    
    #[error("Invalid credential")]
    Invalid,
    
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    #[error("Scope violation: {0}")]
    ScopeViolation(String),
}
```

---

### OAuth2Error

OAuth 2.0 specific errors.

```rust
#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
    #[error("Invalid grant: {0}")]
    InvalidGrant(String),
    
    #[error("Invalid client: {0}")]
    InvalidClient(String),
    
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    
    #[error("Unauthorized client")]
    UnauthorizedClient,
    
    #[error("Access denied")]
    AccessDenied,
    
    #[error("Unsupported grant type: {0}")]
    UnsupportedGrantType(String),
    
    #[error("Invalid scope: {0}")]
    InvalidScope(String),
    
    #[error("Token expired")]
    TokenExpired,
    
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
}
```

---

## State Types

### CredentialState

Lifecycle state enum.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialState {
    Uninitialized,
    PendingInteraction,
    Authenticating,
    Active,
    Expired,
    Rotating,
    GracePeriod,
    Revoked,
    Invalid,
}

impl CredentialState {
    /// Check if transition is valid
    pub fn can_transition_to(&self, target: CredentialState) -> bool {
        use CredentialState::*;
        matches!(
            (self, target),
            (Uninitialized, PendingInteraction | Authenticating) |
            (PendingInteraction, Authenticating) |
            (Authenticating, Active | Invalid) |
            (Active, Expired | Rotating | Revoked) |
            (Expired, Rotating | Active) |
            (Rotating, GracePeriod | Active) |
            (GracePeriod, Active | Rotating)
        )
    }
}
```

---

### FlowState

State of interactive credential flow.

```rust
pub enum FlowState<Request, Output> {
    /// Flow needs user interaction
    NeedsInteraction(Request),
    
    /// Flow is complete
    Complete(Output),
    
    /// Flow failed
    Failed(CredentialError),
}
```

---

## Utility Types

### SecretString

Zero-copy string that auto-redacts in logs.

```rust
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        Self { inner: s.into() }
    }
    
    /// Explicit access (auditable)
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl Debug for SecretString {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Display for SecretString {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "***")
    }
}
```

---

### CredentialId

Strongly-typed credential identifier.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialId(String);

impl CredentialId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

---

### CredentialScope

Scope isolation for credentials.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialScope {
    /// Accessible only within specific workflow
    Workflow(String),
    
    /// Accessible to all workflows in organization
    Organization(String),
    
    /// Globally accessible (use sparingly!)
    Global,
}
```

---

## See Also

- [[Architecture|System Architecture]]
- [[Getting-Started/Core-Concepts|Core Concepts]]
- [[Reference/Configuration-Options|Configuration Options]]
- [[Reference/Glossary|Glossary]]
- [[Examples/OAuth2-Flow|OAuth2 Example]]
- [[Examples/API-Key-Basic|API Key Example]]
