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
//! - `Credential` — unified trait: `resolve()`, `refresh()`, `test()`, `project()`, `schema()`.
//! - `CredentialMetadata` — static type descriptor: key, name, schema, `AuthPattern`.
//! - `CredentialRecord` — runtime operational state (created_at, version, expiry, tags). Previously
//!   named `Metadata` (ADR 0004).
//! - `CredentialStore` — persistence trait. Concrete impls + composable layers (`EncryptionLayer`,
//!   `CacheLayer`, `AuditLayer`, `ScopeLayer`) live in `nebula_storage::credential` per ADR-0032.
//! - Engine-owned runtime resolution lives in `nebula-engine::credential`.
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
// Free-standing concerns: errors, resolve pipeline, storage, refresh coordinator, etc.

/// Error types for credential operations.
pub mod error;
/// Credential lifecycle events for cross-crate signaling (EventBus payload).
pub mod event;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store for testing and development.
pub mod pending_store_memory;
/// Refresh coordination — thundering herd prevention.
pub mod refresh;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
/// Retry logic with exponential backoff.
pub mod retry;
/// Credential snapshot.
pub mod snapshot;
/// Credential store trait with layered composition.
pub mod store;
/// In-memory `CredentialStore` impl for testing and internal use.
///
/// **Not** the canonical production impl — that lives in
/// `nebula_storage::credential::InMemoryStore` per
/// [ADR-0032](https://github.com/vanyastaff/nebula/blob/main/docs/adr/0032-credential-store-canonical-home.md).
/// Kept here because crate-internal code (OAuth2 credential impl, refresh
/// tests, unit tests under `crates/credential/tests/`) references it directly
/// and cannot depend on `nebula-storage` —
/// ADR-0032 §3 forbids `nebula-credential → nebula-storage` in either
/// `[dependencies]` or `[dev-dependencies]` (the latter triggers a
/// two-copies cargo resolution that breaks trait bounds).
///
/// Production consumers and composition roots should prefer
/// `nebula_storage::credential::InMemoryStore`; both implementations are
/// behaviour-identical.
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
// Derive macros
pub use nebula_credential_macros::{AuthScheme, Credential};
// Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
pub use pending_store_memory::InMemoryPendingStore;
// Refresh coordination
pub use refresh::{RefreshAttempt, RefreshCoordinator};
// Resolve types
pub use resolve::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
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
// Store trait + DTOs (canonical impls live in `nebula_storage::credential` per ADR-0032)
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
// In-memory impl — behaviour-identical to `nebula_storage::credential::InMemoryStore`.
// Kept here to avoid a dep cycle; production consumers should prefer the storage copy.
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
