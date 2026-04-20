//! # nebula-credential
//!
//! **Role:** Credential Contract — stored state vs projected auth material;
//! engine-owned rotation and refresh. Canon §3.5, §12.5.
//!
//! The engine owns the split between stored `State` (encrypted at rest) and the
//! projected auth material action code receives. Action authors bind to a
//! `Credential` type; they never hand-roll token refresh, never hold plaintext
//! secrets longer than necessary, and never see secrets in logs.
//!
//! ## Key types
//!
//! - `Credential` — unified trait: `resolve()`, `refresh()`, `test()`, `project()`.
//! - `CredentialMetadata` — static type descriptor: key, name, schema, `AuthPattern`.
//! - `CredentialRecord` — runtime operational state (created_at, version, expiry, tags). Previously
//!   named `Metadata` (ADR 0004).
//! - `CredentialStore`, `EncryptionLayer`, `CacheLayer`, `AuditLayer`, `ScopeLayer` — composable
//!   storage with layered decoration.
//! - `CredentialRegistry`, `CredentialResolver` — type-erased dispatch and resolution.
//! - `SecretString`, `CredentialGuard` — zeroizing secret wrappers.
//! - `EncryptedData`, `EncryptionKey`, `encrypt`, `decrypt` — AES-256-GCM primitives.
//! - `#[derive(Credential)]`, `#[derive(AuthScheme)]` — proc-macro derivations.
//!
//! ## Security invariant (canon §12.5)
//!
//! Encryption at rest: AES-256-GCM with Argon2id KDF, credential ID bound as AAD.
//! No bypass for debugging. All intermediate plaintext in `Zeroizing<Vec<u8>>`.
//! `Debug` impls on credential wrappers redact secret fields.
//!
//! See `crates/credential/README.md` for the full contract and canon invariants.
#![forbid(unsafe_code)]

/// Error type for credential access operations.
pub mod access_error;
/// Credential accessor trait and implementations (Noop, Scoped).
pub mod accessor;
/// Object-safe supertrait for credential dependency declaration.
pub mod any;
/// Credential operation context.
pub mod context;
/// Unified Credential trait.
pub mod credential;
/// Built-in credential type implementations.
pub mod credentials;
/// Cryptographic utilities: AES-256-GCM, key derivation, PKCE, serde_base64.
pub mod crypto;
/// Error types for credential operations.
pub mod error;
/// Credential guard — secure wrapper with Deref + Zeroize on drop.
pub mod guard;
/// Typed credential handle returned by the resolver.
pub mod handle;
/// Newtype for credential type keys.
pub mod key;
/// Credential type metadata schema (integration catalog).
pub mod metadata;
/// Serde helpers for [`Option<SecretString>`] that preserve the actual value.
pub mod option_serde_secret;
/// Typed pending state for interactive flows.
pub mod pending;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
/// Credential record — runtime operational state.
pub mod record;
/// Type-erased credential registry for runtime dispatch.
pub mod registry;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
/// Retry logic with exponential backoff.
pub mod retry;
/// Credential rotation
#[cfg(feature = "rotation")]
pub mod rotation;
/// Authentication scheme types.
pub mod scheme;
/// Secret string type with automatic zeroization.
pub mod secret_string;
/// Serde helpers for [`SecretString`] that preserve the actual value.
pub mod serde_secret;
/// Credential snapshot.
pub mod snapshot;
/// Credential state trait for stored credential data.
pub mod state;
/// Reusable protocol pattern for static credentials.
pub mod static_protocol;

// ── Storage ─────────────────────────────────────────────────────────────────

/// Composable storage layers (encryption, etc.) for stores.
pub mod layer;
/// Credential store trait with layered composition.
pub mod store;
/// In-memory credential store for testing.
pub mod store_memory;

// ── Executor ────────────────────────────────────────────────────────────────

/// Framework executor for credential resolution with timeouts.
pub mod executor;

// ── Resolution ──────────────────────────────────────────────────────────────

/// Refresh coordination — thundering herd prevention.
pub mod refresh;
/// Runtime credential resolution.
pub mod resolver;

// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Derive macros
// Credential access error
pub use access_error::CredentialAccessError;
// Credential accessor trait + implementations
pub use accessor::{
    CredentialAccessor, NoopCredentialAccessor, ScopedCredentialAccessor,
    default_credential_accessor,
};
// Unified Credential trait
pub use credential::Credential;
// Built-in credential implementations
pub use credentials::{
    ApiKeyCredential, BasicAuthCredential, OAuth2Credential, OAuth2Pending, OAuth2State,
};
// Framework executor
pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
// Credential guard
pub use guard::CredentialGuard;
// Typed handle
pub use handle::CredentialHandle;
// Credential key newtype
pub use key::CredentialKey;
#[cfg(any(test, feature = "test-util"))]
pub use layer::StaticKeyProvider;
// Storage layers
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer, EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError,
    ScopeLayer, ScopeResolver,
};
pub use nebula_core::{AuthPattern, AuthScheme, CredentialEvent, CredentialId};
pub use nebula_credential_macros::{AuthScheme, Credential};
// Pending state
pub use pending::{NoPendingState, PendingState, PendingToken};
// Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
pub use pending_store_memory::InMemoryPendingStore;
// Refresh coordination
pub use refresh::{RefreshAttempt, RefreshCoordinator};
// Registry
pub use registry::{CredentialRegistry, RegistryError};
// Resolve types
pub use resolve::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
// Resolver
pub use resolver::{CredentialResolver, ResolveError};
// Auth schemes (12 universal types)
pub use scheme::{
    Certificate, ChallengeSecret, ConnectionUri, FederatedAssertion, IdentityPassword,
    InstanceBinding, KeyPair, OAuth2Token, OtpSeed, SecretToken, SharedKey, SigningKey,
};
pub use secret_string::SecretString;
// Credential state
pub use state::CredentialState;
// Static protocol pattern
pub use static_protocol::StaticProtocol;
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
pub use store_memory::InMemoryStore;

// Any-credential object-safe supertrait
pub use crate::any::AnyCredential;
// Core types & errors
pub use crate::context::{CredentialContext, CredentialResolverRef};
// Crypto utilities
pub use crate::crypto::{EncryptedData, EncryptionKey, decrypt, encrypt};
// Rotation (feature-gated)
#[cfg(feature = "rotation")]
pub use crate::rotation::{
    CredentialRotationEvent, GracePeriodConfig, RotationError, RotationResult,
};
pub use crate::{
    error::{
        CredentialError, CryptoError, RefreshErrorKind, ResolutionStage, RetryAdvice,
        ValidationError,
    },
    metadata::{CredentialMetadata, CredentialMetadataBuilder, MetadataCompatibilityError},
    record::CredentialRecord,
    snapshot::{CredentialSnapshot, SnapshotError},
};
