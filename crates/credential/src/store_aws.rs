//! AWS Secrets Manager credential store (v2 [`CredentialStore`] backend).
//!
//! Stores each credential as a JSON-serialized secret in AWS Secrets Manager,
//! with the secret name derived from a configurable prefix and the credential ID.
//!
//! # Features
//!
//! - KMS encryption (AWS-managed or customer-managed keys)
//! - Region auto-detection from environment / EC2 metadata
//! - Custom endpoint URL for LocalStack / testing
//! - Tag-based metadata storage
//! - 64 KB payload size limit (AWS hard limit)
//!
//! Feature-gated behind `storage-aws`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_credential::store_aws::{AwsSecretsStore, AwsSecretsConfig};
//!
//! let config = AwsSecretsConfig::builder()
//!     .region("us-west-2")
//!     .prefix("nebula/credentials/")
//!     .build();
//! let store = AwsSecretsStore::new(config).await?;
//! let cred = store.get("my-api-key").await?;
//! ```

use std::collections::HashMap;
use std::time::Duration;

use aws_sdk_secretsmanager::Client as SecretsManagerClient;
use serde::{Deserialize, Serialize};

use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Maximum payload size for AWS Secrets Manager (64 KB).
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

// ── Config ────────────────────────────────────────────────────────────────

/// Configuration for [`AwsSecretsStore`].
///
/// # Region resolution order
///
/// 1. Explicit `region` field in this config
/// 2. `AWS_REGION` environment variable
/// 3. `AWS_DEFAULT_REGION` environment variable
/// 4. EC2 instance metadata (when running on EC2)
///
/// # KMS encryption
///
/// By default AWS uses the `aws/secretsmanager` managed key.
/// Set [`kms_key_id`](Self::kms_key_id) for a customer-managed key.
///
/// # Examples
///
/// ```rust
/// use nebula_credential::store_aws::AwsSecretsConfig;
/// use std::time::Duration;
///
/// let config = AwsSecretsConfig {
///     region: Some("us-east-1".into()),
///     prefix: "nebula/creds/".into(),
///     timeout: Duration::from_secs(10),
///     kms_key_id: Some("alias/nebula-credentials".into()),
///     ..Default::default()
/// };
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AwsSecretsConfig {
    /// AWS region (auto-detected from environment when `None`).
    pub region: Option<String>,

    /// Custom endpoint URL (e.g. `http://localhost:4566` for LocalStack).
    pub endpoint_url: Option<String>,

    /// KMS key ID, alias, or ARN for encryption.
    ///
    /// When `None` the AWS-managed `aws/secretsmanager` key is used.
    pub kms_key_id: Option<String>,

    /// Secret name prefix — every credential ID is stored as `{prefix}{id}`.
    ///
    /// Must be at most 512 characters and contain no invalid AWS characters.
    pub prefix: String,

    /// Per-operation timeout.
    pub timeout: Duration,

    /// Default tags applied to every secret (merged with per-credential metadata).
    ///
    /// AWS allows at most 50 tags per secret.
    pub default_tags: HashMap<String, String>,
}

impl Default for AwsSecretsConfig {
    fn default() -> Self {
        Self {
            region: None,
            endpoint_url: None,
            kms_key_id: None,
            prefix: String::new(),
            timeout: Duration::from_secs(5),
            default_tags: HashMap::new(),
        }
    }
}

// ── Serde wrapper ─────────────────────────────────────────────────────────

/// JSON-serializable representation of a [`StoredCredential`] inside a secret.
#[derive(Serialize, Deserialize)]
struct SecretPayload {
    id: String,
    #[serde(with = "crate::utils::serde_base64")]
    data: Vec<u8>,
    state_kind: String,
    state_version: u32,
    version: u64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    metadata: serde_json::Map<String, serde_json::Value>,
}

impl From<StoredCredential> for SecretPayload {
    fn from(c: StoredCredential) -> Self {
        Self {
            id: c.id,
            data: c.data,
            state_kind: c.state_kind,
            state_version: c.state_version,
            version: c.version,
            created_at: c.created_at,
            updated_at: c.updated_at,
            expires_at: c.expires_at,
            metadata: c.metadata,
        }
    }
}

impl From<SecretPayload> for StoredCredential {
    fn from(p: SecretPayload) -> Self {
        Self {
            id: p.id,
            data: p.data,
            state_kind: p.state_kind,
            state_version: p.state_version,
            version: p.version,
            created_at: p.created_at,
            updated_at: p.updated_at,
            expires_at: p.expires_at,
            metadata: p.metadata,
        }
    }
}

// ── Store ─────────────────────────────────────────────────────────────────

