---
title: nebula-credential — Overview
tags: [nebula, nebula-credential, crate, docs]
status: published
created: 2025-08-24
last_updated: 2025-11-09
---

# nebula-credential — Overview

**nebula-credential** provides secure credential storage, management, and injection for Nebula workflows. It ensures secrets never appear in logs, configurations, or traces while making them easily accessible to actions at runtime.

## What are Credentials?

Credentials in Nebula are secure, typed representations of authentication data such as:

- API keys and tokens
- OAuth2 access/refresh tokens
- Username/password pairs
- TLS/SSL certificates
- Database connection strings
- Cloud provider credentials (AWS, Azure, GCP)
- Custom secret types

Each credential is:

- **Encrypted at rest** using AES-256-GCM
- **Injected at runtime** via secure context (never logged)
- **Versioned** for rotation and rollback
- **Scoped** to specific workflows or actions
- **Auditable** with access logs and compliance tracking

```rust
use nebula_credential::prelude::*;

// Define a credential type
#[derive(Credential)]
struct ApiKeyCredential {
    #[secret] // Automatically redacted from logs
    api_key: String,
    endpoint: String,
}

// Use in an action
async fn execute(&self, input: Input, context: &Context) -> Result<Output> {
    // Credential injected securely
    let cred: ApiKeyCredential = context.get_credential("my_api").await?;

    // api_key is redacted if accidentally logged
    context.log_info(&format!("Using endpoint: {}", cred.endpoint));

    let client = HttpClient::new()
        .header("Authorization", format!("Bearer {}", cred.api_key));

    // ...
}
```

## Why Use nebula-credential?

### Security by Default

❌ **Without nebula-credential:**
```rust
// BAD: Credentials hardcoded in code
let api_key = "sk_live_ABC123DEF456";  // Exposed in version control!

// BAD: Credentials in logs
log::info!("Using API key: {}", api_key);  // Logged in plaintext!

// BAD: Credentials in environment variables
let api_key = env::var("API_KEY").unwrap();  // Visible in process list!
```

✅ **With nebula-credential:**
```rust
// GOOD: Credentials loaded securely
let cred = context.get_credential("api_key").await?;

// GOOD: Automatically redacted from logs
context.log_info(&format!("Credential: {:?}", cred));  // "[REDACTED]"

// GOOD: Encrypted at rest, decrypted in memory only
```

### Key Benefits

- **Never in version control** — Credentials stored separately from code
- **Automatic redaction** — Secrets never appear in logs or traces
- **Encryption at rest** — AES-256-GCM encryption for stored credentials
- **Rotation support** — Update credentials without redeploying workflows
- **Provider integrations** — AWS Secrets Manager, HashiCorp Vault, Azure Key Vault, K8s Secrets
- **Audit trails** — Track who accessed which credential and when
- **Compliance ready** — GDPR, SOC2, HIPAA-compliant credential handling

## Supported Credential Types

| Type | Use Case | Provider | Documentation |
|------|----------|----------|---------------|
| **API Key** | REST API authentication | All | [[Examples/BasicApiKey]] |
| **OAuth2** | Social logins, third-party APIs | All | [[Examples/OAuth2Flow]] |
| **JWT** | Token-based auth | All | [[Examples/JWTTokens]] |
| **Certificate** | mTLS, client certificates | All | [[Examples/CertificateAuth]] |
| **Database** | DB connection strings | All | [[Examples/DatabaseRotation]] |
| **AWS** | AWS services (S3, DynamoDB, etc.) | AWS Secrets Manager | [[Examples/AWSCredentials]] |
| **SSH Key** | SSH connections | All | [[Advanced/SecurityHardening]] |
| **Custom** | Domain-specific credentials | All | [[Examples/CustomCredentialType]] |
| **Composite** | Multiple credentials combined | All | [[Examples/CompositeCredentials]] |

## Documentation Navigation

### 📚 Getting Started

New to nebula-credential? Start here:

- [[Getting-Started/Quick-Start|Quick Start]] - Store your first credential in <10 minutes
- [[Getting-Started/Core-Concepts|Core Concepts]] - Understand credentials, security model, and lifecycle
- [[Getting-Started/Installation|Installation]] - Setup and dependencies

### 📖 Examples

Complete, runnable examples for common scenarios:

**Authentication Protocols**:
- [[Examples/OAuth2-Flow|OAuth2 Flow]] - Authorization Code + PKCE
- [[Examples/OAuth2-GitHub|GitHub OAuth2]] - Complete GitHub integration
- [[Examples/OAuth2-Google|Google OAuth2]] - Google Sign-In
- [[Examples/SAML-Authentication|SAML 2.0]] - Enterprise SSO
- [[Examples/LDAP-Authentication|LDAP]] - Active Directory integration
- [[Examples/JWT-Validation|JWT Tokens]] - Token validation (HS256/RS256/ES256)
- [[Examples/mTLS-Certificate|mTLS]] - Mutual TLS with certificates
- [[Examples/Kerberos-Authentication|Kerberos]] - Kerberos tickets

**Databases**:
- [[Examples/Database-PostgreSQL|PostgreSQL]] - Connection pooling and rotation
- [[Examples/Database-MySQL|MySQL]] - Secure connection strings
- [[Examples/Database-MongoDB|MongoDB]] - Authentication options
- [[Examples/Database-Redis|Redis]] - Password authentication

**Cloud Providers**:
- [[Examples/AWS-Credentials|AWS Credentials]] - Access keys and secrets
- [[Examples/AWS-AssumeRole|AWS AssumeRole]] - Temporary session tokens

**Core Patterns**:
- [[Examples/API-Key-Basic|API Keys]] - Simple token authentication
- [[Examples/SecretString-Usage|SecretString]] - Redaction and zeroization

### 🛠️ How-To Guides

Step-by-step instructions for common tasks:

- [[How-To/Store-Credentials|Store Credentials]] - Save credentials securely
- [[How-To/Retrieve-Credentials|Retrieve Credentials]] - Access credentials with scopes
- [[How-To/Rotate-Credentials|Rotate Credentials]] - Zero-downtime rotation
- [[How-To/Configure-Caching|Configure Caching]] - Performance tuning
- [[How-To/Enable-Audit-Logging|Enable Audit Logging]] - Compliance and monitoring

