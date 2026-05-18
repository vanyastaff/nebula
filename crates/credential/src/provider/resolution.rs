//! Resolution envelope returned by [`ExternalProvider`](super::ExternalProvider).
//!
//! Mirrors the AWS `Identity` envelope (`aws-smithy-runtime-api`
//! `client::identity::Identity`) at a much smaller scope: a secret plus the
//! operational metadata a cache layer needs to do its job without further trait
//! changes. Currently carries the secret, an optional lease handle (HashiCorp
//! Vault style), and an optional TTL hint. `#[non_exhaustive]` lets us grow
//! additional fields (e.g. typed `properties` sidecar) without breaking
//! implementors.

use std::{borrow::Cow, time::Duration};

use crate::SecretString;

/// Outcome of a successful [`ExternalProvider::resolve`](super::ExternalProvider::resolve).
///
/// Static providers (env-var, in-memory) build this via
/// [`ProviderResolution::from_secret`]. Lease-aware providers (Vault dynamic
/// secrets, AWS STS-issued credentials) populate [`lease`](Self::lease) and
/// [`ttl`](Self::ttl); a downstream `ProviderCacheLayer` (in `nebula-storage`)
/// uses the TTL to bound how long the secret may be cached.
///
/// # Fields
///
/// - [`secret`](Self::secret) — the resolved secret value.
/// - [`lease`](Self::lease) — `Some` when the provider issued a time-bounded grant.
///   The caller may renew or revoke via the (future) `LeasedProvider` sub-trait.
/// - [`ttl`](Self::ttl) — caching hint. **`None` means the provider supplies
///   no TTL** (static secrets, env vars). The consumer chooses the policy:
///   `ProviderCacheLayer` falls back to its configured `default_ttl`, treating
///   `default_ttl: ZERO` as "pass through". A cache layer MUST honour
///   `Some(d)` and refresh no later than `d` after resolution.
///
/// `Clone` is required by downstream cache layers (moka's value type bound).
/// Cloning a resolution clones the `SecretString` (and the lease, if any);
/// callers that want to share the underlying allocation cheaply should wrap
/// the resolution in `Arc<ProviderResolution>` themselves.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProviderResolution {
    /// The resolved secret value.
    pub secret: SecretString,
    /// Optional lease for providers that issue time-bounded grants
    /// (HashiCorp Vault leased secrets, AWS STS-issued temporary creds).
    pub lease: Option<LeaseHandle>,
    /// Suggested time-to-live for the resolved secret. `None` means the
    /// provider has no TTL hint — the consumer (e.g.
    /// `ProviderCacheLayer` in `nebula-storage`, future work per external provider)
    /// applies its own default, which may itself be "do not cache".
    pub ttl: Option<Duration>,
}

impl ProviderResolution {
    /// Convenience constructor for static providers without lease / TTL
    /// metadata. Whether the result is cached is up to the consumer:
    /// `ProviderCacheLayer` with the default `default_ttl: Duration::ZERO`
    /// will not cache it; configured with a positive default it will.
    #[must_use]
    pub fn from_secret(secret: SecretString) -> Self {
        Self {
            secret,
            lease: None,
            ttl: None,
        }
    }

    /// Convenience constructor for lease-bearing resolutions.
    ///
    /// The TTL is derived from the lease's `ttl` field, so a cache layer
    /// will refresh in lockstep with the provider's lease lifetime.
    #[must_use]
    pub fn with_lease(secret: SecretString, lease: LeaseHandle) -> Self {
        let ttl = Some(lease.ttl);
        Self {
            secret,
            lease: Some(lease),
            ttl,
        }
    }

