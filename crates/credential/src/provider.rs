//! External credential provider abstraction.
//!
//! Enables delegation of credential resolution to external secret managers
//! (HashiCorp Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault,
//! Infisical, Doppler, OS keyring, etc.).
//!
//! The [`ExternalProvider`] trait defines the contract; concrete implementations
//! live in downstream crates (e.g., `nebula-storage`) behind feature gates.
//!
//! See spec 22 §3.8 for design rationale.

use std::fmt;

use crate::SecretString;

/// Known external provider kinds.
///
/// Extensible via `ProviderKind::Custom(String)` for user-defined providers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// HashiCorp Vault KV v2 or Transit.
    Vault,
    /// AWS Secrets Manager.
    AwsSecretsManager,
    /// GCP Secret Manager.
    GcpSecretManager,
    /// Azure Key Vault.
    AzureKeyVault,
    /// Infisical secrets platform.
    Infisical,
    /// Doppler secrets manager.
    Doppler,
    /// OS-level keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service).
    OsKeyring,
    /// User-defined provider.
    Custom(String),
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vault => write!(f, "vault"),
            Self::AwsSecretsManager => write!(f, "aws_secrets_manager"),
            Self::GcpSecretManager => write!(f, "gcp_secret_manager"),
            Self::AzureKeyVault => write!(f, "azure_key_vault"),
            Self::Infisical => write!(f, "infisical"),
            Self::Doppler => write!(f, "doppler"),
            Self::OsKeyring => write!(f, "os_keyring"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

/// Reference to an externally-managed secret.
///
/// Stored in Nebula's database instead of the actual secret value.
/// On resolution, the framework calls the registered [`ExternalProvider`]
/// to fetch the real secret from the external system.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalReference {
    /// Which external provider manages this secret.
    pub provider: ProviderKind,
    /// Provider-specific path (e.g., `"secret/data/my-app/db-password"` for Vault).
    pub path: String,
    /// Optional version or stage (e.g., `"AWSCURRENT"` for AWS SM, version number for Vault).
    pub version: Option<String>,
    /// Optional field within the secret (for providers that store multiple K/V pairs per secret).
    pub field: Option<String>,
}

/// Error returned by [`ExternalProvider::resolve`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// Secret not found at the given path/version/field.
    #[error("secret not found: {path}")]
    NotFound { path: String },

    /// Provider is temporarily unavailable (network, rate limit, etc.).
    #[error("provider unavailable: {reason}")]
    Unavailable { reason: String },

    /// Caller lacks permission to access the secret.
    #[error("access denied: {reason}")]
    AccessDenied { reason: String },

    /// Catch-all for provider-specific errors.
    #[error("provider error: {0}")]
    Backend(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Trait for external secret providers.
///
/// Implementations fetch secrets from external systems on demand.
/// The credential framework calls [`resolve`](ExternalProvider::resolve)
/// when resolving a credential that references this provider via
/// [`ExternalReference`].
///
/// # Implementors
///
/// Concrete implementations are feature-gated in downstream crates:
/// - `VaultProvider` — HashiCorp Vault KV v2
/// - `AwsSmProvider` — AWS Secrets Manager
/// - `GcpSmProvider` — GCP Secret Manager
/// - `AzureKvProvider` — Azure Key Vault
#[async_trait::async_trait]
pub trait ExternalProvider: Send + Sync {
    /// Resolve a secret from the external system.
    async fn resolve(&self, reference: &ExternalReference) -> Result<SecretString, ProviderError>;

    /// Check provider health / connectivity.
    ///
    /// Default implementation returns `Ok(())` (always healthy).
    async fn health_check(&self) -> Result<(), ProviderError> {
        Ok(())
    }

    /// Human-readable provider name for diagnostics.
    fn provider_name(&self) -> &str;
}