### 🔌 Integrations

Storage provider setup and configuration:

- [[Integrations/Local-Storage|Local Storage]] - Encrypted local storage (development)
- [[Integrations/AWS-Secrets-Manager|AWS Secrets Manager]] - AWS integration
- [[Integrations/HashiCorp-Vault|HashiCorp Vault]] - Vault Transit engine and KV v2
- [[Integrations/Azure-Key-Vault|Azure Key Vault]] - Azure managed identity
- [[Integrations/Kubernetes-Secrets|Kubernetes Secrets]] - K8s RBAC integration
- [[Integrations/Migration-Guide|Migration Guide]] - Provider-to-provider migration
- [[Integrations/Provider-Comparison|Provider Comparison]] - Feature comparison table

### 🎓 Advanced Topics

In-depth documentation for production deployments:

**Security**:
- [[Advanced/Security-Architecture|Security Architecture]] - Threat model and mitigations
- [[Advanced/Key-Management|Key Management]] - Key rotation, versioning, HSM
- [[Advanced/Access-Control|Access Control]] - Ownership model and ACLs
- [[Advanced/Threat-Model|Threat Model]] - 10 threat scenarios with mitigations
- [[Advanced/Security-Best-Practices|Security Best Practices]] - Secure coding guidelines

**Compliance**:
- [[Advanced/Compliance-SOC2|SOC 2 Type II]] - Requirements mapping
- [[Advanced/Compliance-ISO27001|ISO 27001:2013]] - Standards mapping
- [[Advanced/Compliance-HIPAA|HIPAA]] - Healthcare compliance
- [[Advanced/Compliance-GDPR|GDPR]] - Data protection regulations

**Operations**:
- [[Advanced/Observability-Guide|Observability]] - Prometheus metrics and OpenTelemetry
- [[Advanced/Performance-Tuning|Performance Tuning]] - Latency optimization
- [[Advanced/Rotation-Policies|Rotation Policies]] - Periodic, scheduled, before-expiry
- [[Advanced/Credential-Lifecycle|Credential Lifecycle]] - 11-state state machine
- [[Advanced/Testing-Credentials|Testing Credentials]] - Validation strategies

**Extensibility**:
- [[Advanced/Custom-Providers|Custom Providers]] - Build custom storage backends
- [[Advanced/Type-State-Pattern|Type-State Pattern]] - Compile-time state enforcement

### 🔍 Troubleshooting

Diagnostic guides for common issues:

- [[Troubleshooting/Common-Errors|Common Errors]] - Error catalog with solutions
- [[Troubleshooting/Decryption-Failures|Decryption Failures]] - Key and encryption issues
- [[Troubleshooting/OAuth2-Issues|OAuth2 Issues]] - OAuth2 error codes
- [[Troubleshooting/Rotation-Failures|Rotation Failures]] - Rollback procedures
- [[Troubleshooting/Scope-Violations|Scope Violations]] - ACL debugging
- [[Troubleshooting/Provider-Connectivity|Provider Connectivity]] - AWS/Vault/Azure issues
- [[Troubleshooting/Debugging-Checklist|Debugging Checklist]] - Systematic diagnostics

### 📚 Reference

Technical references and API documentation:

- [[Reference/API-Reference|API Reference]] - Complete API documentation
- [[Reference/Configuration-Options|Configuration Options]] - All config types
- [[Reference/Glossary|Glossary]] - Terminology and acronyms
- [[Architecture|System Architecture]] - Complete architectural design
- [[Security/Encryption|Encryption Details]] - AES-256-GCM, Argon2id, BLAKE3

### 📊 Meta

- [[Documentation-Dashboard|Documentation Dashboard]] - Status tracking and metrics

---

## Quick Start

### 1. Add Dependency

```toml
[dependencies]
nebula-credential = "0.1"
serde = { version = "1.0", features = ["derive"] }
```

### 2. Define a Credential Type

```rust
use nebula_credential::prelude::*;

#[derive(Credential, Serialize, Deserialize)]
struct GitHubCredential {
    #[secret]
    personal_access_token: String,

    username: String,
}
```

### 3. Store a Credential

```rust
use nebula_credential::CredentialManager;

let manager = CredentialManager::new();

let cred = GitHubCredential {
    personal_access_token: "ghp_ABC123...".into(),
    username: "octocat".into(),
};

// Stored encrypted
manager.store("github_token", cred).await?;
```

### 4. Use in an Action

```rust
async fn execute(&self, input: Input, context: &Context) -> Result<Output> {
    let github: GitHubCredential = context.get_credential("github_token").await?;

    let client = octocrab::Octocrab::builder()
        .personal_token(github.personal_access_token)
        .build()?;

    // Use the GitHub API...
}
```

### 5. Rotate a Credential

```rust
// Update without redeploying workflows
let new_cred = GitHubCredential {
    personal_access_token: "ghp_XYZ789...".into(),
    username: "octocat".into(),
};

manager.rotate("github_token", new_cred).await?;
```

## Storage Providers

nebula-credential supports multiple backend storage providers:

### Local Storage (Default)

Stores credentials encrypted on disk:

```rust
let manager = CredentialManager::builder()
    .storage(LocalStorage::new("./secrets"))
    .build();
```

### AWS Secrets Manager

```rust
let manager = CredentialManager::builder()
    .storage(AwsSecretsManager::new("us-east-1"))
    .build();
```

See [[Integrations/AWSSecretsManager]] for configuration.

### HashiCorp Vault

```rust
let manager = CredentialManager::builder()
    .storage(VaultStorage::new("https://vault.example.com"))
    .build();
```

See [[Integrations/HashiCorpVault]] for configuration.

### Azure Key Vault

```rust
let manager = CredentialManager::builder()
    .storage(AzureKeyVault::new("https://myvault.vault.azure.net"))
    .build();
```

See [[Integrations/AzureKeyVault]] for configuration.

### Kubernetes Secrets