    /// Marker resolution with no usable secret — public constructor for the
    /// "operation succeeded without producing a secret" return value. Used by
    /// the default [`ExternalProvider::health_check`](super::ExternalProvider::health_check)
    /// implementation and recommended for [`LeasedProvider::revoke`](super::LeasedProvider::revoke)
    /// success returns, where the resolution carries no meaningful secret.
    ///
    /// Callers MUST NOT consume the [`secret`](Self::secret) field of an
    /// `empty()` resolution as a real secret — it is an empty
    /// [`SecretString`] purely so the value satisfies the struct shape
    /// without an `Option` allocation on every health probe.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            secret: SecretString::new(String::new()),
            lease: None,
            ttl: None,
        }
    }

    /// Backwards-compatible alias for [`empty`](Self::empty).
    pub(crate) fn health_ok() -> Self {
        Self::empty()
    }
}

/// Handle to a lease issued by a [`ExternalProvider`](super::ExternalProvider).
///
/// Carried in [`ProviderResolution::lease`]; passed to
/// [`LeasedProvider::renew`](super::LeasedProvider::renew) /
/// [`LeasedProvider::revoke`](super::LeasedProvider::revoke) at lifecycle
/// time. The [`provider`](Self::provider) field carries attribution so
/// composed providers (chain, cache layer) can route lifecycle calls back
/// to the issuing backend — without it, a multi-leased
/// [`ExternalProviderChain`](super::ExternalProviderChain) could
/// misdispatch renew/revoke to the wrong child.
///
/// Construction via [`LeaseHandle::new`] is the canonical public path;
/// `#[non_exhaustive]` reserves additive fields (e.g. backend-specific
/// metadata) without further break.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LeaseHandle {
    /// Name of the provider that issued this lease — matches
    /// [`ExternalProvider::provider_name`](super::ExternalProvider::provider_name)
    /// of the issuer. Used by
    /// [`LeasedProvider::handles_lease`](super::LeasedProvider::handles_lease)
    /// for routing inside composed providers.
    pub provider: Cow<'static, str>,
    /// Provider-specific lease identifier (e.g. Vault lease id).
    pub lease_id: String,
    /// When the provider issued this lease.
    pub issued_at: chrono::DateTime<chrono::Utc>,
    /// Time-to-live communicated by the provider at issue time.
    pub ttl: Duration,
}

impl LeaseHandle {
    /// Build a lease handle with the issuing provider's name as attribution.
    ///
    /// `provider` should match the issuer's
    /// [`ExternalProvider::provider_name`](super::ExternalProvider::provider_name)
    /// so chain / cache routing through
    /// [`LeasedProvider::handles_lease`](super::LeasedProvider::handles_lease)
    /// finds the right backend.
    #[must_use]
    pub fn new(
        provider: impl Into<Cow<'static, str>>,
        lease_id: impl Into<String>,
        issued_at: chrono::DateTime<chrono::Utc>,
        ttl: Duration,
    ) -> Self {
        Self {
            provider: provider.into(),
            lease_id: lease_id.into(),
            issued_at,
            ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_secret_has_no_lease_or_ttl() {
        let r = ProviderResolution::from_secret(SecretString::new("sk-test"));
        assert!(r.lease.is_none());
        assert!(r.ttl.is_none());
        assert_eq!(r.secret.expose_secret(), "sk-test");
    }

    #[test]
    fn with_lease_derives_ttl_from_lease() {
        let lease = LeaseHandle::new(
            "vault",
            "vault-lease-abc",
            chrono::Utc::now(),
            Duration::from_hours(1),
        );
        let r = ProviderResolution::with_lease(SecretString::new("vault-secret"), lease);
        assert_eq!(r.ttl, Some(Duration::from_hours(1)));
        assert!(r.lease.is_some());
        assert_eq!(r.lease.as_ref().expect("lease set").provider, "vault");
    }

    #[test]
    fn empty_has_no_secret_or_lease_or_ttl() {
        let r = ProviderResolution::empty();
        assert!(r.secret.expose_secret().is_empty());
        assert!(r.lease.is_none());
        assert!(r.ttl.is_none());
    }
}
