# Nebula Resource - Credential Integration Design

## Overview

This document outlines how `nebula-credential` integrates with `nebula-resource` to provide secure, automatic credential management for resources that require authentication.

## Current State

✅ **Already Implemented:**
- `ResourceCredentialProvider` - Token caching and refresh
- `CredentialRotationHandler` - Automatic credential rotation
- `CredentialRotationScheduler` - Background rotation scheduling
- `CredentialConfig` in resource configs (Postgres, etc.)
- Connection string placeholder replacement

## Proposed Enhancements

### 1. **Resource-Level Authenticator Integration**

Add authenticator support to resources, combining:
- `nebula-credential` for token management
- `ClientAuthenticator` for creating authenticated clients
- `nebula-resource` for lifecycle management

```rust
// Example: HTTP Client with automatic authentication
use nebula_resource::prelude::*;
use nebula_credential::authenticator::{HttpBearer, AuthenticateWith};

let http_client = manager.get::<HttpClient>("api_client").await?;
let cred_provider = manager.get_credential_provider("api_client").await?;

// Get token and authenticate request
let token = cred_provider.get_token().await?;
let request = http_client.get("https://api.example.com/data");
let auth_request = request.authenticate_with(&HttpBearer, &token).await?;

let response = auth_request.send().await?;
```

### 2. **Authenticator-Aware Resource Trait**

```rust
/// Extension trait for resources that support authentication
#[async_trait]
pub trait AuthenticatedResource: Resource {
    /// The client type this resource provides
    type Client: Send + Sync;

    /// Get authenticated client
    async fn get_authenticated_client(
        &self,
        credential_provider: &ResourceCredentialProvider,
    ) -> ResourceResult<Self::Client>;
}
```

### 3. **Resource-Specific Authenticators**

Each resource type can define its own authenticator:

```rust
// PostgreSQL Authenticator
pub struct PostgresAuthenticator;

#[async_trait]
impl ClientAuthenticator for PostgresAuthenticator {
    type Target = PgConnectOptions;
    type Output = PgPool;

    async fn authenticate(
        &self,
        options: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        // Extract username/password from token
        let (username, password) = parse_postgres_credentials(token)?;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(
                options
                    .username(&username)
                    .password(&password)
            )
            .await?;

        Ok(pool)
    }
}

// MongoDB Authenticator
pub struct MongoAuthenticator;

#[async_trait]
impl ClientAuthenticator for MongoAuthenticator {
    type Target = String; // Base connection string
    type Output = mongodb::Client;

    async fn authenticate(
        &self,
        base_url: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        let auth_url = build_mongo_connection_string(&base_url, token)?;
        let client = mongodb::Client::with_uri_str(&auth_url).await?;
        Ok(client)
    }
}

// Redis Authenticator
pub struct RedisAuthenticator;

#[async_trait]
impl ClientAuthenticator for RedisAuthenticator {
    type Target = String; // Redis URL
    type Output = redis::aio::MultiplexedConnection;

    async fn authenticate(
        &self,
        url: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        let password = token.token.with_exposed(|s| s.to_string());
        let client = redis::Client::open(url)?;
        let mut conn = client.get_multiplexed_async_connection().await?;

        // Authenticate if password is provided
        if !password.is_empty() {
            redis::cmd("AUTH").arg(&password).query_async(&mut conn).await?;
        }

        Ok(conn)
    }
}

// HTTP Client Authenticator (with headers)
pub struct HttpClientAuthenticator {
    pub auth_type: HttpAuthType,
}

pub enum HttpAuthType {
    Bearer,
    ApiKey { header_name: String },
    Basic,
}

#[async_trait]
impl ClientAuthenticator for HttpClientAuthenticator {
    type Target = reqwest::Client;
    type Output = AuthenticatedHttpClient;

    async fn authenticate(
        &self,
        client: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        Ok(AuthenticatedHttpClient {
            client,
            token: token.clone(),
            auth_type: self.auth_type.clone(),
        })
    }
}

/// Wrapper that auto-adds auth to requests
pub struct AuthenticatedHttpClient {
    client: reqwest::Client,
    token: AccessToken,
    auth_type: HttpAuthType,
}

impl AuthenticatedHttpClient {
    pub fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut request = self.client.get(url);
        request = self.add_auth_header(request);
        request
    }

    fn add_auth_header(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_type {
            HttpAuthType::Bearer => {
                let value = self.token.token.with_exposed(|s| format!("Bearer {s}"));
                request.header("Authorization", value)
            }
            HttpAuthType::ApiKey { header_name } => {
                let value = self.token.token.with_exposed(ToString::to_string);
                request.header(header_name, value)
            }
            HttpAuthType::Basic => {
                // Implement Basic auth
                request
            }
        }
    }
}
```