```rust
let manager = CredentialManager::builder()
    .storage(KubernetesSecrets::new())
    .build();
```

See [[Integrations/KubernetesSecrets]] for configuration.

## Core Features

### Automatic Redaction

Credentials marked with `#[secret]` are automatically redacted:

```rust
#[derive(Credential)]
struct MyCredential {
    #[secret]
    password: String,  // Redacted in logs

    username: String,  // Not redacted
}

// Logs: "MyCredential { password: [REDACTED], username: alice }"
log::debug!("{:?}", credential);
```

### Credential Rotation

Update credentials without downtime:

```rust
// Old credential still works
let old_cred = context.get_credential("api_key").await?;

// Rotate to new credential
manager.rotate("api_key", new_credential).await?;

// New credential immediately available
let new_cred = context.get_credential("api_key").await?;
```

See [[How-To/RotateCredentials]] for strategies.

### Expiration & Refresh

Automatically refresh expiring credentials:

```rust
#[derive(Credential)]
struct OAuth2Credential {
    #[secret]
    access_token: String,

    #[secret]
    refresh_token: String,

    expires_at: DateTime<Utc>,
}

// Automatically refreshed before expiration
let cred = context.get_credential_auto_refresh("oauth2").await?;
```

See [[How-To/RefreshTokens]] for auto-refresh patterns.

### Caching

Cache credentials in memory for performance:

```rust
let manager = CredentialManager::builder()
    .cache_ttl(Duration::from_secs(300))  // 5-minute cache
    .build();

// First call: fetches from storage
let cred1 = manager.get("api_key").await?;

// Second call: served from cache (fast!)
let cred2 = manager.get("api_key").await?;
```

See [[How-To/CacheCredentials]] for caching strategies.

### Audit Logging

Track credential access for compliance:

```rust
let manager = CredentialManager::builder()
    .audit_log(AuditLogger::new("./audit.log"))
    .build();

// Automatically logged:
// - Who accessed the credential (user/workflow ID)
// - When it was accessed (timestamp)
// - What credential was accessed (credential ID)
// - Result (success/failure)
```

See [[How-To/AuditLogging]] for compliance integration.

## Common Patterns

### OAuth2 Flow

```rust
let oauth_cred = OAuth2Credential::from_client(
    &client_id,
    &client_secret,
    &redirect_uri,
).await?;

manager.store("oauth2", oauth_cred).await?;
```

See [[Examples/OAuth2Flow]] for complete implementation.

### Database Connection

```rust
#[derive(Credential)]
struct PostgresCredential {
    host: String,
    port: u16,
    #[secret]
    username: String,
    #[secret]
    password: String,
    database: String,
}

// Use with connection pooling
let pg_cred = context.get_credential("postgres").await?;
let pool = PgPoolOptions::new()
    .connect(&pg_cred.connection_string())
    .await?;
```

See [[Examples/DatabaseRotation]] for rotation strategies.

### AWS Credentials

```rust
#[derive(Credential)]
struct AwsCredential {
    #[secret]
    access_key_id: String,
    #[secret]
    secret_access_key: String,
    region: String,
}

// Use with AWS SDK
let aws_cred = context.get_credential("aws").await?;
let s3_client = aws_sdk_s3::Client::from_conf(
    aws_sdk_s3::Config::builder()
        .credentials_provider(aws_cred)
        .build()
);
```

See [[Examples/AWSCredentials]] for AWS integration.

## Documentation Structure

- **[[Getting-Started/]]** — Installation, quick start, basic concepts
- **[[How-To/]]** — Task-oriented guides (store, rotate, cache, audit)
- **[[Examples/]]** — Real-world credential types and patterns
- **[[Advanced/]]** — Security hardening, custom providers, zero-knowledge proofs
- **[[Integrations/]]** — External provider integrations (AWS, Azure, Vault, K8s)
- **[[Patterns/]]** — Design patterns (chaining, circuit breaker, scoped access)
- **[[Reference/]]** — API reference, configuration options

## Security Considerations

- **Encryption**: AES-256-GCM with unique keys per credential
- **Key management**: Keys stored separately from credentials (HSM support)
- **Memory safety**: Credentials zeroized after use (using `zeroize` crate)
- **Access control**: RBAC and ABAC support for credential access
- **Compliance**: Audit logs, secret scanning, rotation policies

See [[Advanced/SecurityHardening]] for production security best practices.

## Related Crates

- **[[02-Crates/nebula-action/README|nebula-action]]** — Access credentials in actions via context
- **[[02-Crates/nebula-resource/README|nebula-resource]]** — Combine credentials with resource pooling
- **[[02-Crates/nebula-storage/README|nebula-storage]]** — Persist encrypted credentials

## Getting Help

- **Concepts**: Read [[03-Concepts/Credentials|Credentials concept]] for mental models
- **How-to**: Follow [[Getting-Started/QuickStart]] for step-by-step guidance
- **Examples**: Browse [[Examples/README]] for real-world patterns
- **Integrations**: See [[Integrations/README]] for provider setup

---

**Next**: Start with [[Getting-Started/QuickStart]] or explore [[Examples/README]].

## Deep Dive: Credential System Architecture

### Credential Lifecycle

Every credential in Nebula goes through a defined lifecycle:

```rust
pub enum CredentialState {
    Draft,           // Being created/edited
    Active,          // In use by workflows
    Rotating,        // Being replaced with new version
    Deprecated,      // Still usable but scheduled for removal
    Expired,         // No longer usable
    Revoked,         // Forcefully invalidated
}

pub struct CredentialMetadata {
    pub id: String,
    pub name: String,
    pub state: CredentialState,
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_accessed_at: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub tags: HashMap<String, String>,
}
```

**Lifecycle Flow:**

```
DRAFT → ACTIVE → ROTATING → DEPRECATED → EXPIRED/REVOKED
                     ↓
                  ACTIVE (new version)
```

### The Credential Trait

All credentials implement the `Credential` trait:

```rust
#[async_trait]
pub trait Credential: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Unique credential type identifier
    fn credential_type() -> &'static str;

    /// Validate credential data
    fn validate(&self) -> Result<(), CredentialError> {
        Ok(())
    }

    /// Check if credential is expired
    fn is_expired(&self) -> bool {
        false
    }

    /// Get expiration time if applicable
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        None
    }

    /// Refresh credential if supported (e.g., OAuth2 tokens)
    async fn refresh(&mut self) -> Result<(), CredentialError> {
        Err(CredentialError::RefreshNotSupported)
    }

    /// Redact sensitive fields for logging
    fn redact(&self) -> Self;

    /// Convert to generic credential value
    fn to_value(&self) -> Result<CredentialValue, CredentialError>;

    /// Create from generic credential value
    fn from_value(value: CredentialValue) -> Result<Self, CredentialError>
    where
        Self: Sized;
}
```

### Complete Example: OAuth2 Credential with Auto-Refresh

```rust
use nebula_credential::prelude::*;
use chrono::{DateTime, Utc, Duration};
use reqwest::Client;

#[derive(Serialize, Deserialize, Clone)]
pub struct OAuth2Credential {
    #[secret]
    pub access_token: String,

    #[secret]
    pub refresh_token: String,

    pub token_type: String,
    pub scope: Vec<String>,
    pub expires_at: DateTime<Utc>,

    // Refresh configuration (not serialized to storage)
    #[serde(skip)]
    pub client_id: String,

    #[serde(skip)]
    pub client_secret: String,

    #[serde(skip)]
    pub token_endpoint: String,
}

impl Credential for OAuth2Credential {
    fn credential_type() -> &'static str {
        "oauth2"
    }

    fn validate(&self) -> Result<(), CredentialError> {
        if self.access_token.is_empty() {
            return Err(CredentialError::validation("access_token cannot be empty"));
        }

        if self.refresh_token.is_empty() {
            return Err(CredentialError::validation("refresh_token cannot be empty"));
        }

        if self.expires_at <= Utc::now() {
            return Err(CredentialError::expired("Token has already expired"));
        }

        Ok(())
    }

    fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        Some(self.expires_at)
    }

    async fn refresh(&mut self) -> Result<(), CredentialError> {
        if self.token_endpoint.is_empty() {
            return Err(CredentialError::RefreshNotSupported);
        }

        let client = Client::new();

        let response = client
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.refresh_token),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
            ])
            .send()
            .await
            .map_err(|e| CredentialError::refresh_failed(format!("HTTP error: {}", e)))?;

        if !response.status().is_success() {
            return Err(CredentialError::refresh_failed(format!(
                "Token refresh failed with status: {}",
                response.status()
            )));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| CredentialError::refresh_failed(format!("Parse error: {}", e)))?;

        // Update credential with new tokens
        self.access_token = token_response.access_token;

        if let Some(new_refresh_token) = token_response.refresh_token {
            self.refresh_token = new_refresh_token;
        }

        self.expires_at = Utc::now() + Duration::seconds(token_response.expires_in);

        Ok(())
    }

    fn redact(&self) -> Self {
        Self {
            access_token: "[REDACTED]".to_string(),
            refresh_token: "[REDACTED]".to_string(),
            token_type: self.token_type.clone(),
            scope: self.scope.clone(),
            expires_at: self.expires_at,
            client_id: self.client_id.clone(),
            client_secret: "[REDACTED]".to_string(),
            token_endpoint: self.token_endpoint.clone(),
        }
    }

    fn to_value(&self) -> Result<CredentialValue, CredentialError> {
        Ok(CredentialValue::OAuth2 {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            token_type: self.token_type.clone(),
            scope: self.scope.clone(),
            expires_at: self.expires_at,
        })
    }

    fn from_value(value: CredentialValue) -> Result<Self, CredentialError> {
        match value {
            CredentialValue::OAuth2 {
                access_token,
                refresh_token,
                token_type,
                scope,
                expires_at,
            } => Ok(Self {
                access_token,
                refresh_token,
                token_type,
                scope,
                expires_at,
                client_id: String::new(),
                client_secret: String::new(),
                token_endpoint: String::new(),
            }),
            _ => Err(CredentialError::type_mismatch("Expected OAuth2 credential")),
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: i64,
    scope: Option<String>,
}
```

### Complete Example: Database Credential with Connection Pooling

```rust
use nebula_credential::prelude::*;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Serialize, Deserialize, Clone)]
pub struct PostgresCredential {
    pub host: String,
    pub port: u16,
    pub database: String,

    #[secret]
    pub username: String,

    #[secret]
    pub password: String,

    pub ssl_mode: SslMode,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connection_timeout: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl Credential for PostgresCredential {
    fn credential_type() -> &'static str {
        "postgres"
    }

    fn validate(&self) -> Result<(), CredentialError> {
        if self.host.is_empty() {
            return Err(CredentialError::validation("host cannot be empty"));
        }

        if self.database.is_empty() {
            return Err(CredentialError::validation("database cannot be empty"));
        }

        if self.username.is_empty() {
            return Err(CredentialError::validation("username cannot be empty"));
        }

        if self.max_connections == 0 {
            return Err(CredentialError::validation("max_connections must be > 0"));
        }

        if self.min_connections > self.max_connections {
            return Err(CredentialError::validation(
                "min_connections cannot exceed max_connections"
            ));
        }

        Ok(())
    }

    fn redact(&self) -> Self {
        Self {
            host: self.host.clone(),
            port: self.port,
            database: self.database.clone(),
            username: "[REDACTED]".to_string(),
            password: "[REDACTED]".to_string(),
            ssl_mode: self.ssl_mode,
            max_connections: self.max_connections,
            min_connections: self.min_connections,
            connection_timeout: self.connection_timeout,
        }
    }

    fn to_value(&self) -> Result<CredentialValue, CredentialError> {
        Ok(CredentialValue::Database {
            connection_string: self.connection_string(),
            driver: "postgres".to_string(),
        })
    }

    fn from_value(value: CredentialValue) -> Result<Self, CredentialError> {
        match value {
            CredentialValue::Database { connection_string, .. } => {
                Self::from_connection_string(&connection_string)
            }
            _ => Err(CredentialError::type_mismatch("Expected database credential")),
        }
    }
}

impl PostgresCredential {
    pub fn connection_string(&self) -> String {
        let ssl_mode_str = match self.ssl_mode {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        };

        format!(
            "postgres://{}:{}@{}:{}/{}?sslmode={}",
            urlencoding::encode(&self.username),
            urlencoding::encode(&self.password),
            self.host,
            self.port,
            self.database,
            ssl_mode_str
        )
    }

    pub fn from_connection_string(conn_str: &str) -> Result<Self, CredentialError> {
        // Parse connection string
        let url = url::Url::parse(conn_str)
            .map_err(|e| CredentialError::parse_error(format!("Invalid URL: {}", e)))?;

        Ok(Self {
            host: url.host_str().unwrap_or("localhost").to_string(),
            port: url.port().unwrap_or(5432),
            database: url.path().trim_start_matches('/').to_string(),
            username: url.username().to_string(),
            password: url.password().unwrap_or("").to_string(),
            ssl_mode: SslMode::Prefer,
            max_connections: 10,
            min_connections: 1,
            connection_timeout: 30,
        })
    }

    pub async fn create_pool(&self) -> Result<PgPool, CredentialError> {
        PgPoolOptions::new()
            .max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .acquire_timeout(std::time::Duration::from_secs(self.connection_timeout))
            .connect(&self.connection_string())
            .await
            .map_err(|e| CredentialError::connection_failed(format!("Pool creation failed: {}", e)))
    }
}
```

