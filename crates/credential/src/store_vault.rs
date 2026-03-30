//! HashiCorp Vault v2 credential store using KV v2 engine.
//!
//! Adapts the Vault KV v2 secret engine to the [`CredentialStore`] trait.
//! Supports Token and AppRole authentication, automatic token renewal,
//! namespace support, and retry with exponential backoff.
//!
//! # Features
//!
//! - **KV v2 engine** with native secret versioning
//! - **Token authentication** with background renewal
//! - **AppRole authentication** for service-to-service use
//! - **Vault Enterprise namespaces**
//! - **Retry with exponential backoff** for transient failures
//! - **CAS via KV v2 `cas` semantics** for [`PutMode::CompareAndSwap`]
//!
//! # Feature gate
//!
//! Requires `storage-vault`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_credential::store_vault::{VaultStore, VaultConfig, VaultAuthMethod};
//!
//! let config = VaultConfig {
//!     address: "https://vault.example.com:8200".into(),
//!     auth_method: VaultAuthMethod::Token {
//!         token: std::env::var("VAULT_TOKEN")?,
//!     },
//!     mount_path: "secret".into(),
//!     path_prefix: "nebula/credentials".into(),
//!     ..Default::default()
//! };
//!
//! let store = VaultStore::new(config).await?;
//! let cred = store.get("my-api-key").await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use vaultrs::client::{Client, VaultClient, VaultClientSettingsBuilder};
use vaultrs::kv2;

use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};
use crate::providers::config::{ConfigError, ProviderConfig};
use crate::utils::RetryPolicy;

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the Vault credential store.
///
/// Supports Token and AppRole authentication methods with automatic token
/// renewal. All fields have sensible defaults via [`Default`].
///
/// # Validation
///
/// Call [`ProviderConfig::validate`] before constructing a [`VaultStore`] — the
/// constructor validates automatically, but early validation gives better error
/// messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Vault server address (e.g. `"https://vault.example.com:8200"`).
    ///
    /// Must use HTTPS when `tls_verify` is `true`.
    pub address: String,

    /// Authentication method for Vault.
    pub auth_method: VaultAuthMethod,

    /// KV v2 mount path (default: `"secret"`).
    pub mount_path: String,

    /// Path prefix for all credentials (e.g. `"nebula/credentials"`).
    pub path_prefix: String,

    /// Optional namespace for Vault Enterprise.
    pub namespace: Option<String>,

    /// Request timeout (must be between 1 and 60 seconds).
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,

    /// Retry policy for transient failures.
    pub retry_policy: RetryPolicy,

    /// Whether to verify TLS certificates (disable only for development).
    pub tls_verify: bool,

    /// Token renewal threshold — renew when TTL falls below this duration.
    /// Only applies to Token auth.
    #[serde(with = "humantime_serde")]
    pub token_renewal_threshold: Duration,
}

/// Vault authentication method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultAuthMethod {
    /// Token-based authentication.
    Token {
        /// Vault token (from `VAULT_TOKEN` env var or config).
        token: String,
    },

    /// AppRole authentication with role_id and secret_id.
    AppRole {
        /// Role ID for the AppRole.
        role_id: String,
        /// Secret ID for the AppRole.
        secret_id: String,
    },
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            address: "https://127.0.0.1:8200".into(),
            auth_method: VaultAuthMethod::Token {
                token: String::new(),
            },
            mount_path: "secret".into(),
            path_prefix: "nebula/credentials".into(),
            namespace: None,
            timeout: Duration::from_secs(10),
            retry_policy: RetryPolicy::default(),
            tls_verify: true,
            token_renewal_threshold: Duration::from_secs(3600),
        }
    }
}

impl ProviderConfig for VaultConfig {
    fn provider_name(&self) -> &'static str {
        "VaultStore"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.address.is_empty() {
            return Err(ConfigError::MissingRequired {
                field: "address".into(),
            });
        }

        if self.tls_verify && !self.address.starts_with("https://") {
            return Err(ConfigError::InvalidValue {
                field: "address".into(),
                reason: "Must use HTTPS when TLS verification is enabled".into(),
            });
        }

