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
//! ## Quick start
//!
//! Secrets are zeroizing wrappers — redacted in `Debug`, wiped from memory on drop:
//!
//! ```
//! use nebula_credential::SecretString;
//!
//! let token = SecretString::new("xoxb-secret-value".to_owned());
//! assert_eq!(token.expose_secret(), "xoxb-secret-value");
//! // The secret never leaks through Debug formatting:
//! assert!(!format!("{token:?}").contains("xoxb-secret-value"));
//! // `token` is zeroized as it drops at end of scope.
//! ```
//!
//! Action authors bind to a [`Credential`] type that maps stored `Properties`
//! into projected auth material; the engine owns refresh and rotation, so action
//! code never hand-rolls token lifecycles.
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
//! - `nebula_storage_port::CredentialPersistence` — the directly object-safe,
//!   owner-scoped persistence port. Concrete adapters and encryption/cache/audit
//!   decorators live in `nebula-storage`; this crate retains the controller,
//!   never a parallel store trait or dyn bridge.
//! - Runtime resolution (resolver / refresh-coordinator / lease / rotation-state)
//!   lives in this crate's `runtime` module (relocated from `nebula-engine` per
//!   ADR-0092); the engine keeps only the accessor bridges + a test coordinator.
//! - `SecretString`, `CredentialGuard` — zeroizing secret wrappers.
//! - AES-256-GCM primitives (`EncryptedData`, `EncryptionKey`, `encrypt_with_aad`,
//!   `encrypt_with_key_id`, `decrypt`, `decrypt_with_aad`) moved to `nebula-crypto`
//!   (ADR-0088). The AAD-free `encrypt` path is intentionally not exposed (SEC-11).
//! - `#[credential]` (attribute), `#[derive(AuthScheme)]` — authoring macros.
//!
//! ## Security invariant (credential secrecy)
//!
//! Encryption at rest: AES-256-GCM with an operator-supplied 256-bit key and
//! credential ID bound as AAD. `nebula-crypto` also provides Argon2id for
//! password-derived keys, but the default `EnvKeyProvider` consumes raw key
//! material and does not run a KDF. No bypass for debugging. All intermediate
//! plaintext lives in `Zeroizing<Vec<u8>>`; credential `Debug` implementations
//! redact secret and user-controlled display fields.
//!
//! See `crates/credential/README.md` for the full contract and canon invariants.
#![forbid(unsafe_code)]
// Library-first API-surface hardening: a `pub` item unreachable from outside
// the crate should be `pub(crate)` so the public surface stays intentional
// (Tokio's `unreachable_pub` discipline). Additive to inherited workspace lints.
#![warn(unreachable_pub)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

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
/// Credential lifecycle as data — `CredentialPolicy` / `RefreshStrategy` /
/// `RevokeStrategy` (ADR-0088 D2: capabilities are data, not sub-traits).
pub(crate) mod lifecycle;
/// Credential operation metrics — counter names and label helpers.
pub(crate) mod metrics;
/// External credential provider abstraction — delegation to external secret managers.
pub mod provider;
/// Authentication scheme types — AuthScheme trait, AuthPattern, 12 built-in schemes.
pub mod scheme;
/// credential secrecy primitives — guards, zeroizing wrappers, PKCE + serde helpers (AES-256-GCM moved to nebula-crypto).
pub mod secrets;

// ── Flattened modules (previously nested under accessor/ and metadata/) ───

/// Credential accessor stub — NoopCredentialAccessor + default_credential_accessor.
///
/// Engine execution enforces the per-action credential allowlist with its own
/// `EngineCredentialAccessor` (`nebula-engine`); this crate supplies only the
/// no-op default accessor.
mod accessor;
/// Audit trait and value types — [`AuditSink`], [`AuditEvent`],
/// [`AuditOperation`], [`AuditResult`]. The audit decorator (`AuditLayer`)
/// stays in `nebula_storage::credential` and imports these from here.
pub(crate) mod audit;
/// Credential operation context — CredentialContext, CredentialContextBuilder.
mod context;
/// Typed credential reference — `CredentialRef<C>` slot-binding handle (typed ref fields).
mod credential_ref;
/// Per-instance credential display metadata — `CredentialDisplay` (name / description / tags).
mod display;
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

