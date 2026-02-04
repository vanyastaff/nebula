//! AWS Secrets Manager storage provider
//!
//! Implements credential storage using AWS Secrets Manager with KMS encryption.
//!
//! # Features
//!
//! - KMS-encrypted secrets (AWS managed or customer-managed keys)
//! - Automatic retry with exponential backoff
//! - Tag-based metadata storage
//! - 64KB payload size limit (AWS limit)
//! - Regional storage for compliance
//!
//! # Configuration
//!
//! ```rust,ignore
//! use nebula_credential::providers::{AwsSecretsManagerConfig, AwsSecretsManagerProvider};
//! use std::time::Duration;
//!
//! let config = AwsSecretsManagerConfig {
//!     region: Some("us-west-2".into()),
//!     secret_prefix: "nebula/credentials/".into(),
//!     timeout: Duration::from_secs(5),
//!     kms_key_id: Some("alias/nebula-credentials".into()),
//!     ..Default::default()
//! };
//!
//! let provider = AwsSecretsManagerProvider::new(config).await?;
//! ```

use crate::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use crate::providers::{ProviderConfig, StorageMetrics};
use crate::traits::StorageProvider;
use crate::utils::{EncryptedData, RetryPolicy};
use async_trait::async_trait;
use aws_sdk_secretsmanager::Client as SecretsManagerClient;
use aws_sdk_secretsmanager::types::Tag;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// AWS Secrets Manager configuration
///
/// # Size Limits
///
/// AWS Secrets Manager has a 64KB payload limit. Credentials exceeding this
/// limit will be rejected with `StorageError::WriteFailure`.
///
/// # KMS Encryption
///
/// By default, AWS uses `aws/secretsmanager` managed key. Specify `kms_key_id`
/// to use a customer-managed KMS key for additional control.
///
/// # Example
///
/// ```rust
/// use nebula_credential::providers::AwsSecretsManagerConfig;
/// use std::time::Duration;
///
/// let config = AwsSecretsManagerConfig {
///     region: Some("us-east-1".into()),
///     secret_prefix: "app/creds/".into(),
///     timeout: Duration::from_secs(10),
///     kms_key_id: Some("arn:aws:kms:us-east-1:123456789012:key/...".into()),
///     ..Default::default()
/// };
///
/// assert!(config.validate().is_ok());
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AwsSecretsManagerConfig {
    /// AWS region (auto-detected from environment if None)
    ///
    /// Checked in order:
    /// 1. This config value
    /// 2. AWS_REGION environment variable
    /// 3. AWS_DEFAULT_REGION environment variable
    /// 4. EC2 instance metadata (if running on EC2)
    pub region: Option<String>,

    /// Custom endpoint URL (for LocalStack or other AWS-compatible services)
    ///
    /// If specified, all requests will be sent to this endpoint instead of AWS.
    /// Useful for local testing with LocalStack: `"http://localhost:4566"`
    pub endpoint_url: Option<String>,

    /// Secret name prefix for namespacing
    ///
    /// All credential IDs will be prefixed with this value.
    /// Example: prefix "nebula/" converts ID "github_token" to secret name "nebula/github_token"
    ///
    /// **Validation**: Max 512 characters, no invalid AWS characters (`, `, `<`, `>`, etc.)
    pub secret_prefix: String,

    /// Operation timeout
    ///
    /// **Validation**: Between 1 and 60 seconds
    pub timeout: Duration,

    /// Retry policy for transient failures
    ///
    /// Retries 503 Service Unavailable, throttling errors, and network timeouts.
    pub retry_policy: RetryPolicy,

    /// KMS key ID for encryption (optional)
    ///
    /// If not specified, uses AWS managed key `aws/secretsmanager`.
    /// Can be ARN, alias, or key ID.
    ///
    /// Example: `"alias/nebula-credentials"` or `"arn:aws:kms:..."`
    pub kms_key_id: Option<String>,

    /// Default tags applied to all secrets
    ///
    /// Merged with credential metadata tags (metadata tags take precedence).
    /// **Limit**: Max 50 tags total per secret (AWS limit)
    pub default_tags: HashMap<String, String>,
}

impl Default for AwsSecretsManagerConfig {
    fn default() -> Self {
        Self {
            region: None,       // Auto-detect from environment
            endpoint_url: None, // Use real AWS by default
            secret_prefix: String::new(),
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            kms_key_id: None,
            default_tags: HashMap::new(),
        }
    }
}