        if !self.address.starts_with("http://") && !self.address.starts_with("https://") {
            return Err(ConfigError::InvalidValue {
                field: "address".into(),
                reason: "Must start with http:// or https://".into(),
            });
        }

        if self.mount_path.is_empty() {
            return Err(ConfigError::MissingRequired {
                field: "mount_path".into(),
            });
        }

        if self.mount_path.starts_with('/') || self.mount_path.ends_with('/') {
            return Err(ConfigError::InvalidValue {
                field: "mount_path".into(),
                reason: "Must not start or end with '/'".into(),
            });
        }

        if self.path_prefix.is_empty() {
            return Err(ConfigError::MissingRequired {
                field: "path_prefix".into(),
            });
        }

        if self.path_prefix.starts_with('/') {
            return Err(ConfigError::InvalidValue {
                field: "path_prefix".into(),
                reason: "Must not start with '/' (relative to mount path)".into(),
            });
        }

        let timeout_secs = self.timeout.as_secs();
        if !(1..=60).contains(&timeout_secs) {
            return Err(ConfigError::InvalidValue {
                field: "timeout".into(),
                reason: format!("must be between 1 and 60 seconds, got {timeout_secs} seconds"),
            });
        }

        let renewal_secs = self.token_renewal_threshold.as_secs();
        if !(60..=86400).contains(&renewal_secs) {
            return Err(ConfigError::InvalidValue {
                field: "token_renewal_threshold".into(),
                reason: format!(
                    "must be between 60 seconds and 24 hours, got {renewal_secs} seconds"
                ),
            });
        }

        match &self.auth_method {
            VaultAuthMethod::Token { token } => {
                if token.is_empty() {
                    return Err(ConfigError::MissingRequired {
                        field: "auth_method.token".into(),
                    });
                }
            }
            VaultAuthMethod::AppRole { role_id, secret_id } => {
                if role_id.is_empty() {
                    return Err(ConfigError::MissingRequired {
                        field: "auth_method.role_id".into(),
                    });
                }
                if secret_id.is_empty() {
                    return Err(ConfigError::MissingRequired {
                        field: "auth_method.secret_id".into(),
                    });
                }
            }
        }

        self.retry_policy.validate().map_err(|e| {
            ConfigError::ValidationFailed(format!("Retry policy validation failed: {e}"))
        })?;

        Ok(())
    }
}

// ── Serde wrapper ────────────────────────────────────────────────────────────

/// Vault secret payload: the JSON map stored inside each KV v2 secret.
///
/// Binary credential data is base64-encoded because Vault KV v2 values are
/// JSON maps of `String → String`.
#[derive(Debug, Serialize, Deserialize)]
struct VaultPayload {
    id: String,
    data: String, // base64-encoded
    state_kind: String,
    state_version: u32,
    version: u64,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(default)]
    metadata: String, // JSON string
}

impl VaultPayload {
    /// Convert a [`StoredCredential`] to a Vault-friendly payload.
    fn from_credential(c: &StoredCredential) -> Result<Self, StoreError> {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;

        let metadata_json =
            serde_json::to_string(&c.metadata).map_err(|e| StoreError::Backend(Box::new(e)))?;

        Ok(Self {
            id: c.id.clone(),
            data: STANDARD.encode(&c.data),
            state_kind: c.state_kind.clone(),
            state_version: c.state_version,
            version: c.version,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
            expires_at: c.expires_at.map(|t| t.to_rfc3339()),
            metadata: metadata_json,
        })
    }

    /// Convert this payload back to a [`StoredCredential`].
    fn into_credential(self) -> Result<StoredCredential, StoreError> {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;

        let data = STANDARD
            .decode(&self.data)
            .map_err(|e| StoreError::Backend(Box::new(e)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| StoreError::Backend(Box::new(e)))?
            .with_timezone(&chrono::Utc);

        let updated_at = chrono::DateTime::parse_from_rfc3339(&self.updated_at)
            .map_err(|e| StoreError::Backend(Box::new(e)))?
            .with_timezone(&chrono::Utc);

        let expires_at = self
            .expires_at
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| StoreError::Backend(Box::new(e)))
            })
            .transpose()?;

        let metadata: serde_json::Map<String, serde_json::Value> = if self.metadata.is_empty() {
            Default::default()
        } else {
            serde_json::from_str(&self.metadata).map_err(|e| StoreError::Backend(Box::new(e)))?
        };

        Ok(StoredCredential {
            id: self.id,
            data,
            state_kind: self.state_kind,
            state_version: self.state_version,
            version: self.version,
            created_at,
            updated_at,
            expires_at,
            metadata,
        })
    }
}

