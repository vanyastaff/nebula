//! External credential provider abstraction.
//!
//! Enables delegation of credential resolution to external secret managers
//! (HashiCorp Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault,
//! Infisical, Doppler, OS keyring, etc.). The contract is defined here;
//! concrete implementations live in downstream crates (e.g., `nebula-storage`)
//! behind feature gates.
//!
//! # Trait shape (per ADR-0051)
//!
//! [`ExternalProvider`] returns [`ProviderFuture<'a>`], a dyn-safe envelope
//! around `Pin<Box<dyn Future + Send>>` that also supports a zero-allocation
//! `ready` variant for synchronous providers (env-var, in-memory). This
//! mirrors the `NowOrLater` pattern from `aws-credential-types` and lets the
//! trait stay dyn-safe (`Arc<dyn ExternalProvider>` is supported) without
//! depending on `async-trait`.
//!
//! Resolutions return a [`ProviderResolution`] envelope (secret + optional
//! lease + optional TTL) rather than a bare [`SecretString`], so a downstream
//! `ProviderCacheLayer` (planned in `nebula-storage` per ADR-0032) can cache
//! provider responses according to provider-suggested TTLs without further
//! trait changes.
//!
//! # Composition
//!
//! [`ExternalProviderChain`] composes providers with **error-discriminated
//! fallback**: only [`ProviderError::NotFound`] triggers the next provider;
//! every other error short-circuits the chain. The chain itself implements
//! [`ExternalProvider`] (Liskov), so nested chains compose.

mod chain;
mod future;
mod resolution;

use std::fmt;

pub use chain::ExternalProviderChain;
pub use future::ProviderFuture;
pub use resolution::{LeaseHandle, ProviderResolution};

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
///
/// # Fall-through semantics
///
/// Only [`ProviderError::NotFound`] triggers
/// [`ExternalProviderChain`] fall-through to the next provider. Every other
/// variant short-circuits the chain so that misconfiguration in a later
/// provider cannot mask an `Unavailable` or `AccessDenied` from an earlier
/// one. Implementations MUST classify errors carefully — for example, a
/// network failure to a backing store is `Unavailable`, not `NotFound`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// Secret not found at the given path/version/field. **Triggers chain
    /// fall-through.**
    #[error("secret not found: {path}")]
    NotFound {
        /// The path that was looked up.
        path: String,
    },

    /// Provider is temporarily unavailable (network, rate limit, etc.).
    /// **Short-circuits the chain.**
    #[error("provider unavailable: {reason}")]
    Unavailable {
        /// Human-readable cause.
        reason: String,
    },

    /// Caller lacks permission to access the secret. **Short-circuits the chain.**
    #[error("access denied: {reason}")]
    AccessDenied {
        /// Human-readable cause.
        reason: String,
    },

    /// Catch-all for provider-specific errors. **Short-circuits the chain.**
    #[error("provider error: {0}")]
    Backend(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Trait for external secret providers.
///
/// Implementations fetch secrets from external systems on demand. The
/// credential framework calls [`resolve`](ExternalProvider::resolve) when
/// resolving a credential that references this provider via
/// [`ExternalReference`].
///
/// # Dyn-safety
///
/// `ExternalProvider` is dyn-safe because [`resolve`](Self::resolve) returns a
/// concrete [`ProviderFuture<'a>`] (not `impl Future`). Use `Arc<dyn
/// ExternalProvider>` for runtime composition.
///
/// # Implementors
///
/// Concrete implementations are feature-gated in downstream crates:
/// - `VaultProvider` — HashiCorp Vault KV v2 (planned)
/// - `AwsSmProvider` — AWS Secrets Manager (planned)
/// - `GcpSmProvider` — GCP Secret Manager (planned)
/// - `AzureKvProvider` — Azure Key Vault (planned)
/// - `EnvProvider` — `std::env::var` (planned, uses [`ProviderFuture::ready`] for zero-alloc resolve)
///
/// # Lease support (deferred)
///
/// Per ADR-0051 a `LeasedProvider: ExternalProvider` sub-trait will add
/// `renew` / `revoke` methods when the first lease-aware implementation
/// lands. The [`LeaseHandle`] data type ships now so resolutions can carry
/// lease metadata without trait support for renewal yet.
pub trait ExternalProvider: Send + Sync + fmt::Debug {
    /// Resolve a secret from the external system.
    ///
    /// Returns a [`ProviderFuture<'a>`] — synchronous providers should prefer
    /// [`ProviderFuture::ready`] to skip the box allocation.
    fn resolve<'a>(&'a self, reference: &'a ExternalReference) -> ProviderFuture<'a>;

    /// Check provider health / connectivity.
    ///
    /// Default implementation returns an empty [`ProviderResolution`] —
    /// override when the provider has a meaningful liveness probe.
    fn health_check(&self) -> ProviderFuture<'_> {
        ProviderFuture::ready(Ok(ProviderResolution::health_ok()))
    }

    /// Human-readable provider name for diagnostics and chain logs.
    fn provider_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_display() {
        assert_eq!(ProviderKind::Vault.to_string(), "vault");
        assert_eq!(
            ProviderKind::Custom("my-vault".to_owned()).to_string(),
            "custom:my-vault"
        );
    }

    #[test]
    fn provider_error_display_includes_payload() {
        let err = ProviderError::NotFound {
            path: "secret/foo".to_owned(),
        };
        assert!(err.to_string().contains("secret/foo"));
    }
}