### 4. **Resource Config Enhancement**

```rust
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub base_url: Option<String>,
    pub timeout: Duration,

    // NEW: Credential and authenticator configuration
    #[cfg(feature = "credentials")]
    pub credential: Option<CredentialResourceConfig>,
}

#[derive(Debug, Clone)]
pub struct CredentialResourceConfig {
    /// Credential ID to use
    pub credential_id: String,

    /// Authenticator type
    pub auth_type: AuthenticatorType,

    /// Auto-refresh settings
    pub auto_refresh: bool,
    pub refresh_threshold_minutes: i64,
}

#[derive(Debug, Clone)]
pub enum AuthenticatorType {
    HttpBearer,
    HttpApiKey { header_name: String },
    HttpBasic,
    Custom(String), // Custom authenticator name
}
```

### 5. **Resource Manager Integration**

```rust
impl ResourceManager {
    /// Get resource with automatic credential provider
    pub async fn get_with_credentials<R>(
        &self,
        resource_id: &str,
    ) -> ResourceResult<(Arc<R>, Arc<ResourceCredentialProvider>)>
    where
        R: Resource + 'static,
    {
        let resource = self.get::<R>(resource_id).await?;

        // Get credential provider if configured
        let provider = self.get_credential_provider(resource_id).await?;

        Ok((resource, provider))
    }

    /// Get authenticated client directly
    pub async fn get_authenticated<R>(
        &self,
        resource_id: &str,
    ) -> ResourceResult<R::Client>
    where
        R: AuthenticatedResource + 'static,
    {
        let (resource, provider) = self.get_with_credentials::<R>(resource_id).await?;
        resource.get_authenticated_client(&provider).await
    }
}
```

### 6. **Usage Examples**

#### Example 1: HTTP Client with Bearer Auth

```rust
use nebula_resource::prelude::*;
use nebula_credential::CredentialManager;

// Setup
let cred_manager = Arc::new(CredentialManager::builder()
    .with_store(postgres_store)
    .with_cache(redis_cache)
    .build()?);

let resource_manager = ResourceManager::new();

// Register HTTP client resource with credentials
let http_config = HttpClientConfig {
    base_url: Some("https://api.example.com".to_string()),
    credential: Some(CredentialResourceConfig {
        credential_id: "api_credentials".to_string(),
        auth_type: AuthenticatorType::HttpBearer,
        auto_refresh: true,
        refresh_threshold_minutes: 5,
    }),
    ..Default::default()
};

resource_manager.register::<HttpClient>("api_client", http_config).await?;

// Use authenticated client
let client = resource_manager.get_authenticated::<HttpClient>("api_client").await?;
let response = client.get("/users").send().await?;
```

#### Example 2: PostgreSQL with Rotating Credentials

```rust
// Create credential for PostgreSQL
let pg_cred_id = cred_manager.create_credential(
    "postgres_password",
    json!({
        "username": "app_user",
        "password": "initial_password"
    })
).await?;

// Configure PostgreSQL resource with credential
let pg_config = PostgresConfig {
    url: "postgresql://localhost/mydb".to_string(),
    credential: Some(CredentialResourceConfig {
        credential_id: pg_cred_id.to_string(),
        auth_type: AuthenticatorType::Custom("postgres".to_string()),
        auto_refresh: true,
        refresh_threshold_minutes: 10,
    }),
    ..Default::default()
};

// Resource automatically handles credential rotation
resource_manager.register::<Postgres>("main_db", pg_config).await?;

// Get pool - credentials are automatically applied
let pool = resource_manager.get_authenticated::<Postgres>("main_db").await?;
let row = sqlx::query("SELECT 1").fetch_one(&pool).await?;
```

#### Example 3: Multi-Resource Credential Sharing

