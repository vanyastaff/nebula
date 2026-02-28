# nebula-credential

Comprehensive credential management system for Nebula. Provides secure, type-safe credential handling with automatic refresh, multi-factor authentication, and tier-specific optimizations.

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Core Concepts](#core-concepts)
4. [Credential Types](#credential-types)
5. [Usage Examples](#usage-examples)
6. [Integration with Actions](#integration-with-actions)
7. [Security Features](#security-features)
8. [Testing](#testing)
9. [Best Practices](#best-practices)

## Overview

nebula-credential provides:
- **Unified credential management** across all authentication types
- **Automatic token refresh** and lifecycle management
- **Type-safe credential access** with compile-time guarantees
- **Tier-specific optimizations** for different deployment scenarios
- **Built-in security features** including encryption and audit logging

## Architecture

### File Structure

```
nebula-credential/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Main exports and prelude
│   │
│   ├── core/                     # Core traits and types
│   │   ├── mod.rs
│   │   ├── credential.rs         # Credential trait
│   │   ├── token.rs              # Token types
│   │   ├── error.rs              # Error types
│   │   ├── metadata.rs           # Credential metadata
│   │   └── context.rs            # CredentialContext
│   │
│   ├── manager/                  # Credential manager
│   │   ├── mod.rs
│   │   ├── manager.rs            # Main CredentialManager
│   │   ├── registry.rs           # Type registry
│   │   ├── cache.rs              # Token caching
│   │   ├── refresh.rs            # Auto-refresh logic
│   │   └── lifecycle.rs          # Lifecycle management
│   │
│   ├── storage/                  # Storage backends
│   │   ├── mod.rs
│   │   ├── traits.rs             # Storage traits
│   │   ├── memory.rs             # In-memory storage
│   │   ├── file.rs               # File-based storage
│   │   ├── database.rs           # Database storage
│   │   └── vault.rs              # HashiCorp Vault
│   │
│   ├── types/                    # Built-in credential types
│   │   ├── mod.rs
│   │   ├── api_key.rs            # API key credentials
│   │   ├── oauth2.rs             # OAuth 2.0 flow
│   │   ├── jwt.rs                # JWT tokens
│   │   ├── basic.rs              # Basic auth
│   │   ├── aws.rs                # AWS credentials
│   │   ├── certificate.rs        # Client certificates
│   │   └── custom.rs             # Custom credentials
│   │
│   ├── security/                 # Security features
│   │   ├── mod.rs
│   │   ├── encryption.rs         # At-rest encryption
│   │   ├── audit.rs              # Audit logging
│   │   ├── validation.rs         # Input validation
│   │   └── sanitization.rs       # Data sanitization
│   │
│   ├── client/                   # Client authentication
│   │   ├── mod.rs
│   │   ├── authenticator.rs      # ClientAuthenticator trait
│   │   ├── http.rs               # HTTP client auth
│   │   ├── grpc.rs               # gRPC client auth
│   │   └── database.rs           # Database client auth
│   │
│   ├── interactive/              # Interactive flows
│   │   ├── mod.rs
│   │   ├── flow.rs               # Flow management
│   │   ├── browser.rs            # Browser-based auth
│   │   ├── device.rs             # Device flow
│   │   └── cli.rs                # CLI prompts
│   │
│   ├── mfa/                      # Multi-factor auth
│   │   ├── mod.rs
│   │   ├── totp.rs               # TOTP/Google Authenticator
│   │   ├── sms.rs                # SMS verification
│   │   ├── email.rs              # Email verification
│   │   └── biometric.rs          # Biometric auth
│   │
│   ├── rotation/                 # Credential rotation
│   │   ├── mod.rs
│   │   ├── rotator.rs            # Rotation engine
│   │   ├── policy.rs             # Rotation policies
│   │   └── scheduler.rs          # Rotation scheduling
│   │
│   └── prelude.rs                # Common imports
│
├── examples/
│   ├── basic_api_key.rs          # Simple API key usage
│   ├── oauth_flow.rs             # OAuth 2.0 flow
│   ├── mfa_setup.rs              # Multi-factor setup
│   ├── rotation.rs               # Credential rotation
│   └── custom_credential.rs       # Custom credential type
│
└── tests/
    ├── integration/
    └── unit/
```

## Core Concepts

### Credential Trait

The foundation of the credential system:

```rust
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Input parameters for this credential type
    type Input: DeserializeOwned + Serialize + Send + Sync;
    
    /// Persistent state for this credential
    type State: CredentialState;
    
    /// Metadata about this credential type
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: std::any::type_name::<Self>(),
            name: "Unknown Credential",
            description: "No description",
            supports_refresh: false,
            requires_interaction: false,
            supported_clients: vec![],
        }
    }
    
    /// Initialize the credential
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError>;
    
    /// Get current token
    async fn get_token(
        &self,
        state: &Self::State,
        context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError>;
    
    /// Refresh token if supported
    async fn refresh_token(
        &self,
        state: &mut Self::State,
        context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        let input = context.get_input::<OAuth2Input>()?;
        let refresh_token = state.refresh_token.as_ref()
            .ok_or(CredentialError::NoRefreshToken)?;
        
        // Create OAuth client
        let client = self.create_oauth_client(&input)?;
        
        // Exchange refresh token
        let token_response = client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.clone()))
            .request_async(async_http_client)
            .await?;
        
        // Update state
        state.access_token = token_response.access_token().secret().clone();
        state.expires_at = Utc::now() + Duration::seconds(
            token_response.expires_in()
                .map(|d| d.as_secs() as i64)
                .unwrap_or(3600)
        );
        
        if let Some(new_refresh) = token_response.refresh_token() {
            state.refresh_token = Some(new_refresh.secret().clone());
        }
        
        Ok(TokenResult::Token(Token {
            value: SecureString::new(&state.access_token),
            token_type: TokenType::Bearer,
            expires_at: Some(state.expires_at),
            scopes: state.scopes.clone(),
            claims: HashMap::new(),
        }))
    }
}

// Handle OAuth callback
impl OAuth2Credential {
    pub async fn handle_callback(
        &self,
        callback_data: CallbackData,
        context: &mut CredentialContext,
    ) -> Result<OAuth2State, CredentialError> {
        // Verify CSRF token
        let stored_csrf = context.get_temp_data::<String>("csrf_token")?;
        if callback_data.state != stored_csrf {
            return Err(CredentialError::InvalidState);
        }
        
        let input = context.get_input::<OAuth2Input>()?;
        let client = self.create_oauth_client(&input)?;
        
        // Exchange authorization code
        let code = AuthorizationCode::new(callback_data.code);
        let token_response = client
            .exchange_code(code)
            .request_async(async_http_client)
            .await?;
        
        Ok(OAuth2State {
            access_token: token_response.access_token().secret().clone(),
            refresh_token: token_response.refresh_token()
                .map(|t| t.secret().clone()),
            expires_at: Utc::now() + Duration::seconds(
                token_response.expires_in()
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(3600)
            ),
            scopes: token_response.scopes()
                .map(|scopes| scopes.iter().map(|s| s.to_string()).collect())
                .unwrap_or_else(|| input.scopes.clone()),
        })
    }
}

### 3. AWS Credentials

```rust
#[derive(Credential)]
#[credential(
    id = "aws",
    name = "AWS Credentials",
    description = "AWS access key and secret with optional session token"
)]
pub struct AwsCredential;

#[derive(Parameters)]
pub struct AwsInput {
    #[parameter(description = "AWS access key ID")]
    pub access_key_id: String,
    
    #[parameter(description = "AWS secret access key", sensitive = true)]
    pub secret_access_key: String,
    
    #[parameter(description = "AWS session token (for temporary credentials)", sensitive = true)]
    pub session_token: Option<String>,
    
    #[parameter(description = "AWS region", default = "us-east-1")]
    pub region: String,
    
    #[parameter(description = "Role ARN to assume")]
    pub role_arn: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct AwsState {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CredentialState for AwsState {
    fn is_valid(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() < expires_at
        } else {
            true // Permanent credentials
        }
    }
    
    fn needs_refresh(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() + Duration::minutes(10) > expires_at
        } else {
            false
        }
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

#[async_trait]
impl Credential for AwsCredential {
    type Input = AwsInput;
    type State = AwsState;
    
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError> {
        let state = if let Some(role_arn) = &input.role_arn {
            // Assume role
            self.assume_role(input, role_arn).await?
        } else {
            // Direct credentials
            AwsState {
                access_key_id: input.access_key_id.clone(),
                secret_access_key: input.secret_access_key.clone(),
                session_token: input.session_token.clone(),
                region: input.region.clone(),
                expires_at: None,
            }
        };
        
        context.save_state(&state).await?;
        Ok(InitializeResult::Ready)
    }
    
    async fn get_token(
        &self,
        state: &Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        if !state.is_valid() {
            return Ok(TokenResult::Expired);
        }
        
        // AWS doesn't use traditional tokens, but we encode credentials
        let token_value = format!(
            "{}:{}:{}",
            state.access_key_id,
            state.secret_access_key,
            state.session_token.as_deref().unwrap_or("")
        );
        
        Ok(TokenResult::Token(Token {
            value: SecureString::new(token_value),
            token_type: TokenType::AWS,
            expires_at: state.expires_at,
            scopes: vec![],
            claims: hashmap! {
                "region".to_string() => Value::String(state.region.clone()),
            },
        }))
    }
}

// Client authenticator for AWS SDK
#[async_trait]
impl ClientAuthenticator<aws_sdk_s3::Client> for AwsCredential {
    async fn create_authenticated_client(
        &self,
        context: &CredentialContext,
    ) -> Result<aws_sdk_s3::Client, CredentialError> {
        let state = context.get_state::<AwsState>()?;
        
        let credentials = aws_credential_types::Credentials::new(
            &state.access_key_id,
            &state.secret_access_key,
            state.session_token.clone(),
            state.expires_at.map(|dt| dt.into()),
            "nebula-credential",
        );
        
        let config = aws_config::from_env()
            .region(aws_config::Region::new(state.region.clone()))
            .credentials_provider(credentials)
            .load()
            .await;
        
        Ok(aws_sdk_s3::Client::new(&config))
    }
}
```

### 4. Custom Credential Type

```rust
// Example: Custom JWT credential
#[derive(Debug)]
pub struct JwtCredential;

#[derive(Parameters)]
pub struct JwtInput {
    #[parameter(description = "JWT token", sensitive = true)]
    pub jwt_token: String,
    
    #[parameter(description = "JWT issuer URL")]
    pub issuer_url: String,
    
    #[parameter(description = "Expected audience")]
    pub audience: String,
    
    #[parameter(description = "Refresh endpoint")]
    pub refresh_endpoint: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct JwtState {
    pub jwt_token: String,
    pub issuer_url: String,
    pub audience: String,
    pub refresh_endpoint: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub claims: HashMap<String, Value>,
}

impl CredentialState for JwtState {
    fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at
    }
    
    fn needs_refresh(&self) -> bool {
        Utc::now() + Duration::minutes(5) > self.expires_at
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        Some(self.expires_at)
    }
}

#[async_trait]
impl Credential for JwtCredential {
    type Input = JwtInput;
    type State = JwtState;
    
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "jwt",
            name: "JWT Token",
            description: "JSON Web Token with validation",
            supports_refresh: true,
            requires_interaction: false,
            supported_clients: vec!["http"],
        }
    }
    
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError> {
        // Validate JWT
        let (claims, expires_at) = self.validate_jwt(&input.jwt_token, &input.issuer_url).await?;
        
        // Check audience
        if claims.get("aud").and_then(|v| v.as_str()) != Some(&input.audience) {
            return Err(CredentialError::ValidationFailed("Invalid audience".into()));
        }
        
        let state = JwtState {
            jwt_token: input.jwt_token.clone(),
            issuer_url: input.issuer_url.clone(),
            audience: input.audience.clone(),
            refresh_endpoint: input.refresh_endpoint.clone(),
            expires_at,
            claims,
        };
        
        context.save_state(&state).await?;
        Ok(InitializeResult::Ready)
    }
    
    async fn get_token(
        &self,
        state: &Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        if state.is_valid() {
            Ok(TokenResult::Token(Token {
                value: SecureString::new(&state.jwt_token),
                token_type: TokenType::JWT,
                expires_at: Some(state.expires_at),
                scopes: state.claims.get("scope")
                    .and_then(|v| v.as_str())
                    .map(|s| s.split(' ').map(String::from).collect())
                    .unwrap_or_default(),
                claims: state.claims.clone(),
            }))
        } else if state.needs_refresh() && state.refresh_endpoint.is_some() {
            Ok(TokenResult::NeedsRefresh)
        } else {
            Ok(TokenResult::Expired)
        }
    }
}
```

## Usage Examples

### Basic Usage

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create credential manager
    let manager = CredentialManager::builder()
        .with_storage_backend(StorageBackend::Encrypted {
            path: "credentials.db",
            password: "secure-password",
        })
        .with_auto_refresh(true)
        .build()
        .await?;
    
    // Register credential types
    manager.register_credential_type(ApiKeyCredential).await?;
    manager.register_credential_type(OAuth2Credential).await?;
    
    // Create API key credential
    let api_key_id = manager.create_credential(
        "api_key",
        json!({
            "api_key": "sk-1234567890",
            "auth_method": "header",
            "param_name": "X-API-Key",
        }),
        &UserContext::new("user123", "192.168.1.1"),
    ).await?;
    
    // Use the credential
    let token = manager.get_token(&api_key_id).await?;
    println!("Token: {}", token.value.expose());
    
    Ok(())
}
```

### OAuth 2.0 Flow

```rust
// Initialize OAuth credential
let oauth_input = OAuth2Input {
    provider: OAuth2Provider::google(),
    client_id: "your-client-id".to_string(),
    client_secret: Some("your-client-secret".to_string()),
    scopes: vec!["openid".to_string(), "profile".to_string()],
    redirect_uri: "http://localhost:8080/callback".to_string(),
};

match manager.create_credential("oauth2", serde_json::to_value(oauth_input)?, &user_context).await {
    Ok(credential_id) => {
        // Credential created without interaction
        println!("Credential ID: {}", credential_id);
    }
    Err(CredentialError::InteractionRequired { flow_id }) => {
        // Need user interaction
        let flow = manager.get_interaction_flow(&flow_id).await?;
        
        match flow.interaction_type {
            InteractionType::BrowserAuth { auth_url, callback_url } => {
                println!("Please visit: {}", auth_url);
                println!("Callback will be at: {}", callback_url);
                
                // In real app, open browser and wait for callback
                // Here we simulate the callback
                let callback_data = wait_for_oauth_callback().await?;
                
                let credential_id = manager.complete_flow(&flow_id, callback_data).await?;
                println!("OAuth credential created: {}", credential_id);
            }
            _ => unreachable!(),
        }
    }
    Err(e) => return Err(e.into()),
}
```

### Multi-Factor Authentication

```rust
// Create MFA credential combining API key + TOTP
let mfa_input = MfaInput {
    primary_factor: CredentialConfig {
        credential_type: "api_key".to_string(),
        input: json!({
            "api_key": "primary-key",
            "auth_method": "header",
            "param_name": "X-API-Key",
        }),
    },
    secondary_factors: vec![
        CredentialConfig {
            credential_type: "totp".to_string(),
            input: json!({
                "secret": "JBSWY3DPEHPK3PXP",
                "issuer": "MyApp",
                "account": "user@example.com",
            }),
        },
    ],
    require_all_secondary: true,
};

let mfa_id = manager.create_credential(
    "mfa",
    serde_json::to_value(mfa_input)?,
    &user_context,
).await?;

// Using MFA credential
match manager.get_token(&mfa_id).await {
    Ok(token) => {
        println!("MFA authentication successful!");
    }
    Err(CredentialError::MfaRequired { challenge }) => {
        // Need TOTP code
        println!("Enter TOTP code:");
        let totp_code = read_line()?;
        
        let token = manager.complete_mfa_challenge(&mfa_id, &challenge, json!({
            "totp_code": totp_code,
        })).await?;
        
        println!("MFA authentication complete!");
    }
    Err(e) => return Err(e.into()),
}
```

## Idempotency Support

### Token Refresh Idempotency

The credential system automatically handles idempotent token refreshes:

```rust
// Multiple concurrent requests for the same token are deduplicated
let token1_future = manager.get_token(&credential_id);
let token2_future = manager.get_token(&credential_id);
let token3_future = manager.get_token(&credential_id);

// Only one actual refresh happens
let (token1, token2, token3) = tokio::join!(
    token1_future,
    token2_future,
    token3_future
);

// All receive the same token
assert_eq!(token1.value.expose(), token2.value.expose());
assert_eq!(token2.value.expose(), token3.value.expose());
```

### Credential Creation Idempotency

Prevent duplicate credential creation:

```rust
let input = ApiKeyInput {
    api_key: "sk-same-key",
    auth_method: "header".to_string(),
    param_name: "X-API-Key".to_string(),
};

// Create with idempotency key
let credential_id1 = manager.create_credential_idempotent(
    "api_key",
    serde_json::to_value(&input)?,
    &user_context,
    Some("unique-request-123"), // Idempotency key
).await?;

// Same idempotency key returns same credential
let credential_id2 = manager.create_credential_idempotent(
    "api_key",
    serde_json::to_value(&input)?,
    &user_context,
    Some("unique-request-123"), // Same key
).await?;

assert_eq!(credential_id1, credential_id2);
```

### Configuration

```rust
let manager = CredentialManager::builder()
    .with_idempotency_config(IdempotencyConfig {
        enabled: true,
        token_deduplication_window: Duration::from_secs(60),
        creation_deduplication_window: Duration::from_hours(24),
        storage_backend: IdempotencyStorageBackend::TierSpecific,
    })
    .build()
    .await?;
```

## Integration with Actions

### Using Credentials in Actions

```rust
use nebula_action::prelude::*;
use nebula_credential::prelude::*;

#[derive(Action)]
#[action(
    id = "api.call",
    name = "API Call with Authentication"
)]
#[auth(api_key)]  // Declares credential requirement
pub struct AuthenticatedApiAction;

#[async_trait]
impl ProcessAction for AuthenticatedApiAction {
    type Input = ApiInput;
    type Output = ApiOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Get authenticated HTTP client automatically
        let client = context.get_client::<reqwest::Client>("api_key").await?;
        
        // Make authenticated request
        let response = client
            .get(&input.endpoint)
            .send()
            .await?;
        
        Ok(ActionResult::Success(ApiOutput {
            status: response.status().as_u16(),
            body: response.text().await?,
        }))
    }
}
```

### Multiple Credentials

```rust
#[derive(Action)]
#[action(id = "multi.service")]
#[auth(telegram_bot, openai_api, database)]
pub struct MultiServiceAction;

#[async_trait]
impl ProcessAction for MultiServiceAction {
    type Input = MultiServiceInput;
    type Output = MultiServiceOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Each client is automatically authenticated
        let telegram = context.get_client::<TelegramBot>("telegram_bot").await?;
        let openai = context.get_client::<OpenAIClient>("openai_api").await?;
        let db = context.get_client::<PgPool>("database").await?;
        
        // Use all three services
        let db_data = db.query(&input.query).await?;
        let ai_summary = openai.summarize(&db_data).await?;
        telegram.send_message(&input.chat_id, &ai_summary).await?;
        
        Ok(ActionResult::Success(MultiServiceOutput {
            message_sent: true,
            summary: ai_summary,
        }))
    }
}
```

### Dynamic Credential Selection

```rust
#[derive(Action)]
#[action(id = "dynamic.auth")]
pub struct DynamicAuthAction;

#[async_trait]
impl ProcessAction for DynamicAuthAction {
    type Input = DynamicInput;
    type Output = DynamicOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Dynamically select credential based on input
        let credential_id = match input.service_type {
            ServiceType::Premium => "premium_api_key",
            ServiceType::Standard => "standard_api_key",
            ServiceType::Free => "free_api_key",
        };
        
        // Get token directly
        let token = context.get_credential(credential_id).await?;
        
        // Create custom client with token
        let client = self.create_client_with_token(token)?;
        
        // Use client...
        Ok(ActionResult::Success(output))
    }
}
```

## Security Features

### 1. Encryption at Rest

```rust
// All credentials encrypted using AES-256-GCM
let manager = CredentialManager::builder()
    .with_encryption(EncryptionConfig {
        algorithm: EncryptionAlgorithm::Aes256Gcm,
        key_derivation: KeyDerivation::Argon2id,
        key_rotation_interval: Duration::days(90),
    })
    .build()
    .await?;
```

### 2. Audit Logging

```rust
// Comprehensive audit trail
let manager = CredentialManager::builder()
    .with_audit_logger(AuditConfig {
        log_level: AuditLevel::Full,
        retention_days: 365,
        pii_handling: PiiHandling::Redact,
        storage: AuditStorage::Database {
            connection_string: "postgres://...",
        },
    })
    .build()
    .await?;

// Query audit logs
let logs = manager.query_audit_logs(AuditQuery {
    credential_id: Some(credential_id),
    user_id: None,
    action_type: Some(AuditAction::GetToken),
    date_range: (Utc::now() - Duration::days(7), Utc::now()),
}).await?;
```

### 3. Access Control

```rust
// Role-based access control
let access_control = AccessControl::new()
    .add_role("admin", vec![
        Permission::CreateCredential,
        Permission::ReadCredential,
        Permission::UpdateCredential,
        Permission::DeleteCredential,
        Permission::RotateCredential,
    ])
    .add_role("user", vec![
        Permission::ReadCredential,
        Permission::UseCredential,
    ])
    .add_role("service", vec![
        Permission::UseCredential,
    ]);

let manager = CredentialManager::builder()
    .with_access_control(access_control)
    .build()
    .await?;

// Check permissions
let user_context = UserContext::new("user123", "192.168.1.1")
    .with_roles(vec!["user"]);

// This will fail with PermissionDenied
manager.delete_credential(&credential_id, &user_context).await?;
```

### 4. Credential Rotation

```rust
// Automatic rotation policy
let rotation_policy = RotationPolicy {
    rotation_interval: Duration::days(30),
    warning_period: Duration::days(7),
    grace_period: Duration::hours(24),
    rotation_strategy: RotationStrategy::CreateNewBeforeDelete,
};

let manager = CredentialManager::builder()
    .with_rotation_policy(rotation_policy)
    .build()
    .await?;

// Manual rotation
let new_credential_id = manager.rotate_credential(&old_credential_id).await?;

// Check rotation status
let status = manager.get_rotation_status(&credential_id).await?;
match status {
    RotationStatus::Current => println!("Credential is current"),
    RotationStatus::WarningPeriod { expires_in } => {
        println!("Credential expires in {:?}", expires_in);
    }
    RotationStatus::GracePeriod { new_credential_id } => {
        println!("In grace period, new credential: {}", new_credential_id);
    }
    RotationStatus::Expired => println!("Credential has expired"),
}
```

## Testing

### Unit Testing with Mock Credentials

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::testing::*;
    
    #[tokio::test]
    async fn test_credential_lifecycle() {
        // Create test manager with in-memory storage
        let manager = TestCredentialManager::new();
        
        // Create test credential
        let credential_id = manager.create_test_credential(
            TestCredential::api_key("test-key-123")
        ).await?;
        
        // Get token
        let token = manager.get_token(&credential_id).await?;
        assert_eq!(token.value.expose(), "Bearer test-key-123");
        
        // Test refresh
        manager.expire_credential(&credential_id).await?;
        let result = manager.get_token(&credential_id).await;
        assert!(matches!(result, Err(CredentialError::CredentialExpired)));
    }
    
    #[tokio::test]
    async fn test_oauth_flow() {
        let manager = TestCredentialManager::new();
        
        // Mock OAuth provider
        let mock_provider = MockOAuthProvider::new()
            .expect_authorize("http://localhost/auth")
            .expect_token_exchange("test-token", "test-refresh");
        
        manager.register_mock_provider("google", mock_provider);
        
        // Test OAuth flow
        let flow_result = manager.start_oauth_flow(
            "google",
            "client-id",
            vec!["scope1", "scope2"],
        ).await?;
        
        // Simulate callback
        let credential_id = manager.complete_oauth_flow(
            flow_result.flow_id,
            "auth-code-123",
        ).await?;
        
        // Verify token
        let token = manager.get_token(&credential_id).await?;
        assert_eq!(token.value.expose(), "test-token");
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_with_real_services() {
    // Use test containers for external services
    let postgres = PostgresContainer::new().start().await;
    let vault = VaultContainer::new().start().await;
    
    let manager = CredentialManager::builder()
        .with_storage_backend(StorageBackend::Postgres {
            url: postgres.connection_string(),
        })
        .with_secret_backend(SecretBackend::Vault {
            url: vault.url(),
            token: vault.root_token(),
        })
        .build()
        .await?;
    
    // Test with real backends
    let credential_id = manager.create_credential(
        "api_key",
        json!({ "api_key": "test-key" }),
        &UserContext::system(),
    ).await?;
    
    // Verify storage
    let stored = postgres.query_one(
        "SELECT * FROM credentials WHERE id = $1",
        &[&credential_id],
    ).await?;
    assert!(stored.is_some());
    
    // Verify secrets
    let secret = vault.read_secret(&format!("credentials/{}", credential_id)).await?;
    assert!(secret.contains_key("api_key"));
}
```

## Best Practices

### 1. Credential Design

- **Minimize Scope**: Request only necessary permissions
- **Use Short-Lived Tokens**: Prefer tokens that expire
- **Support Refresh**: Implement refresh for long-running operations
- **Validate Input**: Thoroughly validate all credential inputs

### 2. Security

- **Never Log Credentials**: Use SecureString for sensitive data
- **Encrypt at Rest**: Always encrypt stored credentials
- **Audit Everything**: Log all credential operations
- **Rotate Regularly**: Implement rotation policies

### 3. Error Handling

- **Graceful Degradation**: Handle credential failures gracefully
- **Retry Logic**: Implement smart retry for transient failures
- **Clear Error Messages**: Provide actionable error messages
- **Fallback Options**: Support credential fallback chains

### 4. Performance

- **Cache Tokens**: Cache valid tokens to reduce API calls
- **Batch Operations**: Support bulk credential operations
- **Async Everything**: Use async for all I/O operations
- **Connection Pooling**: Reuse authenticated connections

### 5. Testing

- **Mock External Services**: Use mocks for unit tests
- **Test Error Paths**: Test credential failures
- **Integration Tests**: Test with real services in CI
- **Security Tests**: Test encryption and access control

## Troubleshooting

### Common Issues

1. **Token Expired**
   - Check token TTL and refresh configuration
   - Ensure refresh tokens are stored properly
   - Verify system time synchronization

2. **Permission Denied**
   - Check user roles and permissions
   - Verify credential scopes
   - Review audit logs for details

3. **Storage Errors**
   - Check storage backend connectivity
   - Verify encryption keys
   - Check disk space for file storage

4. **Interactive Flow Failures**
   - Verify callback URLs
   - Check CSRF token handling
   - Ensure proper state management

## Migration Guide

### From External Credential Systems

```rust
// Import from environment variables
let importer = CredentialImporter::new();
importer.import_from_env(vec![
    ("API_KEY", "api_key", json!({ "param_name": "X-API-Key" })),
    ("DATABASE_URL", "postgres", json!({ "parse_connection_string": true })),
]).await?;

// Import from other credential stores
importer.import_from_vault(vault_client, "/secret/credentials/*").await?;
importer.import_from_aws_secrets_manager(aws_client, "prod/*").await?;
```

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

## License

Licensed under MIT or Apache-2.0 at your option.::State,
        context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        Err(CredentialError::RefreshNotSupported)
    }
    
    /// Validate credential state
    fn validate_state(&self, state: &Self::State) -> Result<(), CredentialError> {
        if state.is_valid() {
            Ok(())
        } else {
            Err(CredentialError::InvalidState)
        }
    }
}

/// Credential state trait
pub trait CredentialState: Serialize + DeserializeOwned + Send + Sync {
    /// Check if state is still valid
    fn is_valid(&self) -> bool;
    
    /// Check if refresh is needed
    fn needs_refresh(&self) -> bool;
    
    /// Get expiration time if any
    fn expires_at(&self) -> Option<DateTime<Utc>>;
}
```

### Token Types

```rust
/// Authentication token
#[derive(Debug, Clone)]
pub struct Token {
    /// Token value (encrypted in memory)
    pub value: SecureString,
    
    /// Token type
    pub token_type: TokenType,
    
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    
    /// Associated scopes
    pub scopes: Vec<String>,
    
    /// Additional claims/metadata
    pub claims: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    Bearer,
    Basic,
    ApiKey,
    JWT,
    OAuth2,
    AWS,
    Custom(String),
}

/// Secure string that zeros memory on drop
pub struct SecureString(Box<[u8]>);

impl SecureString {
    pub fn new(value: impl Into<String>) -> Self {
        let bytes = value.into().into_bytes();
        Self(bytes.into_boxed_slice())
    }
    
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap()
    }
}

impl Drop for SecureString {
    fn drop(&mut self) {
        // Zero out memory
        for byte in self.0.iter_mut() {
            *byte = 0;
        }
    }
}
```

### CredentialManager

The main interface for credential operations:

```rust
pub struct CredentialManager {
    storage: Arc<dyn CredentialStorage>,
    registry: Arc<CredentialRegistry>,
    cache: Arc<TokenCache>,
    refresh_scheduler: Arc<RefreshScheduler>,
    audit_logger: Arc<AuditLogger>,
    encryption: Arc<EncryptionService>,
}

impl CredentialManager {
    /// Create a new credential
    pub async fn create_credential(
        &self,
        credential_type: &str,
        input: Value,
        context: &UserContext,
    ) -> Result<CredentialId, CredentialError> {
        // Audit log
        self.audit_logger.log_create_attempt(credential_type, context).await;
        
        // Get credential implementation
        let credential = self.registry.get(credential_type)?;
        
        // Create credential context
        let mut cred_context = CredentialContext::new(self.clone());
        
        // Initialize
        match credential.initialize(&input, &mut cred_context).await? {
            InitializeResult::Ready => {
                // Save to storage
                let id = CredentialId::new();
                self.storage.save(
                    &id,
                    credential_type,
                    &input,
                    &cred_context.state,
                    context,
                ).await?;
                
                // Schedule refresh if needed
                if let Some(state) = &cred_context.state {
                    if let Some(expires_at) = state.expires_at() {
                        self.refresh_scheduler.schedule(&id, expires_at).await;
                    }
                }
                
                self.audit_logger.log_create_success(&id, credential_type, context).await;
                Ok(id)
            }
            InitializeResult::RequiresInteraction { .. } => {
                Err(CredentialError::InteractionRequired)
            }
        }
    }
    
    /// Get token for credential
    pub async fn get_token(&self, credential_id: &CredentialId) -> Result<Token, CredentialError> {
        // Check cache first
        if let Some(token) = self.cache.get(credential_id).await {
            if !token.is_expired() {
                return Ok(token);
            }
        }
        
        // Load credential
        let (cred_type, input, state) = self.storage.load(credential_id).await?;
        let credential = self.registry.get(&cred_type)?;
        
        // Create context
        let mut context = CredentialContext::new(self.clone());
        context.set_input(input);
        context.set_state(state);
        
        // Get token
        match credential.get_token(&state, &mut context).await? {
            TokenResult::Token(token) => {
                // Cache token
                self.cache.put(credential_id, &token).await;
                Ok(token)
            }
            TokenResult::NeedsRefresh => {
                // Try refresh
                self.refresh_credential(credential_id).await
            }
            TokenResult::Expired => {
                Err(CredentialError::CredentialExpired)
            }
        }
    }
    
    /// Refresh credential
    pub async fn refresh_credential(&self, credential_id: &CredentialId) -> Result<Token, CredentialError> {
        let (cred_type, input, mut state) = self.storage.load(credential_id).await?;
        let credential = self.registry.get(&cred_type)?;
        
        let mut context = CredentialContext::new(self.clone());
        context.set_input(input);
        
        match credential.refresh_token(&mut state, &mut context).await? {
            TokenResult::Token(token) => {
                // Update state
                self.storage.update_state(credential_id, &state).await?;
                
                // Cache new token
                self.cache.put(credential_id, &token).await;
                
                // Reschedule refresh
                if let Some(expires_at) = token.expires_at {
                    self.refresh_scheduler.schedule(credential_id, expires_at).await;
                }
                
                Ok(token)
            }
            _ => Err(CredentialError::RefreshFailed),
        }
    }
}
```

## Credential Types

### 1. API Key Credential

```rust
use nebula_credential::prelude::*;

#[derive(Credential)]
#[credential(
    id = "api_key",
    name = "API Key",
    description = "Simple API key authentication"
)]
pub struct ApiKeyCredential;

#[derive(Parameters)]
pub struct ApiKeyInput {
    #[parameter(description = "API key value", sensitive = true)]
    pub api_key: String,
    
    #[parameter(description = "How to send the key", default = "header")]
    pub auth_method: String, // "header", "query", "body"
    
    #[parameter(description = "Parameter name", default = "Authorization")]
    pub param_name: String,
    
    #[parameter(description = "Key prefix", default = "Bearer")]
    pub prefix: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ApiKeyState {
    pub api_key: String,
    pub auth_method: String,
    pub param_name: String,
    pub prefix: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl CredentialState for ApiKeyState {
    fn is_valid(&self) -> bool {
        !self.api_key.is_empty()
    }
    
    fn needs_refresh(&self) -> bool {
        false // API keys don't need refresh
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        None // API keys don't expire
    }
}

#[async_trait]
impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type State = ApiKeyState;
    
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError> {
        // Validate API key
        if input.api_key.is_empty() {
            return Err(CredentialError::ValidationFailed("API key cannot be empty".into()));
        }
        
        // Create state
        let state = ApiKeyState {
            api_key: input.api_key.clone(),
            auth_method: input.auth_method.clone(),
            param_name: input.param_name.clone(),
            prefix: input.prefix.clone(),
            created_at: Utc::now(),
        };
        
        context.save_state(&state).await?;
        Ok(InitializeResult::Ready)
    }
    
    async fn get_token(
        &self,
        state: &Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        let value = if let Some(prefix) = &state.prefix {
            format!("{} {}", prefix, state.api_key)
        } else {
            state.api_key.clone()
        };
        
        Ok(TokenResult::Token(Token {
            value: SecureString::new(value),
            token_type: TokenType::ApiKey,
            expires_at: None,
            scopes: vec![],
            claims: HashMap::new(),
        }))
    }
}

// Client authenticator for API key
#[async_trait]
impl ClientAuthenticator<reqwest::Client> for ApiKeyCredential {
    async fn create_authenticated_client(
        &self,
        context: &CredentialContext,
    ) -> Result<reqwest::Client, CredentialError> {
        let state = context.get_state::<ApiKeyState>()?;
        let token = self.get_token(&state, context).await?.into_token()?;
        
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            &state.param_name,
            token.value.expose().parse().map_err(|_| CredentialError::InvalidToken)?,
        );
        
        Ok(reqwest::Client::builder()
            .default_headers(headers)
            .build()?)
    }
}
```

### 2. OAuth 2.0 Credential

```rust
#[derive(Credential)]
#[credential(
    id = "oauth2",
    name = "OAuth 2.0",
    description = "OAuth 2.0 authentication with automatic refresh"
)]
pub struct OAuth2Credential;

#[derive(Parameters)]
pub struct OAuth2Input {
    #[parameter(description = "OAuth provider configuration")]
    pub provider: OAuth2Provider,
    
    #[parameter(description = "Client ID")]
    pub client_id: String,
    
    #[parameter(description = "Client secret", sensitive = true)]
    pub client_secret: Option<String>,
    
    #[parameter(description = "Requested scopes")]
    pub scopes: Vec<String>,
    
    #[parameter(description = "Redirect URI")]
    pub redirect_uri: String,
}

#[derive(Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

impl CredentialState for OAuth2State {
    fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at
    }
    
    fn needs_refresh(&self) -> bool {
        // Refresh 5 minutes before expiry
        Utc::now() + Duration::minutes(5) > self.expires_at
    }
    
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        Some(self.expires_at)
    }
}

#[async_trait]
impl Credential for OAuth2Credential {
    type Input = OAuth2Input;
    type State = OAuth2State;
    
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "oauth2",
            name: "OAuth 2.0",
            description: "OAuth 2.0 with automatic token refresh",
            supports_refresh: true,
            requires_interaction: true,
            supported_clients: vec!["http", "grpc"],
        }
    }
    
    async fn initialize(
        &self,
        input: &Self::Input,
        context: &mut CredentialContext,
    ) -> Result<InitializeResult, CredentialError> {
        // Create OAuth client
        let client = self.create_oauth_client(input)?;
        
        // Generate authorization URL
        let (auth_url, csrf_token) = client
            .authorize_url(CsrfToken::new_random)
            .add_scopes(input.scopes.iter().map(|s| Scope::new(s.clone())))
            .url();
        
        // Save CSRF token
        context.set_temp_data("csrf_token", csrf_token.secret())?;
        
        Ok(InitializeResult::RequiresInteraction {
            interaction_type: InteractionType::BrowserAuth {
                auth_url: auth_url.to_string(),
                callback_url: input.redirect_uri.clone(),
            },
            state_token: context.state_token(),
            expires_in: Duration::minutes(10),
        })
    }
    
    async fn get_token(
        &self,
        state: &Self::State,
        _context: &mut CredentialContext,
    ) -> Result<TokenResult, CredentialError> {
        if state.is_valid() {
            Ok(TokenResult::Token(Token {
                value: SecureString::new(&state.access_token),
                token_type: TokenType::Bearer,
                expires_at: Some(state.expires_at),
                scopes: state.scopes.clone(),
                claims: HashMap::new(),
            }))
        } else if state.needs_refresh() && state.refresh_token.is_some() {
            Ok(TokenResult::NeedsRefresh)
        } else {
            Ok(TokenResult::Expired)
        }
    }
    
    async fn refresh_token(
        &self,
        state: &mut Self