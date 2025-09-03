---

title: Credential Integration
tags: [nebula-resource, how-to, credentials, security]
status: stable
created: 2025-08-17
---

# Credential Integration

Complete guide to integrating nebula-resource with nebula-credential for secure credential management.

## Overview

Resources often need credentials to access external services. This integration provides:

- Automatic credential injection
- Credential rotation support
- Secure storage and retrieval
- Audit logging
- Multi-provider support

## Basic Credential Usage

### Step 1: Declare Required Credentials

```rust
use nebula_resource::prelude::*;
use nebula_credential::prelude::*;

#[derive(Resource)]
#[resource(
    id = "payment_processor",
    name = "Payment Processing Service",
    // Declare required credentials
    credentials = ["stripe_secret_key", "stripe_webhook_secret", "encryption_key"]
)]
pub struct PaymentProcessorResource;

#[derive(ResourceConfig)]
pub struct PaymentProcessorConfig {
    /// Reference to Stripe API key credential
    #[credential(id = "stripe_secret_key", required = true)]
    pub stripe_key_credential: String,
    
    /// Reference to webhook secret
    #[credential(id = "stripe_webhook_secret", required = true)]
    pub webhook_secret_credential: String,
    
    /// Optional encryption key for sensitive data
    #[credential(id = "encryption_key", required = false)]
    pub encryption_key_credential: Option<String>,
    
    /// API endpoint (not a credential)
    #[validate(url)]
    pub api_endpoint: String,
    
    /// Rate limiting
    #[validate(range = "1..=1000")]
    pub max_requests_per_minute: u32,
}
```

### Step 2: Retrieve Credentials in Resource

```rust
#[async_trait]
impl Resource for PaymentProcessorResource {
    type Config = PaymentProcessorConfig;
    type Instance = PaymentProcessorInstance;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        // Retrieve credentials through context
        let stripe_key = context
            .get_credential(&config.stripe_key_credential)
            .await
            .map_err(|e| ResourceError::MissingCredential(
                format!("Failed to get Stripe key: {}", e)
            ))?;
        
        let webhook_secret = context
            .get_credential(&config.webhook_secret_credential)
            .await
            .map_err(|e| ResourceError::MissingCredential(
                format!("Failed to get webhook secret: {}", e)
            ))?;
        
        // Optional credential
        let encryption_key = if let Some(ref cred_name) = config.encryption_key_credential {
            Some(context.get_credential(cred_name).await?)
        } else {
            None
        };
        
        // Create client with credentials
        let stripe_client = stripe::Client::new(stripe_key.expose_secret());
        
        // Test connection with credentials
        stripe_client
            .ping()
            .await
            .map_err(|e| ResourceError::CreationFailed(
                format!("Failed to connect to Stripe: {}", e)
            ))?;
        
        context.log_info("PaymentProcessor resource created with credentials");
        
        Ok(PaymentProcessorInstance {
            stripe_client,
            webhook_secret,
            encryption_key,
            config: config.clone(),
        })
    }
    
    fn required_credentials() -> Vec<&'static str> {
        vec!["stripe_secret_key", "stripe_webhook_secret"]
    }
}
```

## Advanced: Credential Rotation

### Implement Rotation Support