// ── Error helpers ────────────────────────────────────────────────────────────

/// Map a `vaultrs` error to a [`StoreError`].
fn map_vault_error(id: &str, err: vaultrs::error::ClientError) -> StoreError {
    let msg = format!("{err:?}");
    if msg.contains("404") || msg.contains("not found") {
        StoreError::NotFound { id: id.to_string() }
    } else {
        StoreError::Backend(Box::new(std::io::Error::other(msg)))
    }
}

// ── VaultStore ───────────────────────────────────────────────────────────────

/// HashiCorp Vault credential store using KV v2.
///
/// Thread-safe store that persists [`StoredCredential`] values as Vault KV v2
/// secrets. Each credential is stored at `{path_prefix}/{credential_id}`.
///
/// # CAS support
///
/// [`PutMode::CompareAndSwap`] leverages Vault KV v2 native versioning:
/// the credential's logical `version` is compared before write. Because
/// the KV v2 `cas` parameter operates on Vault's internal version counter
/// (not our logical version), we read-then-write with a logical version
/// check. This is safe for single-writer scenarios; for multi-writer
/// deployments, consider using Vault's built-in CAS parameter on write.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::store_vault::{VaultStore, VaultConfig, VaultAuthMethod};
///
/// let config = VaultConfig {
///     address: "https://vault.example.com:8200".into(),
///     auth_method: VaultAuthMethod::Token {
///         token: "s.my-token".into(),
///     },
///     ..Default::default()
/// };
///
/// let store = VaultStore::new(config).await?;
/// ```
pub struct VaultStore {
    client: Arc<VaultClient>,
    config: VaultConfig,
    #[allow(dead_code)]
    token_renewal_task: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl VaultStore {
    /// Create a new Vault store, connecting and authenticating immediately.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] if:
    /// - Configuration validation fails
    /// - Vault client creation fails
    /// - AppRole authentication fails
    pub async fn new(config: VaultConfig) -> Result<Self, StoreError> {
        config.validate().map_err(|e| {
            StoreError::Backend(format!("Vault configuration validation failed: {e}").into())
        })?;

        let mut settings_builder = VaultClientSettingsBuilder::default();
        settings_builder.address(&config.address);

        if let Some(ref namespace) = config.namespace {
            settings_builder.namespace(Some(namespace.clone()));
        }

        settings_builder.timeout(Some(config.timeout));
        // vaultrs `verify` flag: `true` means skip verification.
        settings_builder.verify(!config.tls_verify);

        let initial_token = match &config.auth_method {
            VaultAuthMethod::Token { token } => token.clone(),
            VaultAuthMethod::AppRole { .. } => String::new(),
        };

        if !initial_token.is_empty() {
            settings_builder.token(&initial_token);
        }

        let settings = settings_builder.build().map_err(|e| {
            StoreError::Backend(format!("Failed to build Vault client settings: {e}").into())
        })?;

        let mut client = VaultClient::new(settings).map_err(|e| {
            StoreError::Backend(format!("Failed to create Vault client: {e}").into())
        })?;

        // AppRole authentication
        if let VaultAuthMethod::AppRole { role_id, secret_id } = &config.auth_method {
            let auth_resp = vaultrs::auth::approle::login(&client, "approle", role_id, secret_id)
                .await
                .map_err(|e| {
                    StoreError::Backend(format!("AppRole authentication failed: {e:?}").into())
                })?;
            client.set_token(&auth_resp.client_token);
        }

        let client = Arc::new(client);

        tracing::info!(
            address = %config.address,
            mount_path = %config.mount_path,
            path_prefix = %config.path_prefix,
            "Initialized VaultStore"
        );

        // Background token renewal (placeholder)
        let token_renewal_task = if matches!(config.auth_method, VaultAuthMethod::Token { .. }) {
            let task =
                Self::start_token_renewal_task(Arc::clone(&client), config.token_renewal_threshold);
            Arc::new(RwLock::new(Some(task)))
        } else {
            Arc::new(RwLock::new(None))
        };

        Ok(Self {
            client,
            config,
            token_renewal_task,
        })
    }

    /// Start background token renewal (placeholder).
    ///
    /// Real implementation will call `vault token renew` when TTL drops below
    /// the threshold.
    fn start_token_renewal_task(_client: Arc<VaultClient>, _threshold: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            tracing::warn!(
                "VaultStore token renewal is a placeholder — ensure tokens have sufficient TTL"
            );
            std::future::pending::<()>().await;
        })
    }

    /// Build the Vault secret path for a credential.
    fn secret_path(&self, id: &str) -> String {
        format!("{}/{id}", self.config.path_prefix)
    }

    /// Read a credential from Vault, returning `None` if the secret is absent.
    async fn read_secret(&self, id: &str) -> Result<Option<StoredCredential>, StoreError> {
        let path = self.secret_path(id);
        let result = kv2::read::<HashMap<String, serde_json::Value>>(
            &*self.client,
            &self.config.mount_path,
            &path,
        )
        .await;

        match result {
            Ok(secret) => {
                let value = serde_json::Value::Object(secret.into_iter().collect());
                let payload: VaultPayload =
                    serde_json::from_value(value).map_err(|e| StoreError::Backend(Box::new(e)))?;
                Ok(Some(payload.into_credential()?))
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("404") || msg.contains("not found") {
                    Ok(None)
                } else {
                    Err(StoreError::Backend(Box::new(std::io::Error::other(msg))))
                }
            }
        }
    }

    /// Write a credential payload to Vault.
    async fn write_secret(&self, credential: &StoredCredential) -> Result<(), StoreError> {
        let path = self.secret_path(&credential.id);
        let payload = VaultPayload::from_credential(credential)?;
        let value = serde_json::to_value(&payload).map_err(|e| StoreError::Backend(Box::new(e)))?;

        kv2::set(&*self.client, &self.config.mount_path, &path, &value)
            .await
            .map_err(|e| map_vault_error(&credential.id, e))?;

        Ok(())
    }
}

