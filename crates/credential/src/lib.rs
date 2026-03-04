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
//! - **Provider abstraction** - Decoupled credential access via `CredentialProvider` trait
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use nebula_credential::{CredentialManager, CredentialContext, SecretString};
//!
//! // Create manager
//! let manager = CredentialManager::builder()
//!     .with_storage(local_storage)
//!     .build()?;
//!
//! // Store credential
//! let ctx = CredentialContext::new("user_123");
//! manager.store("github_token", secret_data, &ctx).await?;
//!
//! // Retrieve credential
//! let secret = manager.get("github_token", &ctx).await?;
//! ```
//!
//! # CredentialProvider Pattern
//!
//! For decoupled credential access in actions/triggers:
//!
//! ```rust,ignore
//! use nebula_credential::{CredentialProvider, CredentialRef};
//!
//! // Type-safe acquisition
//! struct GithubToken;
//! let token = provider.credential::<GithubToken>(&ctx).await?;
//!
//! // Dynamic acquisition
//! let token = provider.get("github_token", &ctx).await?;
//! ```
//!
//! See [`core::reference`] module for details on `CredentialRef` and `CredentialProvider`.
#![deny(unsafe_code)]
#![forbid(unsafe_code)]

/// Core types, errors, and primitives
pub mod core;
/// Credential manager - high-level API for credential operations
pub mod manager;
/// Built-in reusable credential protocols (ApiKey, OAuth2, etc.)
pub mod protocols;
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
pub use crate::core::reference::ErasedCredentialRef;
pub use crate::core::result::{CreateResult, InitializeResult};
pub use crate::core::{
    CredentialContext, CredentialDescription, CredentialError, CredentialFilter, CredentialId,
    CredentialMetadata, CredentialProvider, CredentialRef, CredentialState, CredentialStatus,
    CryptoError, ManagerError, ManagerResult, SecretString, StorageError, ValidationError,
    status_from_metadata,
};

// Traits
pub use crate::traits::{
    CredentialResource, CredentialType, DistributedLock, FlowProtocol, InteractiveCredential,
    LockError, LockGuard, Refreshable, Revocable, RotationStrategy, StateStore, StaticProtocol,
    StorageProvider,
};

// Protocols
pub use crate::protocols::{
    ApiKeyProtocol, ApiKeyState, AuthStyle, BasicAuthProtocol, BasicAuthState, DatabaseProtocol,
    DatabaseState, GrantType, HeaderAuthProtocol, HeaderAuthState, KerberosConfig, LdapConfig,
    LdapProtocol, LdapState, MtlsConfig, OAuth2Config, OAuth2ConfigBuilder, OAuth2Protocol,
    OAuth2State, SamlBinding, SamlConfig, TlsMode,
};

// Utils - crypto
pub use crate::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};

// Rotation
pub use crate::rotation::{
    CredentialRotationEvent, GracePeriodConfig, RotationError, RotationResult,
};

/// Commonly used types and traits
pub mod prelude {
    // Core types
    pub use crate::core::result::{CreateResult, InitializeResult};
    pub use crate::core::{
        CredentialContext, CredentialError, CredentialFilter, CredentialId, CredentialMetadata,
        CredentialProvider, CredentialRef, CredentialStatus, SecretString, status_from_metadata,
    };

    // Rotation types
    pub use crate::rotation::policy::RotationPolicy;
    pub use crate::rotation::{RotationError, RotationResult};

    // Traits
    pub use crate::traits::{
        CredentialResource, CredentialType, DistributedLock, FlowProtocol, InteractiveCredential,
        LockError, LockGuard, Refreshable, Revocable, StateStore, StaticProtocol, StorageProvider,
    };

    // Protocols
    pub use crate::protocols::{
        ApiKeyProtocol, ApiKeyState, AuthStyle, BasicAuthProtocol, BasicAuthState,
        DatabaseProtocol, DatabaseState, GrantType, HeaderAuthProtocol, HeaderAuthState,
        KerberosConfig, LdapConfig, LdapProtocol, LdapState, MtlsConfig, OAuth2Config,
        OAuth2ConfigBuilder, OAuth2Protocol, OAuth2State, SamlBinding, SamlConfig, TlsMode,
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

    #[cfg(feature = "storage-postgres")]
    pub use crate::providers::PostgresStorageProvider;

    // Retry utilities
    pub use crate::utils::RetryPolicy;

    // Credential Manager (Phase 3)
    pub use crate::manager::{
        CacheConfig, CacheLayer, CacheStats, CredentialManager, CredentialManagerBuilder,
        CredentialTypeSchema, EvictionStrategy, ManagerConfig, ValidationDetails, ValidationResult,
    };

    // Credential Rotation (Phase 4) - Already exported in prelude above
}