```rust
// One credential used by multiple resources
let api_cred_id = cred_manager.create_credential(
    "api_key",
    json!({
        "api_key": "sk-1234567890"
    })
).await?;

// HTTP Client
let http_config = HttpClientConfig {
    credential: Some(CredentialResourceConfig {
        credential_id: api_cred_id.to_string(),
        auth_type: AuthenticatorType::HttpBearer,
        ..Default::default()
    }),
    ..Default::default()
};

// WebSocket Client (hypothetical)
let ws_config = WebSocketConfig {
    credential: Some(CredentialResourceConfig {
        credential_id: api_cred_id.to_string(), // Same credential!
        auth_type: AuthenticatorType::HttpBearer,
        ..Default::default()
    }),
    ..Default::default()
};

// Both resources share the same credential, automatic refresh applies to both
resource_manager.register::<HttpClient>("http", http_config).await?;
resource_manager.register::<WebSocketClient>("ws", ws_config).await?;
```

#### Example 4: Custom Authenticator

```rust
// Define custom authenticator for your API
struct MyApiAuthenticator {
    api_version: String,
}

#[async_trait]
impl ClientAuthenticator for MyApiAuthenticator {
    type Target = reqwest::Client;
    type Output = MyAuthenticatedClient;

    async fn authenticate(
        &self,
        client: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        // Custom authentication logic
        let api_key = token.token.with_exposed(ToString::to_string);
        let session_id = call_auth_endpoint(&api_key).await?;

        Ok(MyAuthenticatedClient {
            client,
            session_id,
            api_version: self.api_version.clone(),
        })
    }
}

// Use in resource
impl AuthenticatedResource for MyApiResource {
    type Client = MyAuthenticatedClient;

    async fn get_authenticated_client(
        &self,
        provider: &ResourceCredentialProvider,
    ) -> ResourceResult<Self::Client> {
        let token = provider.get_token().await?;
        let authenticator = MyApiAuthenticator {
            api_version: "v2".to_string(),
        };

        self.client
            .clone()
            .authenticate_with(&authenticator, &token)
            .await
            .map_err(|e| ResourceError::internal("auth", e.to_string()))
    }
}
```

## Implementation Phases

### Phase 1: Core Integration ✅ (Already Done)
- [x] `ResourceCredentialProvider`
- [x] Connection string placeholder replacement
- [x] Credential rotation scheduler

### Phase 2: Authenticator Support (Proposed)
- [ ] Add `AuthenticatedResource` trait
- [ ] Implement resource-specific authenticators
- [ ] Enhance `ResourceManager` with credential methods
- [ ] Add examples for each resource type

### Phase 3: Advanced Features (Future)
- [ ] Credential caching at resource level
- [ ] Automatic credential rotation on connection failures
- [ ] Metrics for credential usage
- [ ] Audit logging for credential access
- [ ] Support for credential versioning

## Benefits

1. **Separation of Concerns**
   - Resource management: `nebula-resource`
   - Credential management: `nebula-credential`
   - Authentication logic: `ClientAuthenticator`

2. **Automatic Token Refresh**
   - Resources don't need to handle token expiration
   - Transparent to application code

3. **Type-Safe Authentication**
   - Compile-time verification
   - Each resource type has its own authenticator

4. **Reusable Credentials**
   - One credential can be used by multiple resources
   - Centralized credential rotation

5. **Testability**
   - Easy to mock authenticators
   - Mock credential providers for testing

6. **Flexibility**
   - Custom authenticators for specific needs
   - Chain authenticators for complex auth flows

## Security Considerations

1. **No Plaintext Storage**: Credentials stored in `SecureString` with zeroization
2. **Minimal Exposure**: Tokens only exposed during authentication
3. **Automatic Rotation**: Reduces risk of credential compromise
4. **Audit Trail**: All credential access logged
5. **Scoped Access**: Resources only access their assigned credentials

## Migration Path

For existing code:

```rust
// Before (manual credential handling)
let password = env::var("DB_PASSWORD")?;
let url = format!("postgresql://user:{}@localhost/db", password);
let pool = PgPoolOptions::new().connect(&url).await?;

// After (nebula-credential integration)
let pool = resource_manager
    .get_authenticated::<Postgres>("main_db")
    .await?;
// Credentials automatically managed, rotated, and secured
```

## Next Steps

1. Implement `AuthenticatedResource` trait
2. Create authenticators for common resources:
   - PostgreSQL
   - MongoDB
   - Redis
   - HTTP Client
3. Add examples demonstrating integration
4. Write integration tests
5. Document patterns for custom resources

## Questions to Consider

1. Should authenticators be registered globally or per-resource?
2. How to handle authenticator configuration in resource config?
3. Should we support authenticator chaining at resource level?
4. How to handle credentials that need interactive flows (OAuth)?
5. Should resources auto-reconnect on credential rotation?
