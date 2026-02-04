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
/// Storage provider implementations
pub mod providers;
/// Core traits for credentials, storage, and locking
pub mod traits;
/// Utilities for crypto, time, etc.
pub mod utils;

/// Commonly used types and traits
pub mod prelude {
    // Core types
    pub use crate::core::{
        CredentialContext, CredentialError, CredentialFilter, CredentialId, CredentialMetadata,
        SecretString,
    };

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
}