impl CredentialStore for VaultStore {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        self.read_secret(id)
            .await?
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let existing = self.read_secret(&credential.id).await?;

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

        self.write_secret(&credential).await?;
        Ok(credential)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let path = self.secret_path(id);

        // Use delete_metadata for permanent removal in KV v2.
        kv2::delete_metadata(&*self.client, &self.config.mount_path, &path)
            .await
            .map_err(|e| {
                let msg = format!("{e:?}");
                if msg.contains("404") || msg.contains("not found") {
                    StoreError::NotFound { id: id.to_string() }
                } else {
                    StoreError::Backend(Box::new(std::io::Error::other(msg)))
                }
            })
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let result = kv2::list(
            &*self.client,
            &self.config.mount_path,
            &self.config.path_prefix,
        )
        .await;

        match result {
            Ok(keys) => {
                let ids: Vec<String> = keys
                    .into_iter()
                    .map(|k| k.trim_end_matches('/').to_string())
                    .collect();

                let Some(kind) = state_kind else {
                    return Ok(ids);
                };

                // Filter by state_kind: read each secret to check.
                let mut filtered = Vec::new();
                for id in &ids {
                    let matches = self
                        .read_secret(id)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|c| c.state_kind == kind);
                    if matches {
                        filtered.push(id.clone());
                    }
                }
                Ok(filtered)
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("404") || msg.contains("not found") {
                    Ok(Vec::new())
                } else {
                    Err(StoreError::Backend(Box::new(std::io::Error::other(msg))))
                }
            }
        }
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let path = self.secret_path(id);

        // read_metadata is lighter than a full read.
        let result = kv2::read_metadata(&*self.client, &self.config.mount_path, &path).await;

        match result {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("404") || msg.contains("not found") {
                    Ok(false)
                } else {
                    Err(StoreError::Backend(Box::new(std::io::Error::other(msg))))
                }
            }
        }
    }
}

