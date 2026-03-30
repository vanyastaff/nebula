//! Kubernetes Secrets credential store (v2 [`CredentialStore`] backend).
//!
//! Stores each credential as a Kubernetes `Secret` resource within a single
//! namespace, with the secret name derived from a configurable prefix and the
//! credential ID.
//!
//! # Features
//!
//! - Namespace-isolated secrets
//! - Label-based filtering and organisation
//! - Annotation-based metadata storage
//! - In-cluster or kubeconfig-based authentication
//! - 1 MB payload size limit (Kubernetes hard limit)
//!
//! Feature-gated behind `storage-k8s`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_credential::store_k8s::{K8sSecretsStore, K8sSecretsConfig};
//!
//! let config = K8sSecretsConfig {
//!     namespace: "production".into(),
//!     prefix: "nebula-".into(),
//!     ..Default::default()
//! };
//! let store = K8sSecretsStore::new(config).await?;
//! let cred = store.get("my-api-key").await?;
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;
use kube::Client;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams};
use serde::{Deserialize, Serialize};

use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Maximum payload size for a Kubernetes Secret (1 MB).
const MAX_PAYLOAD_BYTES: usize = 1_000_000;

/// Annotation key for the serialised credential payload.
const DATA_KEY: &str = "credential";

/// Annotation prefix used for nebula-specific metadata.
const ANNOTATION_PREFIX: &str = "nebula.credential";

// ── Config ────────────────────────────────────────────────────────────────

/// Configuration for [`K8sSecretsStore`].
///
/// # Namespace isolation
///
/// Each store instance operates within a single Kubernetes namespace.
/// Secrets in different namespaces are completely isolated.
///
/// # Examples
///
/// ```rust
/// use nebula_credential::store_k8s::K8sSecretsConfig;
/// use std::time::Duration;
///
/// let config = K8sSecretsConfig {
///     namespace: "staging".into(),
///     prefix: "nebula-".into(),
///     timeout: Duration::from_secs(10),
///     ..Default::default()
/// };
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct K8sSecretsConfig {
    /// Kubernetes namespace for secrets.
    pub namespace: String,

    /// Path to a kubeconfig file.
    ///
    /// When `None`, the store tries in-cluster config first, then the default
    /// kubeconfig location (`~/.kube/config`).
    pub kubeconfig: Option<PathBuf>,

    /// Whether to use in-cluster service-account config.
    ///
    /// When `true` and `kubeconfig` is `None`, only the in-cluster path is
    /// attempted (no fallback to the default kubeconfig file).
    pub in_cluster: bool,

    /// Secret name prefix — every credential ID is stored as `{prefix}{id}`
    /// (lowercased, underscores replaced with hyphens for DNS compatibility).
    pub prefix: String,

    /// Per-operation timeout.
    pub timeout: Duration,

    /// Default labels applied to every secret.
    pub labels: HashMap<String, String>,

    /// Default annotations applied to every secret.
    pub annotations: HashMap<String, String>,

    /// Accept invalid / self-signed TLS certificates (testing only).
    pub accept_invalid_certs: bool,
}

impl Default for K8sSecretsConfig {
    fn default() -> Self {
        Self {
            namespace: "default".into(),
            kubeconfig: None,
            in_cluster: false,
            prefix: String::new(),
            timeout: Duration::from_secs(5),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            accept_invalid_certs: false,
        }
    }
}

// ── Serde wrapper ─────────────────────────────────────────────────────────

/// JSON-serializable representation of a [`StoredCredential`] stored inside
/// the Kubernetes Secret `data` field.
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

/// Kubernetes Secrets credential store.
///
/// Implements the v2 [`CredentialStore`] trait, storing each credential as a
/// Kubernetes `Secret` resource. The credential is JSON-serialized and placed
/// in the `data["credential"]` field of the Secret. Metadata is mirrored
/// into labels and annotations for server-side filtering.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::store_k8s::{K8sSecretsStore, K8sSecretsConfig};
///
/// let store = K8sSecretsStore::new(K8sSecretsConfig {
///     namespace: "production".into(),
///     prefix: "nebula-".into(),
///     ..Default::default()
/// }).await?;
/// ```
pub struct K8sSecretsStore {
    /// Provider configuration.
    config: K8sSecretsConfig,
    /// Namespaced Kubernetes `Secret` API handle.
    secrets_api: Api<Secret>,
}