/// Dyn-erasure bridge for generic pending-state storage only.
pub(crate) mod erased;
/// Error types for credential operations.
pub mod error;
/// Credential lifecycle events for cross-crate signaling.
pub(crate) mod event;
/// Pending state store trait for interactive credential flows.
pub mod pending_store;
/// Credential lifecycle orchestration the execution engine drives —
/// resolution executor, capability dispatchers, scoped accessor (ADR-0092,
/// relocated from `nebula-engine::credential`).
pub mod runtime;
/// Credential semantic service plus the authority-bound management controller
/// (ADR-0092, relocated from `nebula-credential-runtime`). Supported
/// authenticated HTTP management enters through the controller; technical
/// runtime/service seams remain direct until K3.
pub(crate) mod service;
/// Credential snapshot.
pub(crate) mod snapshot;

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
    Testable, compute_capabilities,
};
// Resolve types
pub use contract::{
    DisplayData, InteractionRequest, ReauthReason, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestFailureCode, TestResult, UserInput,
};
// Built-in credential implementations
pub use credential_ref::CredentialRef;
pub use credentials::{
    ApiKeyCredential, ApiKeyProperties, BasicAuthCredential, BasicAuthProperties,
    BearerTokenCredential, BearerTokenProperties, OAuth2Credential, OAuth2Pending,
    OAuth2Properties, OAuth2State, SharedKeyCredential, SharedKeyProperties, SigningKeyCredential,
    SigningKeyProperties, register_builtins,
};
pub use handle::CredentialHandle;
pub use metrics::CredentialMetrics;
/// Re-export core's [`CredentialAccessor`] trait as the canonical accessor trait.
pub use nebula_core::accessor::CredentialAccessor;
// Domain identifiers — re-exported from nebula_core for discoverability.
pub use nebula_core::{CredentialId, CredentialKey, credential_key};
// Re-exported so `#[credential]` can emit `::nebula_credential::schema_of`
// without forcing plugin authors onto a direct `nebula-schema` dependency
// (schema-of properties: `Self::Properties: HasSchema` is the single source of truth).
pub use nebula_schema::schema_of;
// Authoring macros. `credential` is the canonical ADR-0088 D1 attribute macro
// (one-impl-block authoring); `AuthScheme` derives the scheme's `AuthPattern`.
// (The legacy `#[derive(Credential)]` was removed — the attribute macro covers
// every case and infers capabilities from method presence.)
pub use nebula_credential_macros::{AuthScheme, credential};
// Opt-out built-in (lives at root, not under credentials::, because it has
// no Input form and is never registered in CredentialRegistry — it's a
// Resource-side type marker per credential isolation).
pub use no_credential::{NoCredential, NoCredentialState};
// Pending-state operations are generic and retain a byte-core dyn bridge.
// Credential persistence itself is directly object-safe in storage-port.
pub use erased::{DynPendingStateStore, ErasedPendingStore};
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
// Refresh coordination (`RefreshCoordinator`, `RefreshDisposition`, …) lives in the
// `runtime::refresh` module of this crate (relocated from `nebula-engine` per
// ADR-0092); reach it via `nebula_credential::runtime::*` rather than a flat
// crate-root re-export.
// Auth schemes — open trait + 11-variant classification + 9 built-in scheme types.
// Pruned 2026-04-24: FederatedAssertion (Plane A), OtpSeed + ChallengeSecret
// (integration-internal, не projected auth material).
pub use scheme::{
    AuthPattern, AuthScheme, AuthStyle, Certificate, CertificateFamily, ConnectionUri,
    ConnectionUriFamily, EgressShape, ExternalScheme, IdentityPassword, IdentityPasswordFamily,
    InstanceBinding, InstanceBindingFamily, KeyPair, KeyPairFamily, OAuth2Family, OAuth2Token,
    PublicScheme, SchemeFamily, SecretToken, SecretTokenFamily, SensitiveScheme, SharedKey,
    SharedKeyFamily, SigningKey, SigningKeyFamily,
};
// credential secrecy secret-handling primitives — crypto, guard, zeroizing wrappers,
// scheme-guard refresh surface (§15.7). The refresh-notification hook
// itself lives on `nebula_resource::resource::Provider::on_credential_refresh`
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
    CredentialLifecycle, CredentialPolicy, Decision, LeaseRef, RefreshStrategy,
    RefreshStrategyKind, RevokeStrategy, SchemeId,
};
// Audit contract — trait + value types (decorator AuditLayer stays in nebula_storage::credential)
pub use audit::{AuditEvent, AuditOperation, AuditResult, AuditSink};