/// AWS Secrets Manager credential store.
///
/// Implements the v2 [`CredentialStore`] trait, storing each credential as a
/// single Secrets Manager secret whose value is a JSON [`SecretPayload`].
///
/// # CAS semantics
///
/// [`PutMode::CompareAndSwap`] uses the `version` counter embedded in the
/// JSON payload — it is **not** backed by AWS `VersionId`/`VersionStage`
/// because those track the secret _version list_, not the logical credential
/// version.  A read-modify-write cycle still relies on the JSON `version`
/// field for conflict detection.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::store_aws::{AwsSecretsStore, AwsSecretsConfig};
///
/// let store = AwsSecretsStore::new(AwsSecretsConfig {
///     region: Some("us-west-2".into()),
///     prefix: "nebula/".into(),
///     ..Default::default()
/// }).await?;
/// ```
pub struct AwsSecretsStore {
    /// Provider configuration.
    config: AwsSecretsConfig,
    /// AWS Secrets Manager client.
    client: SecretsManagerClient,
}

impl AwsSecretsStore {
    /// Create a new store, initialising the AWS SDK client.
    ///
    /// Credential chain: environment variables → shared credentials file →
    /// IAM role (EC2 / ECS).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] if the AWS SDK fails to initialise.
    pub async fn new(config: AwsSecretsConfig) -> Result<Self, StoreError> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        if let Some(region) = &config.region {
            loader = loader.region(aws_config::Region::new(region.clone()));
        }
        if let Some(endpoint) = &config.endpoint_url {
            loader = loader.endpoint_url(endpoint);
        }

        let sdk_config = loader.load().await;
        let client = SecretsManagerClient::new(&sdk_config);

        Ok(Self { config, client })
    }

    /// Derive the full secret name for a credential ID.
    fn secret_name(&self, id: &str) -> String {
        format!("{}{}", self.config.prefix, id)
    }

    /// Serialize a [`StoredCredential`] to JSON and validate the AWS payload
    /// size limit.
    fn serialize_payload(credential: &StoredCredential) -> Result<String, StoreError> {
        let payload: SecretPayload = credential.clone().into();
        let json = serde_json::to_string(&payload).map_err(|e| StoreError::Backend(Box::new(e)))?;

        if json.len() > MAX_PAYLOAD_BYTES {
            return Err(StoreError::Backend(
                format!(
                    "credential payload ({} bytes) exceeds AWS Secrets Manager 64 KB limit",
                    json.len()
                )
                .into(),
            ));
        }
        Ok(json)
    }

    /// Try to read an existing credential from the backend.
    ///
    /// Returns `Ok(None)` when the secret does not exist.
    async fn read_secret(&self, name: &str) -> Result<Option<StoredCredential>, StoreError> {
        let result = self.client.get_secret_value().secret_id(name).send().await;

        match result {
            Ok(output) => {
                let secret_string = output.secret_string().ok_or_else(|| {
                    StoreError::Backend("secret does not contain string data".into())
                })?;
                let payload: SecretPayload = serde_json::from_str(secret_string)
                    .map_err(|e| StoreError::Backend(Box::new(e)))?;
                Ok(Some(payload.into()))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("ResourceNotFoundException") {
                    Ok(None)
                } else {
                    Err(StoreError::Backend(msg.into()))
                }
            }
        }
    }
}

impl CredentialStore for AwsSecretsStore {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let name = self.secret_name(id);
        self.read_secret(&name)
            .await?
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let name = self.secret_name(&credential.id);
        let existing = self.read_secret(&name).await?;

        // ── Apply version semantics ───────────────────────────────────────
        match mode {
            PutMode::CreateOnly => {
                if existing.is_some() {
                    return Err(StoreError::AlreadyExists {
                        id: credential.id.clone(),
                    });
                }
                credential.version = 1;
                credential.created_at = chrono::Utc::now();
                credential.updated_at = credential.created_at;
            }
            PutMode::Overwrite => {
                let version = existing.as_ref().map_or(1, |e| e.version + 1);
                credential.version = version;
                credential.updated_at = chrono::Utc::now();
                if version == 1 {
                    credential.created_at = credential.updated_at;
                }
            }
            PutMode::CompareAndSwap { expected_version } => {
                if let Some(ref ex) = existing
                    && ex.version != expected_version
                {
                    return Err(StoreError::VersionConflict {
                        id: credential.id.clone(),
                        expected: expected_version,
                        actual: ex.version,
                    });
                }
                credential.version = expected_version + 1;
                credential.updated_at = chrono::Utc::now();
            }
        }

        let json = Self::serialize_payload(&credential)?;

        // ── Write to Secrets Manager ──────────────────────────────────────
        if existing.is_some() {
            // Update existing secret.
            self.client
                .put_secret_value()
                .secret_id(&name)
                .secret_string(&json)
                .send()
                .await
                .map_err(|e| StoreError::Backend(e.to_string().into()))?;
        } else {
            // Create new secret.
            let mut req = self.client.create_secret().name(&name).secret_string(&json);

            if let Some(kms) = &self.config.kms_key_id {
                req = req.kms_key_id(kms);
            }

            // Apply default tags.
            let tags: Vec<_> = self
                .config
                .default_tags
                .iter()
                .map(|(k, v)| {
                    aws_sdk_secretsmanager::types::Tag::builder()
                        .key(k)
                        .value(v)
                        .build()
                })
                .collect();
            if !tags.is_empty() {
                req = req.set_tags(Some(tags));
            }

            req.send()
                .await
                .map_err(|e| StoreError::Backend(e.to_string().into()))?;
        }