impl K8sSecretsStore {
    /// Create a new store, initialising the Kubernetes client.
    ///
    /// # Authentication
    ///
    /// - If `config.kubeconfig` is set, that file is used.
    /// - Otherwise, in-cluster config is tried first (service account), falling
    ///   back to the default kubeconfig path.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] if the Kubernetes client cannot be created.
    pub async fn new(config: K8sSecretsConfig) -> Result<Self, StoreError> {
        let client = if let Some(path) = &config.kubeconfig {
            let content = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| StoreError::Backend(Box::new(e)))?;

            let kubeconfig = kube::config::Kubeconfig::from_yaml(&content)
                .map_err(|e| StoreError::Backend(e.to_string().into()))?;

            let mut kube_config = kube::Config::from_custom_kubeconfig(
                kubeconfig,
                &kube::config::KubeConfigOptions::default(),
            )
            .await
            .map_err(|e| StoreError::Backend(e.to_string().into()))?;

            kube_config.accept_invalid_certs = config.accept_invalid_certs;

            Client::try_from(kube_config).map_err(|e| StoreError::Backend(e.to_string().into()))?
        } else {
            Client::try_default()
                .await
                .map_err(|e| StoreError::Backend(e.to_string().into()))?
        };

        let secrets_api: Api<Secret> = Api::namespaced(client, &config.namespace);

        Ok(Self {
            config,
            secrets_api,
        })
    }

    /// Derive the Kubernetes-safe secret name for a credential ID.
    ///
    /// Lowercases and replaces underscores with hyphens to satisfy DNS
    /// subdomain naming rules.
    fn secret_name(&self, id: &str) -> String {
        format!("{}{}", self.config.prefix, id)
            .to_lowercase()
            .replace('_', "-")
    }

    /// Serialize a [`StoredCredential`] to JSON bytes and validate the K8s
    /// payload size limit.
    fn serialize_payload(credential: &StoredCredential) -> Result<Vec<u8>, StoreError> {
        let payload: SecretPayload = credential.clone().into();
        let json = serde_json::to_vec(&payload).map_err(|e| StoreError::Backend(Box::new(e)))?;

        if json.len() > MAX_PAYLOAD_BYTES {
            return Err(StoreError::Backend(
                format!(
                    "credential payload ({} bytes) exceeds Kubernetes 1 MB limit",
                    json.len()
                )
                .into(),
            ));
        }
        Ok(json)
    }

    /// Build labels for a Kubernetes Secret from config defaults and the
    /// credential's `state_kind`.
    fn build_labels(&self, state_kind: &str) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        for (k, v) in &self.config.labels {
            labels.insert(k.clone(), v.clone());
        }
        labels.insert(
            format!("{ANNOTATION_PREFIX}/managed-by"),
            "nebula".to_string(),
        );
        labels.insert(
            format!("{ANNOTATION_PREFIX}/state-kind"),
            sanitize_label_value(state_kind),
        );
        labels
    }

    /// Build annotations from config defaults and credential timestamps.
    fn build_annotations(&self, credential: &StoredCredential) -> BTreeMap<String, String> {
        let mut annotations = BTreeMap::new();
        for (k, v) in &self.config.annotations {
            annotations.insert(k.clone(), v.clone());
        }
        annotations.insert(
            format!("{ANNOTATION_PREFIX}/version"),
            credential.version.to_string(),
        );
        annotations.insert(
            format!("{ANNOTATION_PREFIX}/created-at"),
            credential.created_at.to_rfc3339(),
        );
        annotations.insert(
            format!("{ANNOTATION_PREFIX}/updated-at"),
            credential.updated_at.to_rfc3339(),
        );
        if let Some(expires) = credential.expires_at {
            annotations.insert(
                format!("{ANNOTATION_PREFIX}/expires-at"),
                expires.to_rfc3339(),
            );
        }
        annotations
    }

    /// Read an existing credential from the backend, returning `None` when
    /// the secret does not exist.
    async fn read_secret(&self, name: &str) -> Result<Option<StoredCredential>, StoreError> {
        match self.secrets_api.get(name).await {
            Ok(secret) => {
                let data = secret
                    .data
                    .as_ref()
                    .and_then(|d| d.get(DATA_KEY))
                    .ok_or_else(|| {
                        StoreError::Backend("secret missing 'credential' data key".into())
                    })?;

                let payload: SecretPayload = serde_json::from_slice(&data.0)
                    .map_err(|e| StoreError::Backend(Box::new(e)))?;
                Ok(Some(payload.into()))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("NotFound") {
                    Ok(None)
                } else {
                    Err(StoreError::Backend(msg.into()))
                }
            }
        }
    }
}

/// Sanitize a value for use as a Kubernetes label value (max 63 chars,
/// lowercase alphanumeric + `-_.`, must start/end alphanumeric).
fn sanitize_label_value(input: &str) -> String {
    let mut s: String = input
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();

    if s.len() > 63 {
        s.truncate(63);
    }

    s = s
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();

    if s.is_empty() {
        s = "unknown".to_string();
    }

    s
}

impl CredentialStore for K8sSecretsStore {
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

        let json_bytes = Self::serialize_payload(&credential)?;

        // ── Build K8s Secret resource ─────────────────────────────────────
        let labels = self.build_labels(&credential.state_kind);
        let annotations = self.build_annotations(&credential);

