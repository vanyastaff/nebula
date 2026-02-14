---
title: CredentialTypes
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# Credential Types Reference

Complete reference for all built-in credential types.

## Overview

|Type|ID|Refresh|Interactive|Use Case|
|---|---|---|---|---|
|[ApiKeyCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#apikeycredential)|`api_key`|❌|❌|REST APIs|
|[OAuth2Credential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#oauth2credential)|`oauth2`|✅|✅|OAuth 2.0 flows|
|[JwtCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#jwtcredential)|`jwt`|✅|❌|JWT tokens|
|[AwsCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#awscredential)|`aws`|✅|❌|AWS services|
|[BasicCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#basiccredential)|`basic`|❌|❌|Basic auth|
|[BearerCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#bearercredential)|`bearer`|✅|❌|Bearer tokens|
|[CertificateCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#certificatecredential)|`certificate`|❌|❌|mTLS|
|[DatabaseCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#databasecredential)|`database`|❌|❌|Databases|
|[SshCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#sshcredential)|`ssh`|❌|❌|SSH connections|
|[CustomCredential](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#customcredential)|`custom`|✅|✅|Custom auth|

## ApiKeyCredential

Simple API key authentication for REST APIs.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct ApiKeyInput {
    /// The API key value
    #[parameter(sensitive = true)]
    pub api_key: String,
    
    /// Header or parameter name (default: "Authorization")
    #[parameter(default = "Authorization")]
    pub param_name: String,
    
    /// How to send the key: "header", "query", "body"
    #[parameter(default = "header")]
    pub auth_method: String,
    
    /// Optional prefix (e.g., "Bearer", "Token")
    pub prefix: Option<String>,
}
```

### State

```rust
pub struct ApiKeyState {
    pub api_key: String,
    pub param_name: String,
    pub auth_method: String,
    pub prefix: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

### Usage Example

```rust
let api_key = manager.create_credential(
    "api_key",
    json!({
        "api_key": "sk_live_abc123",
        "param_name": "X-API-Key",
        "auth_method": "header"
    }),
    &context
).await?;
```

### Client Support

- ✅ HTTP (reqwest, hyper)
- ✅ gRPC (tonic)
- ❌ Database
- ❌ SSH

## OAuth2Credential

Full OAuth 2.0 authentication with automatic token refresh.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct OAuth2Input {
    /// OAuth provider configuration
    pub provider: OAuth2Provider,
    
    /// Client ID
    pub client_id: String,
    
    /// Client secret
    #[parameter(sensitive = true)]
    pub client_secret: Option<String>,
    
    /// Requested scopes
    pub scopes: Vec<String>,
    
    /// Redirect URI for authorization code flow
    pub redirect_uri: String,
    
    /// OAuth flow type
    #[parameter(default = "authorization_code")]
    pub flow_type: OAuth2FlowType,
}

pub enum OAuth2FlowType {
    AuthorizationCode,
    ClientCredentials,
    ResourceOwnerPassword,
    ImplicitGrant,
    DeviceCode,
}
```

### Providers

```rust
pub enum OAuth2Provider {
    Google,
    GitHub,
    Microsoft,
    Facebook,
    Twitter,
    LinkedIn,
    Spotify,
    Discord,
    Slack,
    Custom {
        auth_url: String,
        token_url: String,
        revoke_url: Option<String>,
    },
}
```

### State

```rust
pub struct OAuth2State {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
    pub token_type: String,
}
```

### Usage Example

```rust
// Authorization Code Flow
let oauth = manager.create_credential(
    "oauth2",
    json!({
        "provider": "google",
        "client_id": "your-client-id",
        "client_secret": "your-client-secret",
        "scopes": ["email", "profile"],
        "redirect_uri": "http://localhost:8080/callback",
        "flow_type": "authorization_code"
    }),
    &context
).await?;

// Client Credentials Flow
let oauth = manager.create_credential(
    "oauth2",
    json!({
        "provider": {
            "Custom": {
                "auth_url": "https://auth.example.com/authorize",
                "token_url": "https://auth.example.com/token"
            }
        },
        "client_id": "service-client",
        "client_secret": "service-secret",
        "scopes": ["api.read", "api.write"],
        "flow_type": "client_credentials"
    }),
    &context
).await?;
```

### Interactive Flow

```rust
match manager.initialize_oauth(&oauth_config).await? {
    InitializeResult::RequiresInteraction { auth_url, state } => {
        // Direct user to auth_url
        println!("Please visit: {}", auth_url);
        
        // Wait for callback
        let callback_data = wait_for_callback(state).await?;
        
        // Complete flow
        manager.complete_oauth_flow(callback_data).await?;
    }
    InitializeResult::Ready => {
        // Client credentials flow doesn't need interaction
    }
}
```

## JwtCredential

JSON Web Token authentication with validation and refresh.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct JwtInput {
    /// JWT token
    #[parameter(sensitive = true)]
    pub jwt_token: String,
    
    /// Issuer URL for validation
    pub issuer_url: String,
    
    /// Expected audience
    pub audience: String,
    
    /// Algorithm for signature verification
    #[parameter(default = "RS256")]
    pub algorithm: String,
    
    /// Public key for verification (if not using JWKS)
    pub public_key: Option<String>,
    
    /// JWKS endpoint
    pub jwks_url: Option<String>,
}
```

### State

```rust
pub struct JwtState {
    pub jwt_token: String,
    pub expires_at: DateTime<Utc>,
    pub claims: HashMap<String, Value>,
    pub refresh_token: Option<String>,
}
```

### Usage Example

```rust
let jwt = manager.create_credential(
    "jwt",
    json!({
        "jwt_token": "eyJhbGciOiJSUzI1NiIs...",
        "issuer_url": "https://auth.example.com",
        "audience": "api.example.com",
        "jwks_url": "https://auth.example.com/.well-known/jwks.json"
    }),
    &context
).await?;
```

## AwsCredential

AWS credentials with STS support and role assumption.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct AwsInput {
    /// AWS access key ID
    pub access_key_id: String,
    
    /// AWS secret access key
    #[parameter(sensitive = true)]
    pub secret_access_key: String,
    
    /// Session token for temporary credentials
    #[parameter(sensitive = true)]
    pub session_token: Option<String>,
    
    /// AWS region
    #[parameter(default = "us-east-1")]
    pub region: String,
    
    /// Role ARN to assume
    pub role_arn: Option<String>,
    
    /// External ID for role assumption
    pub external_id: Option<String>,
    
    /// Session duration in seconds
    #[parameter(default = 3600)]
    pub duration_seconds: i32,
}
```

### State

```rust
pub struct AwsState {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
    pub expires_at: Option<DateTime<Utc>>,
}
```

### Usage Example

```rust
// Direct credentials
let aws = manager.create_credential(
    "aws",
    json!({
        "access_key_id": "AKIAIOSFODNN7EXAMPLE",
        "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "region": "us-west-2"
    }),
    &context
).await?;

// With role assumption
let aws = manager.create_credential(
    "aws",
    json!({
        "access_key_id": "AKIAIOSFODNN7EXAMPLE",
        "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "role_arn": "arn:aws:iam::123456789012:role/MyRole",
        "external_id": "unique-external-id",
        "duration_seconds": 3600
    }),
    &context
).await?;
```

### Client Support

- ✅ AWS SDK
- ✅ S3
- ✅ DynamoDB
- ✅ Lambda
- ✅ STS

## BasicCredential

HTTP Basic authentication.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct BasicInput {
    /// Username
    pub username: String,
    
    /// Password
    #[parameter(sensitive = true)]
    pub password: String,
}
```

### State

```rust
pub struct BasicState {
    pub username: String,
    pub password: String,
    pub created_at: DateTime<Utc>,
}
```

### Usage Example

```rust
let basic = manager.create_credential(
    "basic",
    json!({
        "username": "admin",
        "password": "secure_password"
    }),
    &context
).await?;
```

## BearerCredential

Bearer token authentication.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct BearerInput {
    /// Bearer token
    #[parameter(sensitive = true)]
    pub token: String,
    
    /// Token type (default: "Bearer")
    #[parameter(default = "Bearer")]
    pub token_type: String,
    
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    
    /// Refresh token
    #[parameter(sensitive = true)]
    pub refresh_token: Option<String>,
    
    /// Refresh endpoint
    pub refresh_url: Option<String>,
}
```

### State

```rust
pub struct BearerState {
    pub token: String,
    pub token_type: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<String>,
    pub refresh_url: Option<String>,
}
```

## CertificateCredential

Client certificate authentication for mTLS.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct CertificateInput {
    /// Client certificate (PEM or DER)
    #[parameter(sensitive = true)]
    pub certificate: Vec<u8>,
    
    /// Private key
    #[parameter(sensitive = true)]
    pub private_key: Vec<u8>,
    
    /// CA bundle for verification
    pub ca_bundle: Option<Vec<u8>>,
    
    /// Passphrase for encrypted private key
    #[parameter(sensitive = true)]
    pub passphrase: Option<String>,
    
    /// Certificate format: "pem" or "der"
    #[parameter(default = "pem")]
    pub format: String,
}
```

### Usage Example

```rust
let cert = manager.create_credential(
    "certificate",
    json!({
        "certificate": read_file("client.crt")?,
        "private_key": read_file("client.key")?,
        "ca_bundle": read_file("ca-bundle.crt")?,
        "format": "pem"
    }),
    &context
).await?;
```

## DatabaseCredential

Database connection credentials.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct DatabaseInput {
    /// Database type
    pub database_type: DatabaseType,
    
    /// Host
    pub host: String,
    
    /// Port
    pub port: u16,
    
    /// Database name
    pub database: String,
    
    /// Username
    pub username: String,
    
    /// Password
    #[parameter(sensitive = true)]
    pub password: String,
    
    /// SSL mode
    #[parameter(default = "prefer")]
    pub ssl_mode: String,
    
    /// Additional connection parameters
    pub params: HashMap<String, String>,
}

pub enum DatabaseType {
    PostgreSQL,
    MySQL,
    MariaDB,
    SQLite,
    MongoDB,
    Redis,
    Cassandra,
    Elasticsearch,
}
```

### State

```rust
pub struct DatabaseState {
    pub connection_string: String,
    pub database_type: DatabaseType,
    pub created_at: DateTime<Utc>,
}
```

### Usage Example

```rust
let db = manager.create_credential(
    "database",
    json!({
        "database_type": "postgresql",
        "host": "localhost",
        "port": 5432,
        "database": "myapp",
        "username": "dbuser",
        "password": "dbpass",
        "ssl_mode": "require",
        "params": {
            "application_name": "nebula",
            "connect_timeout": "10"
        }
    }),
    &context
).await?;
```

## SshCredential

SSH authentication credentials.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct SshInput {
    /// SSH host
    pub host: String,
    
    /// SSH port
    #[parameter(default = 22)]
    pub port: u16,
    
    /// Username
    pub username: String,
    
    /// Authentication method: "password" or "key"
    pub auth_method: String,
    
    /// Password (for password auth)
    #[parameter(sensitive = true)]
    pub password: Option<String>,
    
    /// Private key (for key auth)
    #[parameter(sensitive = true)]
    pub private_key: Option<Vec<u8>>,
    
    /// Passphrase for private key
    #[parameter(sensitive = true)]
    pub passphrase: Option<String>,
    
    /// Known hosts file content
    pub known_hosts: Option<String>,
}
```

## CustomCredential

Flexible credential type for custom authentication schemes.

### Input Parameters

```rust
#[derive(Parameters)]
pub struct CustomInput {
    /// Credential type identifier
    pub credential_type: String,
    
    /// Authentication data (flexible JSON)
    #[parameter(sensitive = true)]
    pub auth_data: Value,
    
    /// Headers to add to requests
    pub headers: HashMap<String, String>,
    
    /// Query parameters to add
    pub query_params: HashMap<String, String>,
    
    /// Custom metadata
    pub metadata: HashMap<String, Value>,
}
```

### Usage Example

```rust
let custom = manager.create_credential(
    "custom",
    json!({
        "credential_type": "proprietary_auth",
        "auth_data": {
            "api_id": "12345",
            "api_secret": "secret_key",
            "account_id": "acc_789"
        },
        "headers": {
            "X-Custom-Auth": "enabled",
            "X-Account-ID": "acc_789"
        },
        "metadata": {
            "environment": "production",
            "version": "v2"
        }
    }),
    &context
).await?;
```

## Type Selection Guide

### Decision Tree

```mermaid
graph TD
    A[What type of auth?] --> B{Token-based?}
    B -->|Yes| C{What kind?}
    B -->|No| D{Certificate?}
    C -->|API Key| E[ApiKeyCredential]
    C -->|OAuth2| F[OAuth2Credential]
    C -->|JWT| G[JwtCredential]
    C -->|Bearer| H[BearerCredential]
    D -->|Yes| I[CertificateCredential]
    D -->|No| J{Username/Password?}
    J -->|Yes| K[BasicCredential]
    J -->|No| L{Cloud Provider?}
    L -->|AWS| M[AwsCredential]
    L -->|Other| N[CustomCredential]
```

### Comparison Matrix

|Feature|API Key|OAuth2|JWT|AWS|Basic|Certificate|
|---|---|---|---|---|---|---|
|Complexity|Low|High|Medium|Medium|Low|High|
|Security|Medium|High|High|High|Low|Very High|
|Refresh Support|❌|✅|✅|✅|❌|❌|
|User Interaction|❌|✅|❌|❌|❌|❌|
|Standard|Various|RFC 6749|RFC 7519|AWS|RFC 7617|X.509|

## Related

- [Credential Trait](https://claude.ai/chat/CredentialTrait.md)
- [Credential Manager](https://claude.ai/chat/CredentialManager.md)
- [Storage Backends](https://claude.ai/chat/StorageBackends.md)