```rust
use nebula_credential::rotation::*;

/// Trait for resources that support credential rotation
#[async_trait]
pub trait CredentialRotationAware: ResourceInstance {
    /// Called when a credential is rotated
    async fn on_credential_rotated(
        &mut self,
        credential_id: &str,
        new_credential: &Credential,
    ) -> Result<(), ResourceError>;
    
    /// Test if new credential works
    async fn validate_credential(
        &self,
        credential_id: &str,
        credential: &Credential,
    ) -> Result<(), ResourceError>;
    
    /// Get grace period for rotation
    fn rotation_grace_period(&self) -> Duration {
        Duration::from_minutes(5)
    }
}

pub struct PaymentProcessorInstance {
    stripe_client: Arc<RwLock<stripe::Client>>,
    webhook_secret: Arc<RwLock<SecretString>>,
    encryption_key: Option<Arc<RwLock<SecretString>>>,
    config: PaymentProcessorConfig,
    rotation_state: Arc<RwLock<RotationState>>,
}

#[derive(Default)]
struct RotationState {
    rotating_credentials: HashSet<String>,
    rotation_history: Vec<RotationRecord>,
}

#[derive(Debug, Clone)]
struct RotationRecord {
    credential_id: String,
    rotated_at: DateTime<Utc>,
    success: bool,
    error: Option<String>,
}

#[async_trait]
impl CredentialRotationAware for PaymentProcessorInstance {
    async fn on_credential_rotated(
        &mut self,
        credential_id: &str,
        new_credential: &Credential,
    ) -> Result<(), ResourceError> {
        let mut rotation_state = self.rotation_state.write().await;
        rotation_state.rotating_credentials.insert(credential_id.to_string());
        
        let result = match credential_id {
            "stripe_secret_key" => {
                self.rotate_stripe_key(new_credential).await
            }
            "stripe_webhook_secret" => {
                self.rotate_webhook_secret(new_credential).await
            }
            "encryption_key" => {
                self.rotate_encryption_key(new_credential).await
            }
            _ => {
                Err(ResourceError::UnknownCredential(credential_id.to_string()))
            }
        };
        
        // Record rotation
        rotation_state.rotation_history.push(RotationRecord {
            credential_id: credential_id.to_string(),
            rotated_at: Utc::now(),
            success: result.is_ok(),
            error: result.as_ref().err().map(|e| e.to_string()),
        });
        
        rotation_state.rotating_credentials.remove(credential_id);
        
        result
    }
    
    async fn validate_credential(
        &self,
        credential_id: &str,
        credential: &Credential,
    ) -> Result<(), ResourceError> {
        match credential_id {
            "stripe_secret_key" => {
                // Test new Stripe key
                let test_client = stripe::Client::new(credential.expose_secret());
                test_client
                    .ping()
                    .await
                    .map_err(|e| ResourceError::InvalidCredential(e.to_string()))?;
            }
            "stripe_webhook_secret" => {
                // Webhook secrets are validated on first use
                // Just check format
                if credential.expose_secret().len() < 32 {
                    return Err(ResourceError::InvalidCredential(
                        "Webhook secret too short".into()
                    ));
                }
            }
            "encryption_key" => {
                // Validate encryption key format
                let key = credential.expose_secret();
                if key.len() != 32 && key.len() != 64 {
                    return Err(ResourceError::InvalidCredential(
                        "Encryption key must be 32 or 64 bytes".into()
                    ));
                }
            }
            _ => {}
        }
        
        Ok(())
    }
}

impl PaymentProcessorInstance {
    async fn rotate_stripe_key(&self, new_credential: &Credential) -> Result<(), ResourceError> {
        // Create new client with new key
        let new_client = stripe::Client::new(new_credential.expose_secret());
        
        // Test new client
        new_client
            .ping()
            .await
            .map_err(|e| ResourceError::CredentialRotationFailed(
                format!("New Stripe key validation failed: {}", e)
            ))?;
        
        // Atomic swap
        let mut client_guard = self.stripe_client.write().await;
        *client_guard = new_client;
        
        log::info!("Successfully rotated Stripe API key");
        Ok(())
    }
    
    async fn rotate_webhook_secret(&self, new_credential: &Credential) -> Result<(), ResourceError> {
        // Update webhook secret
        let mut secret_guard = self.webhook_secret.write().await;
        *secret_guard = SecretString::from(new_credential.expose_secret());
        
        log::info!("Successfully rotated webhook secret");
        Ok(())
    }
    
    async fn rotate_encryption_key(&self, new_credential: &Credential) -> Result<(), ResourceError> {
        if let Some(ref key_lock) = self.encryption_key {
            // Important: May need to re-encrypt existing data
            log::warn!("Rotating encryption key - ensure data migration is handled");
            
            let mut key_guard = key_lock.write().await;
            *key_guard = SecretString::from(new_credential.expose_secret());
            
            log::info!("Successfully rotated encryption key");
        }
        
        Ok(())
    }
}
```

## Credential Providers

### Multi-Provider Support

