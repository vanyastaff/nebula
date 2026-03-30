//! Nebula Credential - Universal credential management system
//!
//! A secure, extensible credential management system for workflow automation.
//!
//! # Architecture (v2)
//!
//! The credential system uses a trait-based approach:
//!
//! - **[`Credential`] trait** -- single unified trait replacing six v1 traits.
//!   Each credential type implements `resolve()`, `refresh()`, `test()`, and
//!   `project()` (extracts the auth scheme consumers see).
//!
//! - **[`CredentialStore`]** -- composable storage with layered encryption,
//!   caching, scoping, and audit via [`layer`] module.
//!
//! - **[`CredentialRegistry`]** -- type-erased runtime dispatch for credential
//!   resolution.
//!
//! - **[`CredentialResolver`]** -- runtime resolution engine: resolve, refresh,
//!   test credentials via the registry.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use nebula_credential::{
//!     Credential, CredentialStore, InMemoryStore,
//!     ApiKeyCredential, BearerToken,
//! };
//! ```
#![deny(unsafe_code)]
#![forbid(unsafe_code)]

/// Object-safe supertrait for credential dependency declaration.
pub mod any;
/// Core types, errors, and primitives
pub mod core;
/// Credential rotation
pub mod rotation;
/// Utilities for crypto, time, etc.
pub mod utils;

// ── v2 Core ─────────────────────────────────────────────────────────────────

/// Typed credential handle returned by the resolver.
pub mod credential_handle;
/// Newtype for credential type keys.
pub mod credential_key;
/// Credential state trait for stored credential data.
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
pub use crate::core::{
    CredentialContext, CredentialDescription, CredentialError, CredentialId, CredentialMetadata,
    CredentialSnapshot, CryptoError, ManagerError, ManagerResult, RefreshErrorKind,
    ResolutionStage, RetryAdvice, SecretString, StorageError, ValidationError,
};

// Utils - crypto
pub use crate::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};

// Rotation
pub use crate::rotation::{
    CredentialRotationEvent, GracePeriodConfig, RotationError, RotationResult,
};

// v2: Credential key newtype
pub use credential_key::CredentialKey;

// v2: Unified Credential trait
pub use credential_trait::Credential;

// v2: Credential state
pub use credential_state::CredentialState;

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
