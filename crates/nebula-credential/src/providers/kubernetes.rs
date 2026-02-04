//! Kubernetes Secrets storage provider
//!
//! Implements credential storage using Kubernetes Secrets with namespace isolation.
//!
//! # Features
//!
//! - Namespace-isolated secrets
//! - Label-based filtering and organization
//! - Annotations for rich metadata
//! - RBAC integration
//! - 1MB payload size limit (K8s limit)
//!
//! # Configuration
//!
//! ```rust,ignore
//! use nebula_credential::providers::{KubernetesSecretsConfig, KubernetesSecretsProvider};
//! use std::time::Duration;
//!
//! let config = KubernetesSecretsConfig {
//!     namespace: "default".into(),
//!     secret_prefix: "nebula-".into(),
//!     timeout: Duration::from_secs(5),
//!     ..Default::default()
//! };
//!
//! let provider = KubernetesSecretsProvider::new(config).await?;
//! ```

use crate::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use crate::providers::{ProviderConfig, StorageMetrics};
use crate::traits::StorageProvider;
use crate::utils::{EncryptedData, RetryPolicy, validate_encrypted_size};
use async_trait::async_trait;
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams};
use kube::{Api, Client};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Kubernetes Secrets configuration
///
/// # Size Limits
///
/// Kubernetes has a 1MB limit per Secret. Credentials exceeding this
/// limit will be rejected with `StorageError::WriteFailure`.
///
/// # Namespace Isolation
///
/// Each provider instance operates within a single Kubernetes namespace.
/// Secrets in different namespaces are completely isolated.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KubernetesSecretsConfig {
    /// Kubernetes namespace for secrets
    pub namespace: String,

    /// Path to kubeconfig file (optional, uses in-cluster config if None)
    pub kubeconfig_path: Option<PathBuf>,

    /// Secret name prefix for namespacing
    pub secret_prefix: String,

    /// Operation timeout
    pub timeout: Duration,

    /// Retry policy for transient failures
    pub retry_policy: RetryPolicy,

    /// Default labels applied to all secrets
    pub default_labels: HashMap<String, String>,

    /// Default annotations applied to all secrets
    pub default_annotations: HashMap<String, String>,

    /// Accept invalid/self-signed TLS certificates (for testing only)
    pub accept_invalid_certs: bool,
}

impl Default for KubernetesSecretsConfig {
    fn default() -> Self {
        Self {
            namespace: "default".into(),
            kubeconfig_path: None,
            secret_prefix: String::new(),
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            default_labels: HashMap::new(),
            default_annotations: HashMap::new(),
            accept_invalid_certs: false,
        }
    }
}

impl ProviderConfig for KubernetesSecretsConfig {
    fn validate(&self) -> Result<(), crate::providers::config::ConfigError> {
        use crate::providers::config::ConfigError;

        // Validate namespace format
        if self.namespace.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "namespace".into(),
                reason: "cannot be empty".into(),
            });
        }

        if self.namespace.len() > 63 {
            return Err(ConfigError::InvalidValue {
                field: "namespace".into(),
                reason: format!("exceeds 63 character limit ({})", self.namespace.len()),
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
        "KubernetesSecrets"
    }
}

/// Kubernetes Secrets storage provider
#[derive(Clone)]
pub struct KubernetesSecretsProvider {
    /// API handle for Secrets in the configured namespace
    secrets_api: Api<Secret>,

    /// Provider configuration
    config: KubernetesSecretsConfig,

    /// Metrics collection
    metrics: Arc<StorageMetrics>,
}

impl std::fmt::Debug for KubernetesSecretsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubernetesSecretsProvider")
            .field("namespace", &self.config.namespace)
            .field("config", &self.config)
            .finish()
    }
}

