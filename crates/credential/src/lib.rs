//! # nebula-credential
//!
//! **Role:** Credential Contract — stored state vs projected auth material;
//! engine-owned rotation and refresh. Integration-model boundary; credential secrecy.
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
//! - `Credential` — base trait: `resolve()`, `project()`. Capability methods
//!   (`continue_resolve`, `refresh`, `revoke`, `test`, `release`) live on dedicated sub-traits per
//!   Tech Spec §15.4 — `Interactive`, `Refreshable`, `Revocable`, `Testable`, `Dynamic`. Phase 5 of
//!   the M6 redesign renamed `Credential::Input` → `Credential::Properties` to mirror
//!   `Action::Input` / `Resource::Config`; the `Properties: HasSchema` bound is the single
//!   source of truth, read via `nebula_schema::schema_of::<C::Properties>()` (schema-of properties — no
//!   per-trait schema method).
//! - `CredentialMetadata` — static type descriptor: key, name, schema, `AuthPattern`.
//! - `CredentialRecord` — runtime operational state (created_at, version, expiry, tags). Previously
//!   named `Metadata` (ADR 0004).
//! - `CredentialStore` — persistence trait. Concrete impls + composable layers (`EncryptionLayer`,
//!   `CacheLayer`, `AuditLayer`) live in `nebula_storage::credential` per storage credential layers; the multi-tenant
//!   scope layer was re-homed to `nebula_tenancy::CredentialScopeLayer` (spec §8).
//! - Engine-owned runtime resolution lives in `nebula-engine::credential`.
//! - `SecretString`, `CredentialGuard` — zeroizing secret wrappers.
//! - AES-256-GCM primitives (`EncryptedData`, `EncryptionKey`, `encrypt_with_aad`,
//!   `encrypt_with_key_id`, `decrypt`, `decrypt_with_aad`) moved to `nebula-crypto`
//!   (ADR-0088). The AAD-free `encrypt` path is intentionally not exposed (SEC-11).
//! - `#[derive(Credential)]`, `#[derive(AuthScheme)]` — proc-macro derivations.
//!
//! ## Security invariant (credential secrecy)
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
/// Credential lifecycle as data — `CredentialPolicy` / `RefreshStrategy` /
/// `RevokeStrategy` / `CredentialCategory` (ADR-0088 D2: capabilities are data,
/// not sub-traits).
pub mod lifecycle;
/// Credential operation metrics — counter names and label helpers.
pub mod metrics;
/// External credential provider abstraction — delegation to external secret managers.
pub mod provider;
/// Credential rotation (blue-green, transaction, state machine).
#[cfg(feature = "rotation")]
pub mod rotation;
/// Authentication scheme types — AuthScheme trait, AuthPattern, 12 built-in schemes.
pub mod scheme;
/// credential secrecy primitives — guards, zeroizing wrappers, PKCE + serde helpers (AES-256-GCM moved to nebula-crypto).
pub mod secrets;

// ── Flattened modules (previously nested under accessor/ and metadata/) ───

/// Credential accessor stub — NoopCredentialAccessor + default_credential_accessor.
///
/// The engine-runtime allowlist-enforcing accessor lives in
/// `nebula_engine::credential::ScopedCredentialAccessor`.
mod accessor;
/// Credential operation context — CredentialContext, CredentialContextBuilder.
mod context;
/// Typed credential reference — `CredentialRef<C>` slot-binding handle (typed ref fields).
mod credential_ref;
/// Typed credential handle — CredentialHandle (ArcSwap-backed).
mod handle;
/// Credential metadata — static type descriptor (CredentialMetadata, builder, compat).
mod metadata;
/// `NoCredential` opt-out type — for resources without an authenticated binding (credential isolation).
mod no_credential;
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
/// Credential snapshot.
pub mod snapshot;
/// Credential store trait with layered composition.
pub mod store;

// ── Backward-compat re-export: `nebula_credential::resolve::*` ──────────
// The proc-macro and downstream crates reference `nebula_credential::resolve::`.
// The module now lives inside `contract/resolve`; this re-export keeps the
// path intact.
// ── Root re-exports ─────────────────────────────────────────────────────────
// Commonly-used types available directly as `nebula_credential::TypeName`.

