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
/// Credential operation context.
pub mod context;
/// Unified Credential trait replacing six v1 traits.
pub mod credential;
/// Built-in credential type implementations.
pub mod credentials;
/// Credential type description schema.
pub mod description;
/// Error types for credential operations.
pub mod error;
/// Typed credential handle returned by the resolver.
pub mod handle;
/// Newtype for credential type keys.
pub mod key;
/// Credential metadata.
pub mod metadata;
/// Typed pending state for interactive flows.
pub mod pending;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
/// Type-erased credential registry for runtime dispatch.
pub mod registry;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
/// Credential rotation
#[cfg(feature = "rotation")]
pub mod rotation;
/// Authentication scheme types.
pub mod scheme;
/// Credential snapshot.
pub mod snapshot;
/// Credential state trait for stored credential data.
pub mod state;
/// Reusable protocol pattern for static credentials.
pub mod static_protocol;
/// Utilities for crypto, time, etc.
pub mod utils;

// ── v2 Storage ──────────────────────────────────────────────────────────────

/// Composable storage layers (encryption, etc.) for stores.
pub mod layer;
/// Credential store trait with layered composition.
pub mod store;
/// In-memory credential store for testing.
pub mod store_memory;

// ── v2 Executor ────────────────────────────────────────────────────────

/// Framework executor for credential resolution with timeouts.
pub mod executor;

// ── v2 Resolution ───────────────────────────────────────────────────────────

/// Refresh coordination -- thundering herd prevention.
pub mod refresh;
/// Runtime credential resolution.
pub mod resolver;

// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Any-credential object-safe supertrait
pub use crate::any::AnyCredential;

// Core types & errors
pub use crate::context::{CredentialContext, CredentialResolverRef};
pub use crate::description::CredentialDescription;
pub use crate::error::{
    CredentialError, CryptoError, RefreshErrorKind, ResolutionStage, RetryAdvice, ValidationError,
};
pub use crate::metadata::CredentialMetadata;
pub use crate::snapshot::{CredentialSnapshot, SnapshotError};
pub use nebula_core::CredentialId;

// Utils - crypto
pub use crate::utils::{EncryptedData, EncryptionKey, SecretString, decrypt, encrypt};

// Rotation
#[cfg(feature = "rotation")]
pub use crate::rotation::{
    CredentialRotationEvent, GracePeriodConfig, RotationError, RotationResult,
};

// v2: Credential key newtype
pub use key::CredentialKey;

// v2: Unified Credential trait
pub use credential::Credential;

// v2: Static protocol pattern
pub use static_protocol::StaticProtocol;

// v2: Credential state
pub use state::CredentialState;

// v2: Auth schemes
pub use scheme::{
    ApiKeyAuth, AwsAuth, BasicAuth, BearerToken, CertificateAuth, DatabaseAuth, HeaderAuth,
    HmacSecret, KerberosAuth, LdapAuth, LdapBindMethod, LdapTlsMode, OAuth2Token, SamlAuth,
    SshAuth, SshAuthMethod, SslMode,
};

// v2: Pending state
pub use pending::{NoPendingState, PendingState, PendingToken};

// v2: Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
pub use pending_store_memory::InMemoryPendingStore;

// v2: Built-in credential implementations
pub use credentials::{
    ApiKeyCredential, BasicAuthCredential, DatabaseCredential, HeaderAuthCredential,
    OAuth2Credential, OAuth2Pending, OAuth2State,
};

// v2: Typed handle
pub use handle::CredentialHandle;

// v2: Storage
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer, ScopeLayer, ScopeResolver,
};
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
pub use store_memory::InMemoryStore;

// v2: Registry
pub use registry::{CredentialRegistry, RegistryError};

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