### Complete Example: AWS Credential with AssumeRole

```rust
use nebula_credential::prelude::*;
use aws_config::SdkConfig;
use aws_types::credentials::SharedCredentialsProvider;

#[derive(Serialize, Deserialize, Clone)]
pub struct AwsCredential {
    #[secret]
    pub access_key_id: String,

    #[secret]
    pub secret_access_key: String,

    #[secret]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,

    pub region: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl Credential for AwsCredential {
    fn credential_type() -> &'static str {
        "aws"
    }

    fn validate(&self) -> Result<(), CredentialError> {
        if self.access_key_id.is_empty() {
            return Err(CredentialError::validation("access_key_id cannot be empty"));
        }

        if self.secret_access_key.is_empty() {
            return Err(CredentialError::validation("secret_access_key cannot be empty"));
        }

        if self.region.is_empty() {
            return Err(CredentialError::validation("region cannot be empty"));
        }

        Ok(())
    }

    fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| Utc::now() >= exp).unwrap_or(false)
    }

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    async fn refresh(&mut self) -> Result<(), CredentialError> {
        if let Some(role_arn) = &self.role_arn {
            // Use STS AssumeRole to get temporary credentials
            let sts_config = self.create_config().await?;
            let sts_client = aws_sdk_sts::Client::new(&sts_config);

            let mut assume_role_req = sts_client
                .assume_role()
                .role_arn(role_arn)
                .role_session_name(format!("nebula-session-{}", Uuid::new_v4()));

            if let Some(external_id) = &self.external_id {
                assume_role_req = assume_role_req.external_id(external_id);
            }

            let response = assume_role_req
                .send()
                .await
                .map_err(|e| CredentialError::refresh_failed(format!("AssumeRole failed: {}", e)))?;

            let credentials = response.credentials
                .ok_or_else(|| CredentialError::refresh_failed("No credentials in response"))?;

            self.access_key_id = credentials.access_key_id;
            self.secret_access_key = credentials.secret_access_key;
            self.session_token = Some(credentials.session_token);
            self.expires_at = credentials.expiration.map(|exp| {
                DateTime::from_timestamp(exp.secs(), 0).unwrap()
            });

            Ok(())
        } else {
            Err(CredentialError::RefreshNotSupported)
        }
    }

    fn redact(&self) -> Self {
        Self {
            access_key_id: format!("{}***", &self.access_key_id[..4]),
            secret_access_key: "[REDACTED]".to_string(),
            session_token: self.session_token.as_ref().map(|_| "[REDACTED]".to_string()),
            region: self.region.clone(),
            role_arn: self.role_arn.clone(),
            external_id: self.external_id.as_ref().map(|_| "[REDACTED]".to_string()),
            expires_at: self.expires_at,
        }
    }

    fn to_value(&self) -> Result<CredentialValue, CredentialError> {
        Ok(CredentialValue::Aws {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            region: self.region.clone(),
        })
    }

    fn from_value(value: CredentialValue) -> Result<Self, CredentialError> {
        match value {
            CredentialValue::Aws {
                access_key_id,
                secret_access_key,
                session_token,
                region,
            } => Ok(Self {
                access_key_id,
                secret_access_key,
                session_token,
                region,
                role_arn: None,
                external_id: None,
                expires_at: None,
            }),
            _ => Err(CredentialError::type_mismatch("Expected AWS credential")),
        }
    }
}

impl AwsCredential {
    pub async fn create_config(&self) -> Result<SdkConfig, CredentialError> {
        use aws_credential_types::Credentials;

        let credentials = if let Some(session_token) = &self.session_token {
            Credentials::new(
                &self.access_key_id,
                &self.secret_access_key,
                Some(session_token.clone()),
                self.expires_at.map(|exp| {
                    aws_smithy_types::DateTime::from_secs(exp.timestamp())
                }),
                "nebula-credential",
            )
        } else {
            Credentials::new(
                &self.access_key_id,
                &self.secret_access_key,
                None,
                None,
                "nebula-credential",
            )
        };

        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(self.region.clone()))
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .load()
            .await;

        Ok(config)
    }

    pub async fn assume_role(
        &self,
        role_arn: &str,
        external_id: Option<String>,
    ) -> Result<Self, CredentialError> {
        let mut new_cred = self.clone();
        new_cred.role_arn = Some(role_arn.to_string());
        new_cred.external_id = external_id;
        new_cred.refresh().await?;
        Ok(new_cred)
    }
}
```

## Advanced Patterns

### Pattern 1: Credential Chaining

Chain multiple credentials with fallback:

```rust
use nebula_credential::prelude::*;

pub struct CredentialChain {
    credentials: Vec<(String, Box<dyn Credential>)>,
}

impl CredentialChain {
    pub fn new() -> Self {
        Self {
            credentials: Vec::new(),
        }
    }

    pub fn add<C: Credential>(mut self, name: &str, credential: C) -> Self {
        self.credentials.push((name.to_string(), Box::new(credential)));
        self
    }

    pub async fn get_first_valid(&self) -> Result<&dyn Credential, CredentialError> {
        for (name, cred) in &self.credentials {
            if !cred.is_expired() && cred.validate().is_ok() {
                return Ok(cred.as_ref());
            }
        }

        Err(CredentialError::not_found("No valid credential in chain"))
    }
}

// Usage
let chain = CredentialChain::new()
    .add("primary", aws_cred_1)
    .add("backup", aws_cred_2)
    .add("fallback", aws_cred_3);

let active_cred = chain.get_first_valid().await?;
```

### Pattern 2: Credential Rotation with Zero Downtime

Implement blue-green credential rotation:

```rust
use nebula_credential::prelude::*;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct RotatingCredential<C: Credential> {
    active: Arc<RwLock<C>>,
    standby: Arc<RwLock<Option<C>>>,
    manager: Arc<CredentialManager>,
    name: String,
}

impl<C: Credential + Clone> RotatingCredential<C> {
    pub fn new(initial: C, manager: Arc<CredentialManager>, name: String) -> Self {
        Self {
            active: Arc::new(RwLock::new(initial)),
            standby: Arc::new(RwLock::new(None)),
            manager,
            name,
        }
    }

    /// Get currently active credential
    pub async fn get(&self) -> C {
        self.active.read().await.clone()
    }

    /// Prepare new credential without activating
    pub async fn prepare_rotation(&self, new_cred: C) -> Result<(), CredentialError> {
        // Validate new credential
        new_cred.validate()?;

        // Store in standby
        let mut standby = self.standby.write().await;
        *standby = Some(new_cred);

        Ok(())
    }

    /// Activate standby credential (blue-green switch)
    pub async fn commit_rotation(&self) -> Result<(), CredentialError> {
        let mut standby = self.standby.write().await;

        if let Some(new_cred) = standby.take() {
            // Atomic swap
            let mut active = self.active.write().await;
            let old_cred = std::mem::replace(&mut *active, new_cred.clone());

            // Persist to storage
            self.manager.store(&self.name, new_cred).await?;

            // Keep old credential as standby for potential rollback
            *standby = Some(old_cred);

            Ok(())
        } else {
            Err(CredentialError::invalid_state("No standby credential prepared"))
        }
    }

    /// Rollback to previous credential
    pub async fn rollback(&self) -> Result<(), CredentialError> {
        let mut standby = self.standby.write().await;

        if let Some(old_cred) = standby.take() {
            let mut active = self.active.write().await;
            *active = old_cred.clone();

            // Persist rollback
            self.manager.store(&self.name, old_cred).await?;

            Ok(())
        } else {
            Err(CredentialError::invalid_state("No previous credential to rollback to"))
        }
    }
}

// Usage
let rotating_cred = RotatingCredential::new(
    initial_aws_cred,
    manager.clone(),
    "aws_prod".to_string(),
);

// Prepare new credential
rotating_cred.prepare_rotation(new_aws_cred).await?;

// Test new credential
let test_result = test_credential(&rotating_cred.standby.read().await.as_ref().unwrap()).await;

if test_result.is_ok() {
    // Commit rotation
    rotating_cred.commit_rotation().await?;
} else {
    // Discard standby
    *rotating_cred.standby.write().await = None;
}
```

### Pattern 3: Circuit Breaker for Credential Providers

Prevent cascading failures when fetching credentials:

```rust
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct CircuitBreakerCredentialProvider<P: CredentialProvider> {
    inner: P,
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,
    state: Arc<CircuitState>,
}

struct CircuitState {
    state: AtomicU32,  // 0 = Closed, 1 = Open, 2 = HalfOpen
    failure_count: AtomicU32,
    success_count: AtomicU32,
    last_failure_time: AtomicU64,
}

#[async_trait]
impl<P: CredentialProvider> CredentialProvider for CircuitBreakerCredentialProvider<P> {
    async fn get(&self, name: &str) -> Result<Box<dyn Credential>, CredentialError> {
        // Check circuit state
        if self.state.is_open() {
            let elapsed = self.state.elapsed_since_failure();

            if elapsed < self.timeout {
                return Err(CredentialError::circuit_breaker_open(format!(
                    "Circuit breaker open, retry after {:?}",
                    self.timeout - elapsed
                )));
            }

            // Timeout expired, try half-open
            self.state.half_open();
        }

        // Attempt to get credential
        match self.inner.get(name).await {
            Ok(credential) => {
                let success_count = self.state.record_success();

                // If half-open and enough successes, close circuit
                if self.state.is_half_open() && success_count >= self.success_threshold {
                    self.state.close();
                }

                Ok(credential)
            }
            Err(e) => {
                let failure_count = self.state.record_failure();

                // Open circuit if threshold exceeded
                if failure_count >= self.failure_threshold {
                    self.state.open();
                }

                Err(e)
            }
        }
    }
}
```

### Pattern 4: Cached Credential Provider with TTL

Implement intelligent caching with automatic refresh:

```rust
use lru::LruCache;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct CachedCredentialProvider<P: CredentialProvider> {
    inner: P,
    cache: Arc<Mutex<LruCache<String, CachedEntry>>>,
    ttl: Duration,
    refresh_before_expiry: Duration,
}

struct CachedEntry {
    credential: Box<dyn Credential>,
    cached_at: Instant,
    expires_at: Option<DateTime<Utc>>,
}

impl<P: CredentialProvider> CachedCredentialProvider<P> {
    pub fn new(inner: P, capacity: usize, ttl: Duration) -> Self {
        Self {
            inner,
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            ttl,
            refresh_before_expiry: Duration::from_secs(300),  // 5 minutes
        }
    }

    async fn should_refresh(&self, entry: &CachedEntry) -> bool {
        // Cache TTL expired
        if entry.cached_at.elapsed() > self.ttl {
            return true;
        }

        // Credential has expiration and is near expiry
        if let Some(expires_at) = entry.expires_at {
            let time_until_expiry = expires_at - Utc::now();
            if time_until_expiry.num_seconds() < self.refresh_before_expiry.as_secs() as i64 {
                return true;
            }
        }

        // Credential reports itself as expired
        if entry.credential.is_expired() {
            return true;
        }

        false
    }
}

#[async_trait]
impl<P: CredentialProvider> CredentialProvider for CachedCredentialProvider<P> {
    async fn get(&self, name: &str) -> Result<Box<dyn Credential>, CredentialError> {
        let mut cache = self.cache.lock().await;

        // Check cache
        if let Some(entry) = cache.get(name) {
            if !self.should_refresh(entry).await {
                return Ok(entry.credential.clone());
            }
        }

        // Cache miss or expired, fetch fresh credential
        drop(cache);  // Release lock during network call

        let credential = self.inner.get(name).await?;
        let expires_at = credential.expires_at();

        let entry = CachedEntry {
            credential: credential.clone(),
            cached_at: Instant::now(),
            expires_at,
        };

        // Update cache
        let mut cache = self.cache.lock().await;
        cache.put(name.to_string(), entry);

        Ok(credential)
    }
}
```

### Pattern 5: Composite Credentials

Combine multiple credentials into a single logical unit:

```rust
#[derive(Serialize, Deserialize)]
pub struct CompositeCredential {
    pub credentials: HashMap<String, Box<dyn Credential>>,
}

impl CompositeCredential {
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
        }
    }

    pub fn add<C: Credential>(mut self, name: &str, credential: C) -> Self {
        self.credentials.insert(name.to_string(), Box::new(credential));
        self
    }

    pub fn get<C: Credential>(&self, name: &str) -> Result<&C, CredentialError> {
        self.credentials
            .get(name)
            .ok_or_else(|| CredentialError::not_found(name))?
            .downcast_ref::<C>()
            .ok_or_else(|| CredentialError::type_mismatch("Type mismatch"))
    }
}

// Example: Multi-service workflow
let composite = CompositeCredential::new()
    .add("aws", aws_credential)
    .add("github", github_credential)
    .add("slack", slack_credential);

manager.store("multi_service", composite).await?;

// In action
let composite: CompositeCredential = context.get_credential("multi_service").await?;
let aws_cred: &AwsCredential = composite.get("aws")?;
let github_cred: &GitHubCredential = composite.get("github")?;
```

## Encryption Deep Dive

### AES-256-GCM Encryption Implementation

nebula-credential uses AES-256-GCM for authenticated encryption:

```rust
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{Argon2, PasswordHasher};
use rand::RngCore;

pub struct CredentialEncryption {
    cipher: Aes256Gcm,
}

impl CredentialEncryption {
    /// Create from master key
    pub fn new(master_key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new(master_key.into());
        Self { cipher }
    }

    /// Derive encryption key from password using Argon2
    pub fn from_password(password: &str, salt: &[u8; 16]) -> Result<Self, CredentialError> {
        let argon2 = Argon2::default();

        let mut key = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| CredentialError::encryption_failed(format!("Key derivation failed: {}", e)))?;

        Ok(Self::new(&key))
    }

    /// Encrypt credential data
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CredentialError> {
        // Generate random nonce (96 bits for GCM)
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = self.cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CredentialError::encryption_failed(format!("Encryption failed: {}", e)))?;

        // Prepend nonce to ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt credential data
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<Vec<u8>, CredentialError> {
        if encrypted.len() < 12 {
            return Err(CredentialError::decryption_failed("Data too short"));
        }

        // Extract nonce
        let (nonce_bytes, ciphertext) = encrypted.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Decrypt
        let plaintext = self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CredentialError::decryption_failed(format!("Decryption failed: {}", e)))?;

        Ok(plaintext)
    }

    /// Encrypt and encode credential
    pub fn encrypt_credential<C: Credential>(&self, credential: &C) -> Result<String, CredentialError> {
        // Serialize
        let json = serde_json::to_vec(credential)
            .map_err(|e| CredentialError::serialization_failed(e.to_string()))?;

        // Encrypt
        let encrypted = self.encrypt(&json)?;

        // Base64 encode
        Ok(base64::encode(&encrypted))
    }

    /// Decrypt and decode credential
    pub fn decrypt_credential<C: Credential>(&self, encrypted_b64: &str) -> Result<C, CredentialError> {
        // Base64 decode
        let encrypted = base64::decode(encrypted_b64)
            .map_err(|e| CredentialError::decryption_failed(format!("Base64 decode failed: {}", e)))?;

        // Decrypt
        let plaintext = self.decrypt(&encrypted)?;

        // Deserialize
        let credential = serde_json::from_slice(&plaintext)
            .map_err(|e| CredentialError::deserialization_failed(e.to_string()))?;

        Ok(credential)
    }
}

// Zeroize sensitive data on drop
impl Drop for CredentialEncryption {
    fn drop(&mut self) {
        // Key is zeroized automatically by aes_gcm crate
    }
}
```

### Key Management with HSM Support

