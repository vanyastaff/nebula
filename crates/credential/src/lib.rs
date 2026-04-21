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
//! ## Canonical import paths
//!
//! This crate follows the tokio/tracing idiom: submodules (`contract`,
//! `metadata`, `secrets`, `accessor`, `credentials`) are `pub` for escape
//! hatches, but the canonical public surface is **flat re-exports at the
//! root**. Prefer `use nebula_credential::SecretString;` over
//! `use nebula_credential::secrets::SecretString;`.
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

// Self-import so proc-macros that expand to `::nebula_credential::...` paths
// resolve correctly when the derive is used inside this crate itself (e.g.
// `#[derive(AuthScheme)]` in `scheme/secret_token.rs`). See canonical
// tokio/serde pattern: <https://doc.rust-lang.org/reference/items/extern-crates.html#the-self-keyword>.
extern crate self as nebula_credential;

// ── Submodules ──────────────────────────────────────────────────────────────
// Thematic groupings; each is `pub` for escape hatches but the canonical
// public surface is the flat root re-exports below.

/// Consumer-facing accessor surface — trait, handle, context, errors.
pub mod accessor;
/// Credential contract surface — Credential trait + associated types.
pub mod contract;
/// Built-in credential type implementations.
pub mod credentials;
/// Credential metadata — static descriptors + runtime record + key newtype + id.
pub mod metadata;
/// Credential rotation (blue-green, transaction, state machine).
#[cfg(feature = "rotation")]
pub mod rotation;
/// Authentication scheme types — AuthScheme trait, AuthPattern, 12 built-in schemes.
pub mod scheme;
/// §12.5 secret-handling primitives — AES-256-GCM, guards, zeroizing wrappers, serde helpers.
pub mod secrets;

// ── Utility modules ─────────────────────────────────────────────────────────
// Free-standing concerns: errors, resolve pipeline, storage, registry, etc.

/// Error types for credential operations.
pub mod error;
/// Credential lifecycle events for cross-crate signaling (EventBus payload).
pub mod event;
/// Framework executor for credential resolution with timeouts.
pub mod executor;
/// Composable storage layers (encryption, audit, cache, scope) for stores.
pub mod layer;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
/// Refresh coordination — thundering herd prevention.
pub mod refresh;
/// Type-erased credential registry for runtime dispatch.
pub mod registry;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
/// Runtime credential resolution.
pub mod resolver;
/// Retry logic with exponential backoff.
pub mod retry;
/// Credential snapshot.
pub mod snapshot;
/// Credential store trait with layered composition.
pub mod store;
/// In-memory credential store for testing.
pub mod store_memory;

// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

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
// Storage layers
#[cfg(any(test, feature = "test-util"))]
pub use layer::StaticKeyProvider;
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer, EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError,
    ScopeLayer, ScopeResolver,
};
// Derive macros
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
// Auth schemes — open trait + 13-variant classification + 12 built-in scheme types
pub use scheme::{
    AuthPattern, AuthScheme, Certificate, ChallengeSecret, ConnectionUri, FederatedAssertion,
    IdentityPassword, InstanceBinding, KeyPair, OAuth2Token, OtpSeed, SecretToken, SharedKey,
    SigningKey,
};
// §12.5 secret-handling primitives — crypto, guard, zeroizing wrappers
pub use secrets::{
    CredentialGuard, EncryptedData, EncryptionKey, SecretString, decrypt, decrypt_with_aad,
    encrypt, encrypt_with_aad, encrypt_with_key_id, generate_code_challenge,
    generate_pkce_verifier, generate_random_state,
};
// Store + in-memory impl
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
pub use store_memory::InMemoryStore;

// Rotation (feature-gated)
#[cfg(feature = "rotation")]
pub use crate::rotation::{
    CredentialRotationEvent, GracePeriodConfig, RotationError, RotationResult,
};
/// Back-compat alias: serde attribute paths
/// `nebula_credential::serde_secret` and `nebula_credential::serde_secret::option`
/// continue to resolve here after the `secrets/` submodule move.
pub use crate::secrets::serde_secret;
// Error / event / metadata / snapshot
pub use crate::{
    error::{
        CredentialError, CryptoError, RefreshErrorKind, ResolutionStage, RetryAdvice,
        ValidationError,
    },
    event::CredentialEvent,
    metadata::{
        CredentialId, CredentialKey, CredentialMetadata, CredentialMetadataBuilder,
        CredentialRecord, MetadataCompatibilityError,
    },
    snapshot::{CredentialSnapshot, SnapshotError},
};
