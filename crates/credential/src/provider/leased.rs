//! `LeasedProvider` sub-trait — renew / revoke for time-bounded grants.
//!
//! Per external provider the base [`ExternalProvider`] surface stays oblivious to
//! lease lifecycle: it only resolves secrets and carries optional lease
//! metadata on the [`ProviderResolution`] envelope. Lease-aware providers
//! (HashiCorp Vault dynamic secrets, AWS STS-issued temporary creds, GCP
//! short-lived service-account keys) implement this companion trait to
//! expose lifecycle operations against an issued [`LeaseHandle`].
//!
//! # Capability discovery (no runtime downcasts)
//!
//! Mirrors the sub-trait pattern used by [`Refreshable`] (Tech Spec §15.4):
//! capability is advertised by an [`ExternalProvider::lease_renewal`]
//! method that returns `Option<&dyn LeasedProvider>`. The default impl
//! returns `None`; leased providers override it to return `Some(self)`.
//! Composed providers (e.g. [`ExternalProviderChain`], `ProviderCacheLayer`)
//! forward the call to their inner. The caller never downcasts.
//!
//! # Dyn-safety
//!
//! Like [`ExternalProvider`], `LeasedProvider` is dyn-safe: both methods
//! return the concrete [`ProviderFuture<'a>`] envelope (not `impl Future`,
//! which is not object-safe), so `Arc<dyn LeasedProvider>` is supported.
//!
//! [`Refreshable`]: crate::Refreshable
//! [`ExternalProvider`]: super::ExternalProvider
//! [`ExternalProviderChain`]: super::ExternalProviderChain
//! [`ProviderResolution`]: super::ProviderResolution
//! [`LeaseHandle`]: super::LeaseHandle

use super::{ExternalProvider, LeaseHandle, ProviderFuture};

/// External providers that issue time-bounded grants and accept renew / revoke
/// operations against them.
///
/// Implementations of this trait MUST also implement [`ExternalProvider`].
/// The resolve path stays on the base trait; this sub-trait only adds the
/// lifecycle operations a lease-aware backend offers on top.
///
/// # Method semantics
///
/// - [`renew`](Self::renew) — extend the lease's TTL. On success the returned
///   [`ProviderResolution`](super::ProviderResolution) carries the refreshed
///   lease metadata (new `issued_at` / `ttl`) so a downstream cache layer can
///   reset its expiration in lockstep with the provider.
/// - [`revoke`](Self::revoke) — explicitly tear down the lease so the backing
///   secret is invalidated before its TTL would otherwise elapse. The
///   resolution value returned on success is conventionally a no-secret
///   marker (similar to [`ExternalProvider::health_check`]); callers MUST
///   NOT consume it as a real secret.
///
/// Errors classify the same way as on [`ExternalProvider::resolve`] —
/// [`ProviderError::NotFound`](super::ProviderError::NotFound) means the
/// lease is unknown to the backend (treat as already-gone for revoke);
/// every other variant is a hard error.
///
/// # Capability discovery
///
/// To make a provider participate in lease lifecycle without runtime
/// downcasts, override [`ExternalProvider::lease_renewal`] to return
/// `Some(self)`. The default returns `None`, so providers that do not
/// implement leasing are transparently a no-op for chain / cache layer
/// forwarding.
///
/// # Examples
///
/// ```
/// use nebula_credential::{
///     ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider,
///     ProviderFuture, ProviderResolution, SecretString,
/// };
///
/// // `ExternalProvider` requires `Debug`.
/// #[derive(Debug)]
/// struct VaultProvider;
///
/// impl ExternalProvider for VaultProvider {
///     fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
///         ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
///             "dynamic-secret",
///         ))))
///     }
///     fn provider_name(&self) -> &str {
///         "vault"
///     }
///     // Declare lease capability via the base-trait override — callers
///     // discover it through `lease_renewal`, never a runtime downcast.
///     fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
///         Some(self)
///     }
/// }
///
/// impl LeasedProvider for VaultProvider {
///     fn renew<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
///         ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
///             "renewed-secret",
///         ))))
///     }
///     fn revoke<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
///         ProviderFuture::ready(Ok(ProviderResolution::empty()))
///     }
/// }
///
/// let provider = VaultProvider;
/// assert_eq!(provider.provider_name(), "vault");
/// assert!(provider.lease_renewal().is_some());
/// ```
pub trait LeasedProvider: ExternalProvider {
    /// Whether this provider issued the given lease and is the correct
    /// target for renew/revoke. Default: name match against
    /// [`LeaseHandle::provider`](super::LeaseHandle::provider).
    ///
    /// Composed providers ([`ExternalProviderChain`](super::ExternalProviderChain),
    /// `ProviderCacheLayer`) override this to delegate the decision to their
    /// inner — a chain asks each child, a cache layer asks the provider it
    /// wraps. The default keeps the common case (single backend with a
    /// stable name) one-line.
    fn handles_lease(&self, lease: &LeaseHandle) -> bool {
        self.provider_name() == lease.provider.as_ref()
    }

    /// Extend the lease's TTL. Returns the refreshed lease metadata in a
    /// [`ProviderResolution`](super::ProviderResolution); the `secret` field
    /// is implementation-defined — providers MAY return the same secret or a
    /// rolled value depending on backend semantics.
    ///
    /// Implementations SHOULD reject leases for which
    /// [`handles_lease`](Self::handles_lease) returns `false` — typically
    /// with [`ProviderError::NotFound`](super::ProviderError::NotFound) —
    /// rather than acting on an unrelated lease. The chain / cache layer
    /// guarantee correct routing on their side, but a misrouted call
    /// through a hand-built dispatcher should still surface as an error.
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a>;

    /// Tear down the lease so the backing secret is invalidated immediately.
    /// On success the returned resolution carries no usable secret; prefer
    /// [`ProviderResolution::empty`](super::ProviderResolution::empty) for
    /// the success value.
    ///
    /// Implementations SHOULD reject leases for which
    /// [`handles_lease`](Self::handles_lease) returns `false` — see
    /// [`renew`](Self::renew) for the rationale.
    fn revoke<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a>;
}

#[cfg(test)]
#[path = "leased_tests.rs"]
mod tests;