```rust
use pkcs11::Ctx;

pub enum KeyStore {
    InMemory { key: [u8; 32] },
    File { path: PathBuf },
    Hsm { pkcs11_library: String, slot: u64, key_id: Vec<u8> },
    Kms { provider: KmsProvider },
}

pub struct KeyManager {
    store: KeyStore,
}

impl KeyManager {
    pub async fn get_key(&self) -> Result<[u8; 32], CredentialError> {
        match &self.store {
            KeyStore::InMemory { key } => Ok(*key),

            KeyStore::File { path } => {
                let encrypted_key = tokio::fs::read(path).await
                    .map_err(|e| CredentialError::key_not_found(e.to_string()))?;

                // Decrypt using platform keychain
                self.decrypt_file_key(&encrypted_key)
            }

            KeyStore::Hsm { pkcs11_library, slot, key_id } => {
                // Initialize PKCS#11
                let ctx = Ctx::new_and_initialize(pkcs11_library)
                    .map_err(|e| CredentialError::hsm_error(e.to_string()))?;

                // Open session
                let session = ctx.open_session(*slot, pkcs11::CKF_SERIAL_SESSION, None, None)
                    .map_err(|e| CredentialError::hsm_error(e.to_string()))?;

                // Get key from HSM
                self.get_key_from_hsm(&session, key_id)
            }

            KeyStore::Kms { provider } => {
                // Use cloud KMS
                provider.get_data_key().await
            }
        }
    }

    fn get_key_from_hsm(&self, session: &pkcs11::Session, key_id: &[u8]) -> Result<[u8; 32], CredentialError> {
        // Find key object
        let template = vec![
            pkcs11::types::CK_ATTRIBUTE {
                type_: pkcs11::types::CKA_ID,
                pValue: key_id.as_ptr() as *mut _,
                ulValueLen: key_id.len() as u64,
            },
        ];

        let objects = session.find_objects(&template, 1)
            .map_err(|e| CredentialError::hsm_error(e.to_string()))?;

        if objects.is_empty() {
            return Err(CredentialError::key_not_found("Key not found in HSM"));
        }

        // Derive/unwrap key
        // Implementation depends on HSM capabilities
        todo!("HSM key derivation")
    }
}
```

## Testing Credentials

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_oauth2_refresh() {
        let mut cred = OAuth2Credential {
            access_token: "old_token".to_string(),
            refresh_token: "refresh_123".to_string(),
            token_type: "Bearer".to_string(),
            scope: vec!["read".to_string()],
            expires_at: Utc::now() + Duration::hours(1),
            client_id: "client_id".to_string(),
            client_secret: "client_secret".to_string(),
            token_endpoint: "https://oauth.example.com/token".to_string(),
        };

        // Mock HTTP server
        let mock_server = mockito::Server::new();
        let mock = mock_server.mock("POST", "/token")
            .with_status(200)
            .with_body(r#"{
                "access_token": "new_token",
                "token_type": "Bearer",
                "expires_in": 3600
            }"#)
            .create();

        cred.token_endpoint = format!("{}/token", mock_server.url());

        // Refresh
        cred.refresh().await.unwrap();

        assert_eq!(cred.access_token, "new_token");
        mock.assert();
    }

    #[tokio::test]
    async fn test_credential_redaction() {
        let cred = PostgresCredential {
            host: "localhost".to_string(),
            port: 5432,
            database: "mydb".to_string(),
            username: "admin".to_string(),
            password: "super_secret".to_string(),
            ssl_mode: SslMode::Require,
            max_connections: 10,
            min_connections: 1,
            connection_timeout: 30,
        };

        let redacted = cred.redact();

        assert_eq!(redacted.username, "[REDACTED]");
        assert_eq!(redacted.password, "[REDACTED]");
        assert_eq!(redacted.host, "localhost");
    }

    #[test]
    fn test_encryption_roundtrip() {
        let key = [0u8; 32];
        let encryption = CredentialEncryption::new(&key);

        let original = b"sensitive_data";
        let encrypted = encryption.encrypt(original).unwrap();
        let decrypted = encryption.decrypt(&encrypted).unwrap();

        assert_eq!(original, decrypted.as_slice());
        assert_ne!(encrypted, original);
    }
}
```

### Integration Testing with Storage Providers

```rust
#[tokio::test]
async fn test_aws_secrets_manager_integration() {
    let manager = CredentialManager::builder()
        .storage(AwsSecretsManager::new("us-east-1"))
        .build();

    let cred = ApiKeyCredential {
        api_key: "test_key_123".to_string(),
        endpoint: "https://api.example.com".to_string(),
    };

    // Store
    manager.store("test_api_key", cred.clone()).await.unwrap();

    // Retrieve
    let retrieved: ApiKeyCredential = manager.get("test_api_key").await.unwrap();

    assert_eq!(retrieved.api_key, cred.api_key);

    // Clean up
    manager.delete("test_api_key").await.unwrap();
}
```

## Troubleshooting

### Common Issues

**1. Decryption failures**

```
Error: CredentialError::DecryptionFailed("Decryption failed: aead::Error")
```

**Causes**:
- Wrong encryption key
- Corrupted credential data
- Key rotation without re-encrypting

**Solution**:
```rust
// Check key derivation
let salt = /* load salt from config */;
let encryption = CredentialEncryption::from_password(password, &salt)?;

// Re-encrypt all credentials after key rotation
for cred_id in manager.list().await? {
    let cred = manager.get_raw(&cred_id).await?;
    manager.re_encrypt(&cred_id, &cred, &new_encryption).await?;
}
```

**2. OAuth2 token refresh failures**

```
Error: CredentialError::RefreshFailed("Token refresh failed with status: 401")
```

**Solution**:
```rust
impl OAuth2Credential {
    async fn refresh_with_retry(&mut self) -> Result<(), CredentialError> {
        for attempt in 1..=3 {
            match self.refresh().await {
                Ok(_) => return Ok(()),
                Err(e) if attempt < 3 => {
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}
```

**3. Credential not found**

```
Error: CredentialError::NotFound("Credential 'api_key' not found")
```

**Solution**:
```rust
// List all available credentials
let all_creds = manager.list().await?;
println!("Available credentials: {:?}", all_creds);

// Check with exact name
if !manager.exists("api_key").await? {
    println!("Credential does not exist, creating...");
    manager.store("api_key", new_credential).await?;
}
```

## Best Practices

1. **Use HSM for production keys** - Never store master keys in files
2. **Rotate credentials regularly** - Implement automatic rotation schedules
3. **Monitor credential access** - Enable audit logging for compliance
4. **Use scoped credentials** - Limit credential access to specific workflows
5. **Implement expiration** - Set TTL on all credentials
6. **Test rotation** - Verify zero-downtime rotation in staging
7. **Cache wisely** - Balance performance vs security
8. **Redact everywhere** - Mark all sensitive fields with `#[secret]`
9. **Version credentials** - Keep history for rollback
10. **Validate always** - Check credential validity before use

---

**Next Steps**: Explore provider-specific integrations or implement custom credential types.