impl KubernetesSecretsProvider {
    /// Create a new Kubernetes Secrets provider
    pub async fn new(config: KubernetesSecretsConfig) -> Result<Self, StorageError> {
        // Validate configuration
        config.validate().map_err(|e| StorageError::WriteFailure {
            id: "[config]".into(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
        })?;

        // Initialize Kubernetes client
        let client = if let Some(path) = &config.kubeconfig_path {
            // Use specified kubeconfig path
            let kubeconfig_content =
                tokio::fs::read_to_string(path)
                    .await
                    .map_err(|e| StorageError::WriteFailure {
                        id: "[k8s_client]".into(),
                        source: std::io::Error::other(format!(
                            "Failed to read kubeconfig from {:?}: {}",
                            path, e
                        )),
                    })?;

            let mut kubeconfig = kube::Config::from_custom_kubeconfig(
                kube::config::Kubeconfig::from_yaml(&kubeconfig_content).map_err(|e| {
                    StorageError::WriteFailure {
                        id: "[k8s_client]".into(),
                        source: std::io::Error::other(format!("Failed to parse kubeconfig: {}", e)),
                    }
                })?,
                &kube::config::KubeConfigOptions::default(),
            )
            .await
            .map_err(|e| StorageError::WriteFailure {
                id: "[k8s_client]".into(),
                source: std::io::Error::other(format!("Failed to load kubeconfig: {}", e)),
            })?;

            // Enable insecure mode if configured (for testing with self-signed certs)
            kubeconfig.accept_invalid_certs = config.accept_invalid_certs;

            Client::try_from(kubeconfig).map_err(|e| StorageError::WriteFailure {
                id: "[k8s_client]".into(),
                source: std::io::Error::other(format!("Failed to create K8s client: {}", e)),
            })?
        } else {
            // Try in-cluster first, fall back to default kubeconfig
            Client::try_default()
                .await
                .map_err(|e| StorageError::WriteFailure {
                    id: "[k8s_client]".into(),
                    source: std::io::Error::other(format!("Failed to create K8s client: {}", e)),
                })?
        };

        // Create API handle for Secrets in the configured namespace
        let secrets_api: Api<Secret> = Api::namespaced(client.clone(), &config.namespace);

        tracing::info!(
            provider = "Kubernetes Secrets",
            namespace = %config.namespace,
            prefix = %config.secret_prefix,
            "Initialized Kubernetes Secrets provider"
        );

        Ok(Self {
            secrets_api,
            config,
            metrics: Arc::new(StorageMetrics::default()),
        })
    }

    /// Get full secret name by prefixing credential ID
    fn get_secret_name(&self, id: &CredentialId) -> String {
        format!("{}{}", self.config.secret_prefix, id.as_str())
            .to_lowercase()
            .replace('_', "-")
    }

    /// Convert credential metadata to Kubernetes labels
    fn metadata_to_labels(&self, metadata: &CredentialMetadata) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();

        // Add default labels
        for (key, value) in &self.config.default_labels {
            labels.insert(Self::sanitize_label(key), Self::sanitize_label_value(value));
        }

        // Add tags as labels with "tag/" prefix
        for (tag_key, tag_value) in &metadata.tags {
            let label_key = format!("tag/{}", Self::sanitize_label(tag_key));
            labels.insert(label_key, Self::sanitize_label_value(tag_value));
        }

        labels
    }

    /// Convert credential metadata to Kubernetes annotations
    fn metadata_to_annotations(&self, metadata: &CredentialMetadata) -> BTreeMap<String, String> {
        let mut annotations = BTreeMap::new();

        // Add default annotations
        for (key, value) in &self.config.default_annotations {
            annotations.insert(key.clone(), value.clone());
        }

        // Store timestamps
        annotations.insert(
            "nebula.credential.created-at".to_string(),
            metadata.created_at.to_rfc3339(),
        );

        annotations.insert(
            "nebula.credential.modified-at".to_string(),
            metadata.last_modified.to_rfc3339(),
        );

        if let Some(accessed) = metadata.last_accessed {
            annotations.insert(
                "nebula.credential.accessed-at".to_string(),
                accessed.to_rfc3339(),
            );
        }

        // Store tags as JSON
        if !metadata.tags.is_empty()
            && let Ok(tags_json) = serde_json::to_string(&metadata.tags)
        {
            annotations.insert("nebula.credential.tags".to_string(), tags_json);
        }

        annotations
    }

    /// Sanitize a string to be a valid Kubernetes label key
    pub(crate) fn sanitize_label(input: &str) -> String {
        let mut sanitized = input
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();

        // Truncate to 63 characters
        if sanitized.len() > 63 {
            sanitized.truncate(63);
        }

        // Ensure starts and ends with alphanumeric
        sanitized = sanitized
            .trim_matches(|c: char| !c.is_ascii_alphanumeric())
            .to_string();

        // If empty after sanitization, use placeholder
        if sanitized.is_empty() {
            sanitized = "unknown".to_string();
        }

        sanitized
    }

    /// Sanitize a string to be a valid Kubernetes label value
    pub(crate) fn sanitize_label_value(input: &str) -> String {
        let mut sanitized = input
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();

        // Truncate to 63 characters
        if sanitized.len() > 63 {
            sanitized.truncate(63);
        }

        // Ensure starts and ends with alphanumeric (or empty)
        sanitized = sanitized
            .trim_matches(|c: char| !c.is_ascii_alphanumeric())
            .to_string();

        sanitized
    }
}

#[async_trait]
impl StorageProvider for KubernetesSecretsProvider {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let secret_name = self.get_secret_name(id);
        let start = std::time::Instant::now();

        // Validate size (Kubernetes limit: 1MB)
        validate_encrypted_size(id, &data, 1_000_000, "Kubernetes")?;