```rust
/// Configure different credential providers per resource
#[derive(ResourceConfig)]
pub struct MultiProviderConfig {
    /// AWS Secrets Manager for production keys
    #[credential(
        id = "api_key",
        provider = "aws_secrets_manager",
        secret_name = "prod/api/key"
    )]
    pub api_key_credential: String,
    
    /// HashiCorp Vault for database credentials
    #[credential(
        id = "db_password",
        provider = "vault",
        path = "database/creds/readonly"
    )]
    pub db_password_credential: String,
    
    /// Environment variable for development
    #[credential(
        id = "dev_token",
        provider = "env",
        env_var = "DEV_API_TOKEN"
    )]
    pub dev_token_credential: String,
    
    /// Kubernetes secret
    #[credential(
        id = "k8s_secret",
        provider = "kubernetes",
        secret_name = "app-secrets",
        key = "api-key"
    )]
    pub k8s_secret_credential: String,
}
```

### Custom Credential Provider

```rust
use nebula_credential::provider::*;

/// Custom credential provider for your organization
pub struct CustomCredentialProvider {
    client: CustomVaultClient,
}

#[async_trait]
impl CredentialProvider for CustomCredentialProvider {
    async fn get_credential(
        &self,
        credential_id: &str,
        metadata: &CredentialMetadata,
    ) -> Result<Credential, CredentialError> {
        // Fetch from custom vault
        let secret = self.client
            .get_secret(&metadata.path)
            .await?;
        
        Ok(Credential::new(
            credential_id.to_string(),
            SecretString::from(secret.value),
            CredentialType::ApiKey,
            Some(secret.expires_at),
        ))
    }
    
    async fn refresh_credential(
        &self,
        credential_id: &str,
        current: &Credential,
    ) -> Result<Credential, CredentialError> {
        // Refresh logic for custom provider
        if current.is_expired() {
            self.get_credential(credential_id, &current.metadata).await
        } else {
            Ok(current.clone())
        }
    }
    
    fn supports_rotation(&self) -> bool {
        true
    }
    
    async fn rotate_credential(
        &self,
        credential_id: &str,
        current: &Credential,
    ) -> Result<Credential, CredentialError> {
        // Trigger rotation in custom vault
        let new_secret = self.client
            .rotate_secret(&current.metadata.path)
            .await?;
        
        Ok(Credential::new(
            credential_id.to_string(),
            SecretString::from(new_secret.value),
            current.credential_type.clone(),
            Some(new_secret.expires_at),
        ))
    }
}
```

## Credential Validation

### Pre-flight Validation

```rust
impl ResourceManager {
    /// Validate all required credentials before resource creation
    pub async fn validate_credentials<R: Resource>(
        &self,
        resource: &R,
    ) -> Result<CredentialValidationReport, ValidationError> {
        let mut report = CredentialValidationReport::new();
        
        for credential_id in R::required_credentials() {
            match self.credential_manager.validate_credential(credential_id).await {
                Ok(validation) => {
                    report.add_valid(credential_id, validation);
                }
                Err(e) => {
                    report.add_invalid(credential_id, e);
                }
            }
        }
        
        if !report.is_valid() {
            return Err(ValidationError::InvalidCredentials(report));
        }
        
        Ok(report)
    }
}

#[derive(Debug)]
pub struct CredentialValidationReport {
    valid: Vec<CredentialValidation>,
    invalid: Vec<CredentialError>,
}

#[derive(Debug)]
pub struct CredentialValidation {
    credential_id: String,
    provider: String,
    expires_at: Option<DateTime<Utc>>,
    rotation_supported: bool,
    last_rotated: Option<DateTime<Utc>>,
}
```

## Secure Credential Handling

### Best Practices Implementation

