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

/// Object-safe supertrait for credential dependency declaration.
pub mod any;
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

// ── v2 Core ─────────────────────────────────────────────────────────────────

/// Typed credential handle returned by the resolver.
pub mod credential_handle;
/// Credential state trait for stored credential data.
// TODO: rename CredentialStateV2 → CredentialState once v1 CredentialState
//       (in core::state) is removed.
pub mod credential_state;
/// Unified Credential trait replacing six v1 traits.
pub mod credential_trait;
/// Built-in credential type implementations.
pub mod credentials;
/// Typed pending state for interactive flows.
pub mod pending;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
/// Opaque token for pending interactive flows.
pub mod pending_token;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
/// Authentication scheme types.
pub mod scheme;

// ── v2 Storage ──────────────────────────────────────────────────────────────

/// Credential store trait with layered composition.
pub mod credential_store;
/// Composable storage layers (encryption, etc.) for stores.
pub mod layer;
/// AWS Secrets Manager credential store.
#[cfg(feature = "storage-aws")]
pub mod store_aws;
/// Kubernetes Secrets credential store.
#[cfg(feature = "storage-k8s")]
pub mod store_k8s;
/// Filesystem-based credential store for desktop/single-node use.
#[cfg(feature = "storage-local")]
pub mod store_local;
/// In-memory credential store for testing.
pub mod store_memory;
/// Postgres-backed credential store via `nebula-storage` KV layer.
#[cfg(feature = "storage-postgres")]
pub mod store_postgres;
/// HashiCorp Vault credential store using KV v2 engine.
#[cfg(feature = "storage-vault")]
pub mod store_vault;

// ── v2 Executor ────────────────────────────────────────────────────────

/// Framework executor for credential resolution with timeouts.
pub mod executor;

// ── v2 Resolution ───────────────────────────────────────────────────────────

/// Type-erased credential registry for runtime dispatch.
pub mod credential_registry;
/// Refresh coordination -- thundering herd prevention.
pub mod refresh;
/// Runtime credential resolution.
pub mod resolver;

// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Any-credential object-safe supertrait
pub use crate::any::AnyCredential;

// Core types & errors
pub use crate::core::reference::ErasedCredentialRef;
pub use crate::core::result::{CreateResult, InitializeResult};
pub use crate::core::{
    CredentialContext, CredentialDescription, CredentialError, CredentialFilter, CredentialId,
    CredentialMetadata, CredentialProvider, CredentialRef, CredentialSnapshot, CredentialState,
    CredentialStatus, CryptoError, ManagerError, ManagerResult, RefreshErrorKind, ResolutionStage,
    RetryAdvice, SecretString, StorageError, ValidationError, status_from_metadata,
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

// v2: Unified Credential trait
pub use credential_trait::Credential;

// v2: Credential state
pub use credential_state::CredentialStateV2;

// v2: Auth schemes
pub use scheme::{
    ApiKeyAuth, AwsAuth, BasicAuth, BearerToken, CertificateAuth, DatabaseAuth, HeaderAuth,
    HmacSecret, KerberosAuth, LdapAuth, LdapBindMethod, LdapTlsMode, OAuth2Token, SamlAuth,
    SshAuth, SshAuthMethod, SslMode,
};

// v2: Pending state
pub use pending::{NoPendingState, PendingState};

// v2: Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
pub use pending_store_memory::InMemoryPendingStore;
pub use pending_token::PendingToken;

// v2: Built-in credential implementations
pub use credentials::OAuth2State as OAuth2StateV2;
pub use credentials::{
    ApiKeyCredential, BasicAuthCredential, DatabaseCredential, HeaderAuthCredential,
    OAuth2Credential, OAuth2Pending,
};

// v2: Typed handle
pub use credential_handle::CredentialHandle;

// v2: Storage
pub use credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer, ScopeLayer, ScopeResolver,
};
#[cfg(feature = "storage-aws")]
pub use store_aws::{AwsSecretsConfig, AwsSecretsStore};
#[cfg(feature = "storage-k8s")]
pub use store_k8s::{K8sSecretsConfig, K8sSecretsStore};
#[cfg(feature = "storage-local")]
pub use store_local::LocalFileStore;
pub use store_memory::InMemoryStore;
#[cfg(feature = "storage-postgres")]
pub use store_postgres::PostgresStore;
#[cfg(feature = "storage-vault")]
pub use store_vault::{VaultAuthMethod, VaultConfig, VaultStore};

// v2: Registry
pub use credential_registry::{CredentialRegistry, RegistryError};

// v2: Resolver
pub use resolver::{CredentialResolver, ResolveError};

// v2: Refresh coordination
pub use refresh::{RefreshAttempt, RefreshCoordinator};

// v2: Framework executor
pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};

// v2: Resolve types
pub use resolve::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
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