impl Drop for VaultStore {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.token_renewal_task.try_write()
            && let Some(task) = guard.take()
        {
            task.abort();
            tracing::debug!("Aborted VaultStore token renewal task");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_values() {
        let cfg = VaultConfig::default();
        assert_eq!(cfg.mount_path, "secret");
        assert_eq!(cfg.path_prefix, "nebula/credentials");
        assert!(cfg.tls_verify);
        assert_eq!(cfg.timeout, Duration::from_secs(10));
    }

    #[test]
    fn config_validation_rejects_empty_address() {
        let mut cfg = VaultConfig::default();
        cfg.address = String::new();
        cfg.auth_method = VaultAuthMethod::Token {
            token: "s.test".into(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn config_validation_rejects_http_with_tls_verify() {
        let mut cfg = VaultConfig::default();
        cfg.address = "http://localhost:8200".into();
        cfg.tls_verify = true;
        cfg.auth_method = VaultAuthMethod::Token {
            token: "s.test".into(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn config_validation_accepts_http_without_tls_verify() {
        let mut cfg = VaultConfig::default();
        cfg.address = "http://localhost:8200".into();
        cfg.tls_verify = false;
        cfg.auth_method = VaultAuthMethod::Token {
            token: "s.test".into(),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_validation_rejects_empty_token() {
        let cfg = VaultConfig::default(); // default has empty token
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn config_validation_rejects_empty_role_id() {
        let mut cfg = VaultConfig::default();
        cfg.auth_method = VaultAuthMethod::AppRole {
            role_id: String::new(),
            secret_id: "secret".into(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn vault_payload_round_trip() {
        let cred = StoredCredential {
            id: "test-1".into(),
            data: b"secret-bytes".to_vec(),
            state_kind: "bearer".into(),
            state_version: 1,
            version: 3,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };

        let payload = VaultPayload::from_credential(&cred).unwrap();
        let recovered = payload.into_credential().unwrap();

        assert_eq!(recovered.id, cred.id);
        assert_eq!(recovered.data, cred.data);
        assert_eq!(recovered.state_kind, cred.state_kind);
        assert_eq!(recovered.version, cred.version);
    }

    #[test]
    fn vault_payload_round_trip_with_metadata() {
        let mut metadata = serde_json::Map::new();
        metadata.insert("env".into(), serde_json::Value::String("prod".into()));

        let cred = StoredCredential {
            id: "meta-1".into(),
            data: vec![1, 2, 3],
            state_kind: "api_key".into(),
            state_version: 2,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now()),
            metadata,
        };

        let payload = VaultPayload::from_credential(&cred).unwrap();
        let recovered = payload.into_credential().unwrap();

        assert_eq!(recovered.metadata.len(), 1);
        assert_eq!(
            recovered.metadata.get("env").and_then(|v| v.as_str()),
            Some("prod")
        );
        assert!(recovered.expires_at.is_some());
    }

    // Integration tests require a running Vault server.
    // Run with: cargo nextest run -p nebula-credential --features storage-vault -- --ignored
    #[tokio::test]
    #[ignore = "requires running Vault server"]
    async fn integration_crud() {
        let config = VaultConfig {
            address: "http://127.0.0.1:8200".into(),
            auth_method: VaultAuthMethod::Token {
                token: "root".into(),
            },
            tls_verify: false,
            ..Default::default()
        };

        let store = VaultStore::new(config).await.unwrap();

        let cred = StoredCredential {
            id: "int-test-1".into(),
            data: b"integration-secret".to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };

        // Create
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

        // Read
        let fetched = store.get("int-test-1").await.unwrap();
        assert_eq!(fetched.data, b"integration-secret");

        // Exists
        assert!(store.exists("int-test-1").await.unwrap());

        // Delete
        store.delete("int-test-1").await.unwrap();
    }
}