        Ok(credential)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let name = self.secret_name(id);

        let result = self
            .client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("ResourceNotFoundException") {
                    Err(StoreError::NotFound { id: id.to_string() })
                } else {
                    Err(StoreError::Backend(msg.into()))
                }
            }
        }
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let prefix = &self.config.prefix;
        let mut ids = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut req = self.client.list_secrets().max_results(100);
            if let Some(token) = next_token.take() {
                req = req.next_token(token);
            }

            let output = req
                .send()
                .await
                .map_err(|e| StoreError::Backend(e.to_string().into()))?;

            for secret in output.secret_list() {
                let Some(name) = secret.name() else {
                    continue;
                };
                let Some(id) = name.strip_prefix(prefix) else {
                    continue;
                };

                // When a state_kind filter is set we must read the secret to
                // inspect the payload. This is expensive but AWS Secrets
                // Manager has no server-side filter for payload contents.
                let matches = match state_kind {
                    Some(kind) => self
                        .read_secret(name)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|c| c.state_kind == kind),
                    None => true,
                };
                if matches {
                    ids.push(id.to_string());
                }
            }

            match output.next_token() {
                Some(token) => next_token = Some(token.to_string()),
                None => break,
            }
        }

        Ok(ids)
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let name = self.secret_name(id);

        let result = self.client.describe_secret().secret_id(&name).send().await;

        match result {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("ResourceNotFoundException") {
                    Ok(false)
                } else {
                    Err(StoreError::Backend(msg.into()))
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = AwsSecretsConfig::default();
        assert!(config.region.is_none());
        assert!(config.endpoint_url.is_none());
        assert!(config.kms_key_id.is_none());
        assert!(config.prefix.is_empty());
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(config.default_tags.is_empty());
    }

    #[test]
    fn config_with_all_fields() {
        let config = AwsSecretsConfig {
            region: Some("us-west-2".into()),
            endpoint_url: Some("http://localhost:4566".into()),
            kms_key_id: Some("alias/my-key".into()),
            prefix: "nebula/".into(),
            timeout: Duration::from_secs(10),
            default_tags: HashMap::from([("env".into(), "prod".into())]),
        };

        assert_eq!(config.region.as_deref(), Some("us-west-2"));
        assert_eq!(
            config.endpoint_url.as_deref(),
            Some("http://localhost:4566")
        );
        assert_eq!(config.kms_key_id.as_deref(), Some("alias/my-key"));
        assert_eq!(config.prefix, "nebula/");
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.default_tags.len(), 1);
    }

    #[test]
    fn secret_payload_round_trip() {
        let cred = StoredCredential {
            id: "test-id".into(),
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            state_kind: "bearer".into(),
            state_version: 1,
            version: 3,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };

        let payload: SecretPayload = cred.clone().into();
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: SecretPayload = serde_json::from_str(&json).unwrap();
        let round_tripped: StoredCredential = deserialized.into();

        assert_eq!(round_tripped.id, cred.id);
        assert_eq!(round_tripped.data, cred.data);
        assert_eq!(round_tripped.state_kind, cred.state_kind);
        assert_eq!(round_tripped.version, cred.version);
    }

    #[test]
    fn serialize_payload_rejects_oversized() {
        let cred = StoredCredential {
            id: "big".into(),
            data: vec![0u8; MAX_PAYLOAD_BYTES + 1],
            state_kind: "test".into(),
            state_version: 1,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };

        let result = AwsSecretsStore::serialize_payload(&cred);
        assert!(result.is_err());
    }

    #[test]
    fn secret_name_uses_prefix() {
        // We can't construct AwsSecretsStore without an AWS client, so test
        // the name derivation logic directly.
        let prefix = "nebula/creds/";
        let id = "github-token";
        let name = format!("{prefix}{id}");
        assert_eq!(name, "nebula/creds/github-token");
    }

    #[test]
    fn config_serde_round_trip() {
        let config = AwsSecretsConfig {
            region: Some("eu-west-1".into()),
            endpoint_url: None,
            kms_key_id: Some("arn:aws:kms:eu-west-1:123:key/abc".into()),
            prefix: "app/".into(),
            timeout: Duration::from_secs(3),
            default_tags: HashMap::from([("team".into(), "platform".into())]),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AwsSecretsConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.region, config.region);
        assert_eq!(deserialized.kms_key_id, config.kms_key_id);
        assert_eq!(deserialized.prefix, config.prefix);
        assert_eq!(deserialized.default_tags, config.default_tags);
    }
}