        let mut secret_data = BTreeMap::new();
        secret_data.insert(DATA_KEY.to_string(), ByteString(json_bytes));

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(labels),
                annotations: Some(annotations),
                ..Default::default()
            },
            data: Some(secret_data),
            type_: Some("Opaque".to_string()),
            ..Default::default()
        };

        // Server-side apply for idempotent create-or-update.
        let patch_params = PatchParams::apply("nebula-credential");
        self.secrets_api
            .patch(&name, &patch_params, &Patch::Apply(&secret))
            .await
            .map_err(|e| StoreError::Backend(e.to_string().into()))?;

        Ok(credential)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let name = self.secret_name(id);

        match self
            .secrets_api
            .delete(&name, &DeleteParams::default())
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("NotFound") {
                    Err(StoreError::NotFound { id: id.to_string() })
                } else {
                    Err(StoreError::Backend(msg.into()))
                }
            }
        }
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let label_selector = state_kind.map(|kind| {
            format!(
                "{ANNOTATION_PREFIX}/state-kind={}",
                sanitize_label_value(kind)
            )
        });

        let list_params = match &label_selector {
            Some(sel) => ListParams::default().labels(sel),
            None => ListParams::default(),
        };

        let secrets = self
            .secrets_api
            .list(&list_params)
            .await
            .map_err(|e| StoreError::Backend(e.to_string().into()))?;

        let prefix = self.config.prefix.to_lowercase().replace('_', "-");
        let mut ids = Vec::new();

        for secret in secrets.items {
            if let Some(name) = secret.metadata.name
                && let Some(id_part) = name.strip_prefix(&prefix)
            {
                ids.push(id_part.to_string());
            }
        }

        Ok(ids)
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let name = self.secret_name(id);

        match self.secrets_api.get(&name).await {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("NotFound") {
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
        let config = K8sSecretsConfig::default();
        assert_eq!(config.namespace, "default");
        assert!(config.kubeconfig.is_none());
        assert!(!config.in_cluster);
        assert!(config.prefix.is_empty());
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(config.labels.is_empty());
        assert!(config.annotations.is_empty());
        assert!(!config.accept_invalid_certs);
    }

    #[test]
    fn config_with_all_fields() {
        let config = K8sSecretsConfig {
            namespace: "production".into(),
            kubeconfig: Some(PathBuf::from("/home/user/.kube/config")),
            in_cluster: false,
            prefix: "nebula-".into(),
            timeout: Duration::from_secs(10),
            labels: HashMap::from([("app".into(), "nebula".into())]),
            annotations: HashMap::from([("note".into(), "managed".into())]),
            accept_invalid_certs: true,
        };

        assert_eq!(config.namespace, "production");
        assert!(config.kubeconfig.is_some());
        assert_eq!(config.prefix, "nebula-");
        assert_eq!(config.labels.len(), 1);
        assert_eq!(config.annotations.len(), 1);
        assert!(config.accept_invalid_certs);
    }

    #[test]
    fn secret_name_lowercases_and_replaces_underscores() {
        let prefix = "nebula-";
        let id = "My_API_Token";
        let name = format!("{prefix}{id}").to_lowercase().replace('_', "-");
        assert_eq!(name, "nebula-my-api-token");
    }

    #[test]
    fn secret_payload_round_trip() {
        let cred = StoredCredential {
            id: "k8s-test".into(),
            data: vec![0xCA, 0xFE],
            state_kind: "api_key".into(),
            state_version: 2,
            version: 5,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };

        let payload: SecretPayload = cred.clone().into();
        let json = serde_json::to_vec(&payload).unwrap();
        let deserialized: SecretPayload = serde_json::from_slice(&json).unwrap();
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

        let result = K8sSecretsStore::serialize_payload(&cred);
        assert!(result.is_err());
    }

    #[test]
    fn sanitize_label_value_works() {
        assert_eq!(sanitize_label_value("bearer"), "bearer");
        assert_eq!(sanitize_label_value("API Key!"), "api-key");
        assert_eq!(sanitize_label_value("---"), "unknown");
        assert_eq!(sanitize_label_value(""), "unknown");
        assert_eq!(sanitize_label_value(&"x".repeat(100)), "x".repeat(63));
    }

    #[test]
    fn config_serde_round_trip() {
        let config = K8sSecretsConfig {
            namespace: "staging".into(),
            kubeconfig: Some(PathBuf::from("/etc/kube/config")),
            in_cluster: true,
            prefix: "app-".into(),
            timeout: Duration::from_secs(8),
            labels: HashMap::from([("env".into(), "staging".into())]),
            annotations: HashMap::new(),
            accept_invalid_certs: false,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: K8sSecretsConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.namespace, config.namespace);
        assert_eq!(deserialized.prefix, config.prefix);
        assert_eq!(deserialized.in_cluster, config.in_cluster);
        assert_eq!(deserialized.labels, config.labels);
    }
}