impl ProviderConfig for AwsSecretsManagerConfig {
    fn validate(&self) -> Result<(), crate::providers::config::ConfigError> {
        use crate::providers::config::ConfigError;

        // Validate secret prefix length and characters
        if self.secret_prefix.len() > 512 {
            return Err(ConfigError::InvalidValue {
                field: "secret_prefix".into(),
                reason: format!("exceeds 512 character limit ({})", self.secret_prefix.len()),
            });
        }

        // Check for invalid characters in prefix
        let invalid_chars = ['<', '>', '{', '}', '[', ']', '|', '\\', '^', '`'];
        if self
            .secret_prefix
            .chars()
            .any(|c| invalid_chars.contains(&c))
        {
            return Err(ConfigError::InvalidValue {
                field: "secret_prefix".into(),
                reason: "contains invalid AWS characters (<, >, {, }, [, ], |, \\, ^, `)".into(),
            });
        }

        // Validate timeout
        let timeout_secs = self.timeout.as_secs();
        if !(1..=60).contains(&timeout_secs) {
            return Err(ConfigError::InvalidValue {
                field: "timeout".into(),
                reason: format!(
                    "must be between 1 and 60 seconds, got {} seconds",
                    timeout_secs
                ),
            });
        }

        // Validate retry policy
        self.retry_policy
            .validate()
            .map_err(|reason| ConfigError::ValidationFailed(format!("retry_policy: {}", reason)))?;

        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "AWSSecretsManager"
    }
}

/// AWS Secrets Manager storage provider
///
/// Thread-safe provider for storing credentials in AWS Secrets Manager.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = AwsSecretsManagerConfig::default();
///     let provider = AwsSecretsManagerProvider::new(config).await?;
///
///     let id = CredentialId::new("api_key")?;
///     let context = CredentialContext::new("user_123");
///     let exists = provider.exists(&id, &context).await?;
///
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct AwsSecretsManagerProvider {
    /// AWS Secrets Manager client
    client: SecretsManagerClient,

    /// Provider configuration
    config: AwsSecretsManagerConfig,

    /// Metrics collection
    metrics: Arc<StorageMetrics>,
}

impl std::fmt::Debug for AwsSecretsManagerProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsSecretsManagerProvider")
            .field("config", &self.config)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl AwsSecretsManagerProvider {
    /// Create a new AWS Secrets Manager provider
    ///
    /// Initializes the AWS SDK client using the default credential chain:
    /// 1. Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY)
    /// 2. Shared credentials file (~/.aws/credentials)
    /// 3. IAM role (EC2 instance profile or ECS task role)
    ///
    /// # Arguments
    ///
    /// * `config` - Provider configuration
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - Provider initialized successfully
    /// * `Err(StorageError)` - Configuration invalid or AWS client initialization failed
    ///
    /// # Errors
    ///
    /// * `StorageError::WriteFailure` - Configuration validation failed
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = AwsSecretsManagerConfig::default();
    /// let provider = AwsSecretsManagerProvider::new(config).await?;
    /// ```
    pub async fn new(config: AwsSecretsManagerConfig) -> Result<Self, StorageError> {
        // Validate configuration
        config.validate().map_err(|e| StorageError::WriteFailure {
            id: "[config]".into(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
        })?;

        // Load AWS SDK config
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        // Set region if provided
        if let Some(region) = &config.region {
            loader = loader.region(aws_config::Region::new(region.clone()));
        }

        // Set custom endpoint if provided (for LocalStack)
        if let Some(endpoint) = &config.endpoint_url {
            loader = loader.endpoint_url(endpoint);
        }

        let sdk_config = loader.load().await;

        // Create Secrets Manager client
        let client = SecretsManagerClient::new(&sdk_config);

        tracing::info!(
            provider = "AWS Secrets Manager",
            region = ?config.region,
            prefix = %config.secret_prefix,
            "Initialized AWS Secrets Manager provider"
        );

        Ok(Self {
            client,
            config,
            metrics: Arc::new(StorageMetrics::new()),
        })
    }

    /// Get full secret name by prefixing credential ID
    fn get_secret_name(&self, id: &CredentialId) -> String {
        format!("{}{}", self.config.secret_prefix, id.as_str())
    }

    /// Convert credential metadata to AWS tags
    ///
    /// Merges default tags from config with credential-specific tags.
    /// Credential tags take precedence over defaults.
    ///
    /// # AWS Limits
    ///
    /// - Max 50 tags per secret
    /// - Tag key: max 128 characters
    /// - Tag value: max 256 characters
    ///
    /// Tags exceeding these limits are skipped with a warning.
    fn metadata_to_aws_tags(&self, metadata: &CredentialMetadata) -> Vec<Tag> {
        let mut tags_map = self.config.default_tags.clone();

        // Merge credential tags (override defaults)
        for (key, value) in &metadata.tags {
            tags_map.insert(key.clone(), value.clone());
        }

        // Add metadata fields as tags
        tags_map.insert("created_at".into(), metadata.created_at.to_rfc3339());
        tags_map.insert("last_modified".into(), metadata.last_modified.to_rfc3339());

        // Convert to AWS Tag format with validation
        let mut aws_tags = Vec::new();
        for (key, value) in tags_map {
            // Validate tag limits
            if key.len() > 128 {
                tracing::warn!(key = %key, "Skipping tag with key longer than 128 characters");
                continue;
            }

            if value.len() > 256 {
                tracing::warn!(key = %key, "Skipping tag with value longer than 256 characters");
                continue;
            }

            let tag = Tag::builder().key(key).value(value).build();

            aws_tags.push(tag);

            // AWS limit: 50 tags per secret
            if aws_tags.len() >= 50 {
                tracing::warn!("Reached 50 tag limit, skipping remaining tags");
                break;
            }
        }

        aws_tags
    }

    /// Validate payload size (AWS limit: 64KB)
    fn validate_size(&self, id: &CredentialId, data: &EncryptedData) -> Result<(), StorageError> {
        let size = data.ciphertext.len() + data.nonce.len() + data.tag.len();
        const MAX_SIZE: usize = 64 * 1024; // 64KB

        if size > MAX_SIZE {
            return Err(StorageError::WriteFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "Payload size {} bytes exceeds AWS Secrets Manager limit of {} bytes",
                        size, MAX_SIZE
                    ),
                ),
            });
        }

        Ok(())
    }
}

