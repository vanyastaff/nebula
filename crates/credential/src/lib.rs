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

/// Consumer-facing accessor surface — trait, handle, context, errors.
pub mod accessor;
/// Credential contract surface — Credential trait + associated types.
pub mod contract;
/// Built-in credential type implementations.
pub mod credentials;
/// Error types for credential operations.
pub mod error;
/// Credential metadata — static descriptors + runtime record + key newtype.
pub mod metadata;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
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
/// §12.5 secret-handling primitives — AES-256-GCM, guards, zeroizing wrappers, serde helpers.
pub mod secrets;
/// Credential snapshot.
pub mod snapshot;

/// Back-compat alias: serde attribute paths
/// `nebula_credential::serde_secret` and `nebula_credential::serde_secret::option`
/// continue to resolve here after the `secrets/` submodule move.
pub use crate::secrets::serde_secret;

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
// Consumer-facing accessor surface — trait, impls, handle, context, access error
pub use accessor::{
    CredentialAccessError, CredentialAccessor, CredentialContext, CredentialHandle,
    CredentialResolverRef, NoopCredentialAccessor, ScopedCredentialAccessor,
    default_credential_accessor,
};
// Credential contract — Credential trait + associated types
pub use contract::{
    AnyCredential, Credential, CredentialState, NoPendingState, PendingState, PendingToken,
    StaticProtocol,
};
// Built-in credential implementations
pub use credentials::{
    ApiKeyCredential, BasicAuthCredential, OAuth2Credential, OAuth2Pending, OAuth2State,
};
// Framework executor
pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
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
// §12.5 secret-handling primitives — crypto, guard, zeroizing wrappers
pub use secrets::{
    CredentialGuard, EncryptedData, EncryptionKey, SecretString, decrypt, decrypt_with_aad,
    encrypt, encrypt_with_aad, encrypt_with_key_id, generate_code_challenge,
    generate_pkce_verifier, generate_random_state,
};
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
pub use store_memory::InMemoryStore;

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
    metadata::{
        CredentialKey, CredentialMetadata, CredentialMetadataBuilder, CredentialRecord,
        MetadataCompatibilityError,
    },
    snapshot::{CredentialSnapshot, SnapshotError},
};