        // Serialize encrypted data to JSON
        let data_json = serde_json::to_vec(&data).map_err(|e| StorageError::WriteFailure {
            id: id.as_str().to_string(),
            source: std::io::Error::other(format!("Failed to serialize credential: {}", e)),
        })?;

        // Convert metadata to labels and annotations
        let labels = self.metadata_to_labels(&metadata);
        let annotations = self.metadata_to_annotations(&metadata);

        // Create Secret object
        let mut secret_data = BTreeMap::new();
        secret_data.insert("credential".to_string(), ByteString(data_json));

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.clone()),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(labels),
                annotations: Some(annotations),
                ..Default::default()
            },
            data: Some(secret_data),
            type_: Some("Opaque".to_string()),
            ..Default::default()
        };

        // Store or update secret using server-side apply
        let patch_params = PatchParams::apply("nebula-credential");
        let patch = Patch::Apply(&secret);

        self.secrets_api
            .patch(&secret_name, &patch_params, &patch)
            .await
            .map_err(|e| StorageError::WriteFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other(format!("Failed to store secret: {}", e)),
            })?;

        self.metrics
            .record_operation("store", start.elapsed(), true);

        tracing::debug!(
            secret_name = %secret_name,
            namespace = %self.config.namespace,
            "Stored credential in Kubernetes Secret"
        );

        Ok(())
    }

    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        let secret_name = self.get_secret_name(id);
        let start = std::time::Instant::now();

        // Retrieve secret
        let secret = self.secrets_api.get(&secret_name).await.map_err(|e| {
            if e.to_string().contains("NotFound") {
                StorageError::NotFound {
                    id: id.as_str().to_string(),
                }
            } else {
                StorageError::ReadFailure {
                    id: id.as_str().to_string(),
                    source: std::io::Error::other(format!("Failed to retrieve secret: {}", e)),
                }
            }
        })?;

        // Extract credential data
        let data = secret.data.ok_or_else(|| StorageError::ReadFailure {
            id: id.as_str().to_string(),
            source: std::io::Error::other("Secret has no data field"),
        })?;

        let credential_bytes = data
            .get("credential")
            .ok_or_else(|| StorageError::ReadFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other("Secret missing 'credential' key"),
            })?;

        // Deserialize encrypted data
        let encrypted_data: EncryptedData =
            serde_json::from_slice(&credential_bytes.0).map_err(|e| StorageError::ReadFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other(format!("Failed to deserialize credential: {}", e)),
            })?;

        // Extract metadata from annotations
        let annotations =
            secret
                .metadata
                .annotations
                .as_ref()
                .ok_or_else(|| StorageError::ReadFailure {
                    id: id.as_str().to_string(),
                    source: std::io::Error::other("Secret missing annotations"),
                })?;

        // Parse timestamps
        let created_at = annotations
            .get("nebula.credential.created-at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok_or_else(|| StorageError::ReadFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other("Invalid or missing created-at timestamp"),
            })?;

        let last_modified = annotations
            .get("nebula.credential.modified-at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok_or_else(|| StorageError::ReadFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other("Invalid or missing modified-at timestamp"),
            })?;

        let last_accessed = annotations
            .get("nebula.credential.accessed-at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        // Parse tags from JSON
        let tags = annotations
            .get("nebula.credential.tags")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        // Build metadata
        let metadata = CredentialMetadata {
            created_at,
            last_accessed,
            last_modified,
            rotation_policy: None,
            tags,
        };

        self.metrics
            .record_operation("retrieve", start.elapsed(), true);

        tracing::debug!(
            secret_name = %secret_name,
            namespace = %self.config.namespace,
            "Retrieved credential from Kubernetes Secret"
        );

        Ok((encrypted_data, metadata))
    }

    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let secret_name = self.get_secret_name(id);
        let start = std::time::Instant::now();

        // Delete secret (idempotent)
        match self
            .secrets_api
            .delete(&secret_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {
                self.metrics
                    .record_operation("delete", start.elapsed(), true);

                tracing::debug!(
                    secret_name = %secret_name,
                    namespace = %self.config.namespace,
                    "Deleted credential from Kubernetes Secret"
                );

                Ok(())
            }
            Err(e) if e.to_string().contains("NotFound") => {
                // Idempotent - deleting non-existent secret succeeds
                self.metrics
                    .record_operation("delete", start.elapsed(), true);
                Ok(())
            }
            Err(e) => Err(StorageError::WriteFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other(format!("Failed to delete secret: {}", e)),
            }),
        }
    }

    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        let secret_name = self.get_secret_name(id);
        let start = std::time::Instant::now();

        // Check if secret exists
        let result = self.secrets_api.get(&secret_name).await;

        self.metrics
            .record_operation("exists", start.elapsed(), true);

        match result {
            Ok(_) => Ok(true),
            Err(e) if e.to_string().contains("NotFound") => Ok(false),
            Err(e) => Err(StorageError::ReadFailure {
                id: id.as_str().to_string(),
                source: std::io::Error::other(format!("Failed to check secret existence: {}", e)),
            }),
        }
    }

    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        let start = std::time::Instant::now();

        // Build label selector from filter
        let mut label_selectors = Vec::new();

        if let Some(filter) = filter
            && let Some(tags) = &filter.tags
        {
            for (tag_key, tag_value) in tags.iter() {
                label_selectors.push(format!(
                    "tag/{}={}",
                    Self::sanitize_label(tag_key),
                    Self::sanitize_label_value(tag_value)
                ));
            }
        }

        let label_selector = if label_selectors.is_empty() {
            None
        } else {
            Some(label_selectors.join(","))
        };

        // List secrets with label selector
        let list_params = if let Some(selector) = label_selector {
            ListParams::default().labels(&selector)
        } else {
            ListParams::default()
        };

        let secrets =
            self.secrets_api
                .list(&list_params)
                .await
                .map_err(|e| StorageError::ReadFailure {
                    id: "[list]".to_string(),
                    source: std::io::Error::other(format!("Failed to list secrets: {}", e)),
                })?;

        // Extract credential IDs from secret names
        let mut ids = Vec::new();
        let prefix = &self.config.secret_prefix;

        for secret in secrets.items {
            if let Some(name) = secret.metadata.name {
                // Remove prefix and convert back to credential ID format
                if let Some(id_str) = name.strip_prefix(prefix) {
                    // Convert kebab-case back to snake_case
                    let id_str = id_str.replace('-', "_");
                    if let Ok(id) = CredentialId::new(&id_str) {
                        // Apply additional filters
                        if let Some(filter) = filter
                            && let Some(annotations) = &secret.metadata.annotations
                        {
                            // Filter by created_after
                            if let Some(created_after) = filter.created_after
                                && let Some(created_str) =
                                    annotations.get("nebula.credential.created-at")
                                && let Ok(created) =
                                    chrono::DateTime::parse_from_rfc3339(created_str)
                                && created.with_timezone(&chrono::Utc) < created_after
                            {
                                continue;
                            }

                            // Filter by created_before
                            if let Some(created_before) = filter.created_before
                                && let Some(created_str) =
                                    annotations.get("nebula.credential.created-at")
                                && let Ok(created) =
                                    chrono::DateTime::parse_from_rfc3339(created_str)
                                && created.with_timezone(&chrono::Utc) > created_before
                            {
                                continue;
                            }
                        }

                        ids.push(id);
                    }
                }
            }
        }

        self.metrics.record_operation("list", start.elapsed(), true);

        tracing::debug!(
            namespace = %self.config.namespace,
            count = ids.len(),
            filter = ?filter,
            "Listed credentials from Kubernetes Secrets"
        );

        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::super::kubernetes::{KubernetesSecretsConfig, KubernetesSecretsProvider};
    use crate::providers::ProviderConfig;
    use std::time::Duration;

    #[test]
    fn test_config_default() {
        let config = KubernetesSecretsConfig::default();
        assert_eq!(config.namespace, "default");
        assert_eq!(config.secret_prefix, "");
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_empty_namespace() {
        let mut config = KubernetesSecretsConfig::default();
        config.namespace = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_long_namespace() {
        let mut config = KubernetesSecretsConfig::default();
        config.namespace = "a".repeat(64);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_invalid_timeout() {
        let mut config = KubernetesSecretsConfig::default();
        config.timeout = Duration::from_secs(0);
        assert!(config.validate().is_err());

        config.timeout = Duration::from_secs(61);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_provider_name() {
        let config = KubernetesSecretsConfig::default();
        assert_eq!(config.provider_name(), "KubernetesSecrets");
    }

    #[test]
    fn test_sanitize_label() {
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label("Hello_World-123"),
            "hello_world-123"
        );
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label("Invalid@Chars#Here"),
            "invalid-chars-here"
        );
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label(&"a".repeat(100)),
            "a".repeat(63)
        );
        assert_eq!(KubernetesSecretsProvider::sanitize_label(""), "unknown");
        assert_eq!(KubernetesSecretsProvider::sanitize_label("---"), "unknown");
    }

    #[test]
    fn test_sanitize_label_value() {
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label_value("prod-env"),
            "prod-env"
        );
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label_value("Test Value 123!"),
            "test-value-123"
        );
        assert_eq!(
            KubernetesSecretsProvider::sanitize_label_value(&"b".repeat(100)),
            "b".repeat(63)
        );
        assert_eq!(KubernetesSecretsProvider::sanitize_label_value(""), "");
    }
}