/// Back-compat alias: serde attribute paths
/// `nebula_credential::serde_secret` and `nebula_credential::serde_secret::option`
/// continue to resolve here after the `secrets/` submodule move.
pub use crate::secrets::serde_secret;
// Error / event / metadata / snapshot / identifiers
pub use crate::{
    display::CredentialDisplay,
    error::{
        CredentialAccessError, CredentialError, CryptoError, ProviderErrorContext,
        ProviderErrorKind, RefreshErrorKind, RefreshFailedContext, ResolutionStage, RetryAdvice,
        SchemeMismatch, SecretFreeMessage, ValidationError,
    },
    event::CredentialEvent,
    metadata::{
        CredentialMetadata, CredentialMetadataBuildError, CredentialMetadataBuilder,
        MetadataCompatibilityError,
    },
    record::CredentialRecord,
    snapshot::{CredentialSnapshot, SnapshotError},
};

// CredentialService facade (ADR-0092, relocated from nebula-credential-runtime).
// The `CredentialServiceBuilder` is NOT re-exported here: it pulls in
// `nebula-storage` + `nebula-engine` deps and lives at the api composition root.
pub use service::{
    Acquisition, AuthorizationDecision, CredentialActor, CredentialActorBuildError,
    CredentialActorKind, CredentialAuthenticationBinding, CredentialAuthenticationBindingError,
    CredentialAuthorizationError, CredentialCommand, CredentialCommandResult, CredentialController,
    CredentialControllerError, CredentialDisplayPatch, CredentialHead, CredentialObserver,
    CredentialOperation, CredentialService, CredentialServiceError, CredentialTenantAuthority,
    CredentialTypeInfo, CredentialValidationIssue, CredentialValidationReport, DispatchError,
    DispatchOps, EventMetricObserver, NoopObserver, RefreshReport, StateSource, TenantFingerprint,
    TenantScope, TypeCapabilities, ValidatedCredentialBinding, ValidatedCredentialBindingError,
    register_all_builtin_ops, register_interactive_ops, register_refreshable_ops,
    register_revocable_ops, register_runtime_ops, register_testable_ops,
};

// ── Prelude ───────────────────────────────────────────────────────────────────

/// Prelude — import this for the most common credential types.
///
/// ```rust
/// use nebula_credential::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        AuthPattern, AuthScheme, Credential, CredentialContext, CredentialContextBuilder,
        CredentialError, CredentialGuard, CredentialHandle, CredentialId, CredentialKey,
        CredentialMetadata, CredentialPolicy, CredentialRecord, CredentialRegistry,
        CredentialService, CredentialState, Dynamic, ExternalScheme, Interactive, PublicScheme,
        Refreshable, Revocable, SecretString, SensitiveScheme, Testable, credential,
        credential_key, schema_of,
    };
}

// Internal imports of the Core-tier persistence contract. These names are not
// re-exported from the credential product surface; adapters and composition
// roots depend on `nebula-storage-port` directly.
pub(crate) use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCreate, CredentialPersistence,
    CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialTombstone,
    CredentialVersion, StoredCredential, StoredCredentialHead, StoredLiveCredential,
};

// Credential-owned metadata conventions. Persistence authority comes only
// from the selector/owner column; these values are semantic state maintained
// by the credential service and are never interpreted as authority by storage.
pub(crate) const OWNER_ID_METADATA_KEY: &str = "owner_id";
pub(crate) const LAST_VALIDATED_AT_METADATA_KEY: &str = "last_validated_at";
