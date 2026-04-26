//! Reshaped `Resource` trait — Strategy §4.1 / Tech Spec §3.6 / ADR-0036.
//!
//! Differences vs. the live `crates/resource/src/resource.rs`:
//!
//! - **`type Auth: AuthScheme`** is gone. Replaced by **`type Credential: Credential`**, with
//!   `<Self::Credential as Credential>::Scheme` flowing into `create` and into the new rotation
//!   hooks.
//! - Two new lifecycle hooks: `on_credential_refresh` (default no-op, override with blue-green pool
//!   swap per Tech Spec §3.6) and `on_credential_revoke` (default no-op, override invariant:
//!   post-call, the resource emits no further authenticated traffic on the revoked credential —
//!   Strategy §4.2).
//!
//! Spike is intentionally minimum surface — it omits `check`/`shutdown`/
//! `destroy` lifecycle methods that are orthogonal to the §3.6 shape
//! validation. Phase 6 Tech Spec §3 will add them back.

use std::future::Future;

use nebula_credential::{Credential, CredentialId};

/// Stand-in for the production `ResourceContext`. Spike doesn't need the
/// real one; downstream Tech Spec keeps the same shape (`<'_>` lifetime,
/// reaching into the manager's clock / event bus / metrics handle).
#[derive(Debug, Default)]
pub struct ResourceContext;

/// Stand-in for `nebula_core::ResourceKey`. Real `ResourceKey` is a
/// validated `domain_key` ULID/string; spike uses a thin wrapper to keep
/// the surface compile-clean without dragging the full `nebula-core`
/// dep tree.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceKey(pub &'static str);

/// The Phase 4 spike trait shape — minimum surface to validate Strategy
/// §3.6.
///
/// Intentionally lighter than the final trait: spike includes only the
/// methods that interact with the credential type. `check`, `shutdown`,
/// `destroy`, and the metadata machinery are deferred to Phase 6 — they
/// don't gate the §3.6 ergonomic decision.
pub trait Resource: Send + Sync + 'static {
    /// Operational config (no secrets). Spike keeps it associated-type
    /// open so each topology impl can pick a real type without forcing
    /// a `HasSchema` bound (the production trait does require it).
    type Config: Send + Sync + 'static;

    /// The live resource handle. Pool, client, channel, etc.
    type Runtime: Send + Sync + 'static;

    /// What callers hold during use.
    type Lease: Send + Sync + 'static;

    /// Resource-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// What the engine binds at this resource.
    ///
    /// Use [`crate::NoCredential`] to opt out (`type Credential =
    /// NoCredential;`). Otherwise pick a real `Credential` impl.
    /// `<Self::Credential as Credential>::Scheme` is what `create` /
    /// `on_credential_refresh` get from the credential resolver.
    type Credential: Credential;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config and resolved scheme.
    ///
    /// `scheme` is borrowed from the credential resolver — implementations
    /// must NOT clone it onto the runtime; pull whatever they need (token,
    /// connection string, etc.) and let the borrow end.
    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Optional rotation hook — called when the engine detects that the
    /// bound credential's scheme changed (refresh or rotation).
    ///
    /// Default is no-op. Connection-bound resources (Postgres pool,
    /// Kafka producer) override with the blue-green swap pattern from
    /// Tech Spec §3.6 lines 961-993:
    /// `Arc<RwLock<Pool>>` + write-lock swap.
    ///
    /// Invariant: per-resource isolation. The dispatcher (see
    /// [`crate::Manager`]) gives each resource its own future and timeout
    /// budget; one resource's slow / failed refresh must NOT block siblings.
    fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = new_scheme;
        async { Ok(()) }
    }

    /// Optional revocation hook — called when the engine signals that
    /// the bound credential has been revoked.
    ///
    /// Default is no-op. Override invariant per Strategy §4.2: post-
    /// invocation, the resource emits no further authenticated traffic on
    /// the revoked credential. The mechanism (destroy pool / mark tainted
    /// / wait-for-drain / reject new acquires) is impl-defined.
    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }
}
