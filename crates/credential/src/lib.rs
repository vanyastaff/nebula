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
//! `secrets`, `credentials`) are `pub` for escape hatches, but the canonical
//! public surface is **flat re-exports at the root**. Prefer
//! `use nebula_credential::SecretString;` over
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

/// Credential contract surface — Credential trait + associated types + resolve types.
pub mod contract;
/// Built-in credential type implementations.
pub mod credentials;
/// Typed credential extension trait for capability contexts.
pub mod ext;
/// Credential operation metrics — counter names and label helpers.
pub mod metrics;
/// External credential provider abstraction — delegation to external secret managers.
pub mod provider;
/// Credential rotation (blue-green, transaction, state machine).
#[cfg(feature = "rotation")]
pub mod rotation;
/// Authentication scheme types — AuthScheme trait, AuthPattern, 12 built-in schemes.
pub mod scheme;
/// §12.5 secret-handling primitives — AES-256-GCM, guards, zeroizing wrappers, serde helpers.
pub mod secrets;

// ── Flattened modules (previously nested under accessor/ and metadata/) ───

/// Credential accessor implementations — NoopCredentialAccessor, ScopedCredentialAccessor.
mod accessor;
/// Credential operation context — CredentialContext, CredentialContextBuilder.
mod context;
/// Typed credential handle — CredentialHandle (ArcSwap-backed).
mod handle;
/// Credential metadata — static type descriptor (CredentialMetadata, builder, compat).
mod metadata;
/// Credential record — runtime operational state (timestamps, version, tags).
mod record;

// ── Utility modules ─────────────────────────────────────────────────────────
// Free-standing concerns: errors, storage, refresh coordinator, etc.

/// Error types for credential operations.
pub mod error;
/// Credential lifecycle events for cross-crate signaling.
pub mod event;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// In-memory pending state store — **test shim only**.
///
/// The canonical production impl lives in
/// `nebula_storage::credential::InMemoryPendingStore` per ADR-0032.
/// This copy exists solely for credential's own `#[cfg(test)]` code
/// which cannot depend on `nebula-storage` (dep-cycle, ADR-0032 §3).
#[cfg(any(test, feature = "test-util"))]
pub mod pending_store_memory;
/// Credential snapshot.
pub mod snapshot;
/// Credential store trait with layered composition.
pub mod store;
/// In-memory `CredentialStore` impl — **test shim only**.
///
/// The canonical production impl lives in
/// `nebula_storage::credential::InMemoryStore` per ADR-0032.
/// This copy exists solely for credential's own `#[cfg(test)]` code
/// which cannot depend on `nebula-storage` (dep-cycle, ADR-0032 §3).
#[cfg(any(test, feature = "test-util"))]
pub mod store_memory;

// ── Backward-compat re-export: `nebula_credential::resolve::*` ──────────
// The proc-macro and downstream crates reference `nebula_credential::resolve::`.
// The module now lives inside `contract/resolve`; this re-export keeps the
// path intact.
// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Consumer-facing accessor surface — trait (re-exported from core), impls, handle, context,
// access error
pub use accessor::{NoopCredentialAccessor, ScopedCredentialAccessor, default_credential_accessor};
pub use context::{CredentialContext, CredentialContextBuilder};
pub use contract::resolve;
// Credential contract — Credential trait + associated types
pub use contract::{
    AnyCredential, Credential, CredentialState, NoPendingState, PendingState, PendingToken,
    StaticProtocol,
};
// Resolve types
pub use contract::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
// Built-in credential implementations
pub use credentials::{
    ApiKeyCredential, BasicAuthCredential, OAuth2Credential, OAuth2Pending, OAuth2State,
};
pub use handle::CredentialHandle;
pub use metrics::CredentialMetrics;
/// Re-export core's [`CredentialAccessor`] trait as the canonical accessor trait.
pub use nebula_core::accessor::CredentialAccessor;
// Domain identifiers — re-exported from nebula_core for discoverability.
pub use nebula_core::{CredentialId, CredentialKey, credential_key};
// Derive macros
pub use nebula_credential_macros::{AuthScheme, Credential};
// Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
#[cfg(any(test, feature = "test-util"))]
pub use pending_store_memory::InMemoryPendingStore;
// External provider abstraction
pub use provider::{ExternalProvider, ExternalReference, ProviderError, ProviderKind};
// Refresh coordination — moved to nebula-engine::credential::refresh (ADR-0030 §3 amendment)
// Re-exports removed: RefreshAttempt, RefreshCoordinator now live in nebula-engine.
// Auth schemes — open trait + 11-variant classification + 9 built-in scheme types.
// Pruned 2026-04-24: FederatedAssertion (Plane A), OtpSeed + ChallengeSecret
// (integration-internal, не projected auth material).
pub use scheme::{
    AuthPattern, AuthScheme, Certificate, ConnectionUri, IdentityPassword, InstanceBinding,
    KeyPair, OAuth2Token, SecretToken, SharedKey, SigningKey,
};
// §12.5 secret-handling primitives — crypto, guard, zeroizing wrappers
pub use secrets::{
    CredentialGuard, EncryptedData, EncryptionKey, RedactedSecret, SecretString, decrypt,
    decrypt_with_aad, encrypt, encrypt_with_aad, encrypt_with_key_id, generate_code_challenge,
    generate_pkce_verifier, generate_random_state,
};
// Store trait + DTOs (canonical impls live in `nebula_storage::credential` per ADR-0032)
pub use store::{CredentialStore, PutMode, StoreError, StoredCredential};
// In-memory impl — test shim only; production consumers use
// `nebula_storage::credential::InMemoryStore` (ADR-0032).
#[cfg(any(test, feature = "test-util"))]
pub use store_memory::InMemoryStore;

// Rotation (feature-gated)
#[cfg(feature = "rotation")]
pub use crate::rotation::{CredentialRotationEvent, RotationError, RotationResult};
/// Back-compat alias: serde attribute paths
/// `nebula_credential::serde_secret` and `nebula_credential::serde_secret::option`
/// continue to resolve here after the `secrets/` submodule move.
pub use crate::secrets::serde_secret;
// Error / event / metadata / snapshot / identifiers
pub use crate::{
    error::{
        CredentialAccessError, CredentialError, CryptoError, RefreshErrorKind, ResolutionStage,
        RetryAdvice, ValidationError,
    },
    event::CredentialEvent,
    ext::HasCredentialsExt,
    metadata::{
        CredentialMetadata, CredentialMetadataBuildError, CredentialMetadataBuilder,
        MetadataCompatibilityError,
    },
    record::CredentialRecord,
    snapshot::{CredentialSnapshot, SnapshotError},
};

// ── Prelude ───────────────────────────────────────────────────────────────────

/// Prelude — import this for the most common credential types.
///
/// ```rust
/// use nebula_credential::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        AuthPattern,
        AuthScheme,
        // Core contract
        Credential,
        // Context
        CredentialContext,
        CredentialContextBuilder,
        // Errors
        CredentialError,
        // Guards and handles
        CredentialGuard,
        CredentialHandle,
        // IDs
        CredentialId,
        CredentialKey,
        // Metadata
        CredentialMetadata,
        CredentialState,
        HasCredentialsExt,
        // Secrets
        SecretString,
        credential_key,
    };
}
