//! Resolution envelope returned by [`ExternalProvider`](super::ExternalProvider).
//!
//! Mirrors the AWS `Identity` envelope (`aws-smithy-runtime-api`
//! `client::identity::Identity`) at a much smaller scope: a secret plus the
//! operational metadata a cache layer needs to do its job without further trait
//! changes. Currently carries the secret, an optional lease handle (HashiCorp
//! Vault style), and an optional TTL hint. `#[non_exhaustive]` lets us grow
//! additional fields (e.g. typed `properties` sidecar) without breaking
//! implementors.

use std::time::Duration;

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
/// - [`ttl`](Self::ttl) — caching hint. `None` means "do not cache" (treat every
///   resolve as authoritative); a cache layer MUST honour `Some(d)` and refresh
///   no later than `d` after resolution.
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
    /// Suggested time-to-live for the resolved secret. Consumed by a
    /// downstream cache layer (`ProviderCacheLayer` in `nebula-storage`,
    /// future work per ADR-0051). `None` ⇒ do not cache.
    pub ttl: Option<Duration>,
}

impl ProviderResolution {
    /// Convenience constructor for static providers without lease / TTL
    /// metadata. The resulting resolution will not be cached by a
    /// `ProviderCacheLayer` (TTL is `None`).
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

    /// Marker used by the default `health_check` implementation to return
    /// a no-secret success without allocating a real `SecretString`. The
    /// `secret` field is set to an empty string; callers MUST NOT consume
    /// this resolution as a real secret.
    pub(crate) fn health_ok() -> Self {
        Self {
            secret: SecretString::new(String::new()),
            lease: None,
            ttl: None,
        }
    }
}

/// Handle to a lease issued by a [`ExternalProvider`](super::ExternalProvider).
///
/// Carried in [`ProviderResolution::lease`]; opaque to the caller except for
/// the `lease_id` (used for diagnostics and as the key passed to a future
/// `LeasedProvider::renew` / `LeasedProvider::revoke` API).
#[derive(Debug, Clone)]
pub struct LeaseHandle {
    /// Provider-specific lease identifier (e.g. Vault lease id).
    pub lease_id: String,
    /// When the provider issued this lease.
    pub issued_at: chrono::DateTime<chrono::Utc>,
    /// Time-to-live communicated by the provider at issue time.
    pub ttl: Duration,
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
        let lease = LeaseHandle {
            lease_id: "vault-lease-abc".to_owned(),
            issued_at: chrono::Utc::now(),
            ttl: Duration::from_hours(1),
        };
        let r = ProviderResolution::with_lease(SecretString::new("vault-secret"), lease);
        assert_eq!(r.ttl, Some(Duration::from_hours(1)));
        assert!(r.lease.is_some());
    }
}