```rust
pub struct SecureResourceInstance {
    // Never store credentials directly
    client: Arc<SecureClient>,
    
    // Use secure channels for sensitive operations
    secure_channel: Arc<SecureChannel>,
    
    // Audit all credential access
    audit_log: Arc<AuditLogger>,
}

impl SecureResourceInstance {
    pub async fn perform_secure_operation(&self, input: SecureInput) -> Result<SecureOutput> {
        // Audit credential access
        self.audit_log.log_credential_access(
            "secure_operation",
            &input.credential_id,
            self.get_context(),
        ).await;
        
        // Get credential with minimal exposure
        let result = self.secure_channel
            .execute_with_credential(&input.credential_id, |cred| async {
                // Credential is only available within this closure
                self.client.authenticate(cred).await?;
                self.client.perform_operation(input).await
            })
            .await?;
        
        // Audit completion
        self.audit_log.log_operation_complete(
            "secure_operation",
            result.success,
        ).await;
        
        Ok(result)
    }
}

/// Secure channel that minimizes credential exposure
pub struct SecureChannel {
    credential_manager: Arc<CredentialManager>,
    memory_protector: Arc<MemoryProtector>,
}

impl SecureChannel {
    pub async fn execute_with_credential<F, Fut, T>(
        &self,
        credential_id: &str,
        operation: F,
    ) -> Result<T, SecureError>
    where
        F: FnOnce(&Credential) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // Get credential with memory protection
        let credential = self.credential_manager
            .get_credential_protected(credential_id)
            .await?;
        
        // Lock memory to prevent swapping
        let _guard = self.memory_protector.lock_memory(&credential)?;
        
        // Execute operation
        let result = operation(&credential).await;
        
        // Credential is automatically zeroed when dropped
        drop(credential);
        
        result
    }
}
```

## Testing with Mock Credentials

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::testing::*;
    use nebula_credential::testing::*;
    
    #[tokio::test]
    async fn test_resource_with_credentials() {
        // Create test context with mock credentials
        let context = TestContext::builder()
            .with_credential("stripe_secret_key", TestCredential::api_key("sk_test_123"))
            .with_credential("stripe_webhook_secret", TestCredential::secret("whsec_test_456"))
            .with_credential("encryption_key", TestCredential::encryption_key(32))
            .build();
        
        // Create resource
        let resource = PaymentProcessorResource;
        let config = PaymentProcessorConfig {
            stripe_key_credential: "stripe_secret_key".into(),
            webhook_secret_credential: "stripe_webhook_secret".into(),
            encryption_key_credential: Some("encryption_key".into()),
            api_endpoint: "https://api.stripe.com".into(),
            max_requests_per_minute: 100,
        };
        
        let instance = resource.create(&config, &context).await.unwrap();
        
        // Test that credentials were properly injected
        assert!(instance.stripe_client.is_authenticated());
    }
    
    #[tokio::test]
    async fn test_credential_rotation() {
        let mut instance = create_test_instance().await;
        
        // Simulate credential rotation
        let new_credential = TestCredential::api_key("sk_test_new_789");
        
        instance
            .on_credential_rotated("stripe_secret_key", &new_credential)
            .await
            .unwrap();
        
        // Verify new credential is in use
        assert!(instance.validate_credential("stripe_secret_key", &new_credential).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_missing_credential() {
        let context = TestContext::builder()
            // Missing required credentials
            .build();
        
        let resource = PaymentProcessorResource;
        let config = create_test_config();
        
        let result = resource.create(&config, &context).await;
        
        assert!(matches!(
            result,
            Err(ResourceError::MissingCredential(_))
        ));
    }
}
```

## Configuration

```toml
# Resource configuration with credential references
[payment_processor]
api_endpoint = "https://api.stripe.com"
max_requests_per_minute = 600

# Credential references (not actual secrets!)
[payment_processor.credentials]
stripe_key_credential = "stripe_secret_key"
webhook_secret_credential = "stripe_webhook_secret"
encryption_key_credential = "data_encryption_key"

# Credential provider configuration
[credentials.providers]
default = "aws_secrets_manager"

[credentials.providers.aws_secrets_manager]
region = "us-east-1"
endpoint = "https://secretsmanager.amazonaws.com"

[credentials.providers.vault]
address = "https://vault.example.com"
namespace = "production"
auth_method = "kubernetes"

# Rotation policy
[credentials.rotation]
enabled = true
check_interval = "1h"
grace_period = "5m"
max_rotation_attempts = 3
```

## Best Practices

1. **Never log credentials** - Use audit logs instead
2. **Minimize credential exposure** - Use secure channels
3. **Support rotation** - Implement CredentialRotationAware
4. **Validate before use** - Test credentials during creation
5. **Use appropriate providers** - Match security requirements
6. **Implement pre-flight checks** - Validate all credentials upfront
7. **Audit access** - Log all credential usage
8. **Test with mocks** - Never use real credentials in tests
9. **Zero credentials on drop** - Ensure memory cleanup
10. **Use memory protection** - Prevent swapping sensitive data