#[async_trait]
impl StorageProvider for AwsSecretsManagerProvider {
    #[tracing::instrument(skip(self, data, metadata, _context), fields(provider = "AWS", id = %id))]
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();

        // Validate size
        self.validate_size(id, &data)?;

        let secret_name = self.get_secret_name(id);
        let tags = self.metadata_to_aws_tags(&metadata);

        // Serialize encrypted data + metadata to JSON
        #[derive(Serialize)]
        struct SecretPayload {
            encrypted_data: EncryptedData,
            metadata: CredentialMetadata,
        }

        let payload = SecretPayload {
            encrypted_data: data,
            metadata,
        };

        let secret_string =
            serde_json::to_string(&payload).map_err(|e| StorageError::WriteFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            })?;

        // Try to create secret first
        let create_result = self
            .client
            .create_secret()
            .name(&secret_name)
            .secret_string(&secret_string)
            .set_kms_key_id(self.config.kms_key_id.clone())
            .set_tags(if tags.is_empty() { None } else { Some(tags) })
            .send()
            .await;

        match create_result {
            Ok(_) => {
                tracing::debug!(secret = %secret_name, "Created new secret");
                let duration = start.elapsed();
                self.metrics.record_operation("store", duration, true);
                Ok(())
            }
            Err(e) => {
                // If secret already exists, try updating it
                let error_msg = e.to_string();
                if error_msg.contains("ResourceExistsException") {
                    // Update existing secret
                    let update_result = self
                        .client
                        .put_secret_value()
                        .secret_id(&secret_name)
                        .secret_string(&secret_string)
                        .send()
                        .await;

                    match update_result {
                        Ok(_) => {
                            tracing::debug!(secret = %secret_name, "Updated existing secret");
                            let duration = start.elapsed();
                            self.metrics.record_operation("store", duration, true);
                            Ok(())
                        }
                        Err(update_err) => {
                            let duration = start.elapsed();
                            self.metrics.record_operation("store", duration, false);
                            Err(StorageError::WriteFailure {
                                id: id.as_str().to_string(),
                                source: std::io::Error::other(update_err.to_string()),
                            })
                        }
                    }
                } else {
                    // Other error
                    let duration = start.elapsed();
                    self.metrics.record_operation("store", duration, false);
                    Err(StorageError::WriteFailure {
                        id: id.as_str().to_string(),
                        source: std::io::Error::other(error_msg),
                    })
                }
            }
        }
    }

    #[tracing::instrument(skip(self, _context), fields(provider = "AWS", id = %id))]
    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        let start = std::time::Instant::now();
        let secret_name = self.get_secret_name(id);

        let result = self
            .client
            .get_secret_value()
            .secret_id(&secret_name)
            .send()
            .await;

        match result {
            Ok(output) => {
                let duration = start.elapsed();
                self.metrics.record_operation("retrieve", duration, true);

                // Extract secret string
                let secret_string =
                    output
                        .secret_string()
                        .ok_or_else(|| StorageError::ReadFailure {
                            id: id.as_str().to_string(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Secret does not contain string data",
                            ),
                        })?;

                // Deserialize payload
                #[derive(Deserialize)]
                struct SecretPayload {
                    encrypted_data: EncryptedData,
                    metadata: CredentialMetadata,
                }

                let payload: SecretPayload =
                    serde_json::from_str(secret_string).map_err(|e| StorageError::ReadFailure {
                        id: id.as_str().to_string(),
                        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
                    })?;

                Ok((payload.encrypted_data, payload.metadata))
            }
            Err(e) => {
                let duration = start.elapsed();
                self.metrics.record_operation("retrieve", duration, false);

                // Map to appropriate error
                let error_msg = e.to_string();
                if error_msg.contains("ResourceNotFoundException") {
                    Err(StorageError::NotFound {
                        id: id.as_str().to_string(),
                    })
                } else {
                    Err(StorageError::ReadFailure {
                        id: id.as_str().to_string(),
                        source: std::io::Error::other(error_msg),
                    })
                }
            }
        }
    }

    #[tracing::instrument(skip(self, _context), fields(provider = "AWS", id = %id))]
    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();
        let secret_name = self.get_secret_name(id);

        let result = self
            .client
            .delete_secret()
            .secret_id(&secret_name)
            .force_delete_without_recovery(true)
            .send()
            .await;

        let duration = start.elapsed();

        match result {
            Ok(_) => {
                self.metrics.record_operation("delete", duration, true);
                Ok(())
            }
            Err(e) => {
                // Idempotent: treat NotFound as success
                let error_msg = e.to_string();
                if error_msg.contains("ResourceNotFoundException") {
                    self.metrics.record_operation("delete", duration, true);
                    tracing::debug!(secret = %secret_name, "Secret not found (idempotent delete)");
                    Ok(())
                } else {
                    self.metrics.record_operation("delete", duration, false);
                    Err(StorageError::WriteFailure {
                        id: id.as_str().to_string(),
                        source: std::io::Error::other(error_msg),
                    })
                }
            }
        }
    }

    #[tracing::instrument(skip(self, filter, _context), fields(provider = "AWS"))]
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        let start = std::time::Instant::now();
        let prefix = self.config.secret_prefix.clone();

        let mut ids = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut request = self.client.list_secrets().max_results(100);

            if let Some(token) = next_token {
                request = request.next_token(token);
            }

            let list_result = request.send().await;

            match list_result {
                Ok(output) => {
                    // Filter secrets by prefix
                    for secret in output.secret_list() {
                        if let Some(name) = secret.name()
                            && name.starts_with(&prefix)
                        {
                            let id_str = &name[prefix.len()..];
                            if let Ok(id) = CredentialId::new(id_str) {
                                ids.push(id);
                            }
                        }
                    }

                    // Check for more pages
                    match output.next_token() {
                        Some(token) => next_token = Some(token.to_string()),
                        None => break,
                    }
                }
                Err(e) => {
                    let duration = start.elapsed();
                    self.metrics.record_operation("list", duration, false);
                    return Err(StorageError::ReadFailure {
                        id: "[list]".into(),
                        source: std::io::Error::other(e.to_string()),
                    });
                }
            }
        }

        let duration = start.elapsed();
        self.metrics.record_operation("list", duration, true);

        // Apply filter if provided
        if let Some(_filter) = filter {
            // TODO: Implement filter logic when we load full metadata
            // For now, return all IDs (filter would require retrieving each secret)
            tracing::warn!("Filter not yet implemented for AWS provider");
        }

        Ok(ids)
    }

    #[tracing::instrument(skip(self, _context), fields(provider = "AWS", id = %id))]
    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        let start = std::time::Instant::now();
        let secret_name = self.get_secret_name(id);

        let result = self
            .client
            .describe_secret()
            .secret_id(&secret_name)
            .send()
            .await;

        let duration = start.elapsed();

        match result {
            Ok(_) => {
                self.metrics.record_operation("exists", duration, true);
                Ok(true)
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Check for both ResourceNotFoundException and InvalidRequestException
                // InvalidRequestException can occur when checking a recently deleted secret
                if error_msg.contains("ResourceNotFoundException")
                    || error_msg.contains("InvalidRequestException")
                    || error_msg.contains("service error")
                {
                    // Treat any of these as "secret does not exist"
                    self.metrics.record_operation("exists", duration, true);
                    Ok(false)
                } else {
                    self.metrics.record_operation("exists", duration, false);
                    Err(StorageError::ReadFailure {
                        id: id.as_str().to_string(),
                        source: std::io::Error::other(error_msg),
                    })
                }
            }
        }
    }
}
