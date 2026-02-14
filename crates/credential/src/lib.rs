//! Nebula Credential - Universal credential management system
//!
//! A secure, extensible credential management system for workflow automation.
//!
//! # Features
//!
//! - **Protocol-agnostic flows** - `OAuth2`, API Keys, JWT, SAML, Kerberos, mTLS
//! - **Type-safe credentials** - Compile-time verification with generic flows
//! - **Interactive authentication** - Multi-step flows with user interaction
//! - **Secure storage** - Zero-copy secrets with automatic zeroization
//! - **Minimal boilerplate** - ~30-50 lines to add new integrations
#![deny(unsafe_code)]
#![forbid(unsafe_code)]

/// Core types, errors, and primitives
pub mod core;
/// Credential manager - high-level API for credential operations
pub mod manager;
/// Storage provider implementations
pub mod providers;
/// Credential rotation (Phase 4)
pub mod rotation;
/// Core traits for credentials, storage, and locking
pub mod traits;
/// Utilities for crypto, time, etc.
pub mod utils;

// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Core types & errors
pub use crate::core::{
    CredentialContext, CredentialDescription, CredentialError, CredentialFilter, CredentialId,
    CredentialMetadata, CredentialState, CryptoError, ManagerError, ManagerResult, ScopeId,
    SecretString, StorageError, ValidationError,
};

// Traits
pub use crate::traits::{DistributedLock, LockError, LockGuard, StateStore, StorageProvider};

// Utils - crypto
pub use crate::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};

// Rotation
pub use crate::rotation::{GracePeriodConfig, RotationError, RotationResult};

/// Commonly used types and traits
pub mod prelude {
    // Core types
    pub use crate::core::{
        CredentialContext, CredentialError, CredentialFilter, CredentialId, CredentialMetadata,
        SecretString,
    };

    // Rotation types
    pub use crate::rotation::policy::RotationPolicy;
    pub use crate::rotation::{RotationError, RotationResult};

    // Traits
    pub use crate::traits::{
        // Credential, InteractiveCredential,
        DistributedLock,
        LockError,
        LockGuard,
        StateStore,
        StorageProvider,
    };

    // Utils - crypto functions
    pub use crate::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};

    // Storage providers (Phase 2)
    pub use crate::providers::{ConfigError, MockStorageProvider, ProviderConfig, StorageMetrics};

    #[cfg(feature = "storage-local")]
    pub use crate::providers::{LocalStorageConfig, LocalStorageProvider};

    #[cfg(feature = "storage-aws")]
    pub use crate::providers::{AwsSecretsManagerConfig, AwsSecretsManagerProvider};

    #[cfg(feature = "storage-vault")]
    pub use crate::providers::{HashiCorpVaultProvider, VaultAuthMethod, VaultConfig};

    #[cfg(feature = "storage-k8s")]
    pub use crate::providers::{KubernetesSecretsConfig, KubernetesSecretsProvider};

    // Retry utilities
    pub use crate::utils::RetryPolicy;

    // Credential Manager (Phase 3)
    pub use crate::manager::{
        CacheConfig, CacheLayer, CacheStats, CredentialManager, CredentialManagerBuilder,
        EvictionStrategy, ManagerConfig, ValidationDetails, ValidationResult,
    };

    // Credential Rotation (Phase 4) - Already exported in prelude above
}