// Consumer-facing accessor surface — trait (re-exported from core), impls, handle, context,
// access error
pub use accessor::{NoopCredentialAccessor, default_credential_accessor};
pub use context::{CredentialContext, CredentialContextBuilder};
pub use contract::resolve;
// Credential contract — Credential trait + associated types
pub use contract::{
    AnyCredential, Capabilities, Credential, CredentialRegistry, CredentialState, Dynamic,
    Interactive, NoPendingState, PendingState, PendingToken, Refreshable, RegisterError, Revocable,
    StaticProtocol, Testable, compute_capabilities,
};
// Resolve types
pub use contract::{
    DisplayData, InteractionRequest, ReauthReason, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
// Built-in credential implementations
pub use credential_ref::CredentialRef;
pub use credentials::{
    ApiKeyCredential, ApiKeyProperties, BasicAuthCredential, BasicAuthProperties, OAuth2Credential,
    OAuth2Pending, OAuth2Properties, OAuth2State,
};
pub use handle::CredentialHandle;
pub use metrics::CredentialMetrics;
/// Re-export core's [`CredentialAccessor`] trait as the canonical accessor trait.
pub use nebula_core::accessor::CredentialAccessor;
// Domain identifiers — re-exported from nebula_core for discoverability.
pub use nebula_core::{CredentialId, CredentialKey, credential_key};
// Re-exported so `#[derive(Credential)]` can emit `::nebula_credential::schema_of`
// without forcing plugin authors onto a direct `nebula-schema` dependency
// (schema-of properties: `Self::Properties: HasSchema` is the single source of truth).
pub use nebula_schema::schema_of;
// Derive + attribute macros. `credential` is the ADR-0088 D1 attribute macro
// (one-impl-block authoring); `Credential` is the legacy derive kept during
// the migration window.
pub use nebula_credential_macros::{AuthScheme, Credential, credential};
// Opt-out built-in (lives at root, not under credentials::, because it has
// no Input form and is never registered in CredentialRegistry — it's a
// Resource-side type marker per credential isolation).
pub use no_credential::{NoCredential, NoCredentialState};
// Pending state store
pub use pending_store::{PendingStateStore, PendingStoreError};
// External provider abstraction (redesigned per external provider):
// - Trait & data types (ExternalProvider, ExternalReference, ProviderError, ProviderKind)
// - Future newtype (ProviderFuture) for dyn-safe + zero-alloc-ready resolve
// - Envelope (ProviderResolution, LeaseHandle) carrying secret + lease + TTL
// - Composition (ExternalProviderChain) with error-discriminated fallback
// - Lease lifecycle (LeasedProvider) — renew / revoke, capability-discovered
//   via ExternalProvider::lease_renewal (no runtime downcasts).
pub use provider::{
    ExternalProvider, ExternalProviderChain, ExternalReference, LeaseEvent, LeaseExpiryReason,
    LeaseHandle, LeasedProvider, ProviderError, ProviderFuture, ProviderKind, ProviderResolution,
};
// Refresh coordination — moved to nebula-engine::credential::refresh (engine credential orchestration §3 amendment)
// Re-exports removed: RefreshAttempt, RefreshCoordinator now live in nebula-engine.
// Auth schemes — open trait + 11-variant classification + 9 built-in scheme types.
// Pruned 2026-04-24: FederatedAssertion (Plane A), OtpSeed + ChallengeSecret
// (integration-internal, не projected auth material).
pub use scheme::{
    AuthPattern, AuthScheme, AuthStyle, Certificate, ConnectionUri, IdentityPassword,
    InstanceBinding, KeyPair, OAuth2Token, PublicScheme, SecretToken, SensitiveScheme, SharedKey,
    SigningKey,
};
// credential secrecy secret-handling primitives — crypto, guard, zeroizing wrappers,
// scheme-guard refresh surface (§15.7). The refresh-notification hook
// itself lives on `nebula_resource::Resource::on_credential_refresh`
// per credential isolation; the previously-defined parallel `OnCredentialRefresh<C>`
// trait was removed in nebula-resource П2.
//
// AES-256-GCM primitives moved to `nebula-crypto` (ADR-0088); import them from
// there. PKCE/state helpers + secret wrappers stay here.
pub use secrets::{
    CredentialGuard, ExposeSecret, ExposeSecretMut, RedactedSecret, SchemeFactory, SchemeGuard,
    SecretBox, SecretString, generate_code_challenge, generate_pkce_verifier,
    generate_random_state, secret_from_string,
};
// Lifecycle policy types (ADR-0088 D2): capabilities as data, not sub-traits.
pub use lifecycle::{
    CredentialCategory, CredentialLifecycle, CredentialPolicy, LeaseRef, RefreshStrategy,
    RevokeStrategy,
};
// Store trait + DTOs (canonical impls live in `nebula_storage::credential` per storage credential layers)
pub use store::{CredentialStore, PutMode, ScopeResolver, StoreError, StoredCredential};

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
        CredentialAccessError, CredentialError, CryptoError, ProviderErrorContext,
        ProviderErrorKind, RefreshErrorKind, RefreshFailedContext, ResolutionStage, RetryAdvice,
        RevokeErrorKind, RevokeFailedContext, SchemeIdentity, SchemeKind, SchemeMismatch,
        SecretFreeMessage, ValidationError,
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
        // Sensitivity dichotomy (§15.5)
        PublicScheme,
        // Secrets
        SecretString,
        SensitiveScheme,
        credential_key,
    };
}
