//! Caching layer for [`ExternalProvider`] resolutions.
//!
//! Wraps an `Arc<dyn ExternalProvider>` with a moka cache that:
//!
//! - **Single-flights** concurrent resolves of the same key (one inner call,
//!   N waiters share the result) via [`moka::future::Cache::try_get_with`].
//! - Honours per-entry TTL from
//!   [`ProviderResolution::ttl`](nebula_credential::provider::ProviderResolution::ttl),
//!   falling back to [`ProviderCacheConfig::default_ttl`] when the inner
//!   provider does not advertise a TTL.
//! - Treats an effective TTL of `Duration::ZERO` (i.e. `ttl == None` and
//!   `default_ttl == ZERO`) as **bypass** — the entry is briefly created
//!   under the single-flight guarantee and then evicted by the
//!   [`moka::Expiry`] policy, so the next resolve hits the inner provider
//!   again. This is compliance-critical for providers that explicitly opt
//!   out of caching (env vars, in-memory stubs).
//! - Does **not** cache failures — every concurrent waiter receives a clone
//!   of the inner error, but the cache slot stays empty so the next call
//!   re-attempts the resolve.
//!
//! Sibling to the existing `CacheLayer` / `EncryptionLayer` /
//! `AuditLayer` credential-store wrappers, but wraps the
//! [`ExternalProvider`] trait rather than
//! [`CredentialPersistence`](nebula_storage_port::CredentialPersistence) — hence the
//! disambiguating `Provider` prefix in the type name.
//!
//! See ADR-0081 (ADR-0051, consolidated)
//! for the design that motivated this layer.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use moka::{Expiry, future::Cache};
use nebula_credential::provider::{
    ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider, ProviderError,
    ProviderFuture, ProviderKind, ProviderResolution,
};

/// Cache key derived from [`ExternalReference`].
///
/// Owned so it can live inside the cache (moka requires `K: 'static`). All
/// four reference fields participate in equality: providers are free to
/// interpret `version` / `field` differently (Vault treats a missing
/// `version` as "latest", distinct from any pinned version), so they cannot
/// collapse safely.
#[derive(Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    provider: ProviderKind,
    path: String,
    version: Option<String>,
    field: Option<String>,
}

impl CacheKey {
    fn from_reference(reference: &ExternalReference) -> Self {
        Self {
            provider: reference.provider.clone(),
            path: reference.path.clone(),
            version: reference.version.clone(),
            field: reference.field.clone(),
        }
    }
}

/// Configuration for [`ProviderCacheLayer`].
#[derive(Debug, Clone)]
pub struct ProviderCacheConfig {
    /// Maximum number of cached resolutions. Default: 10,000.
    pub max_entries: u64,
    /// Fallback TTL used when [`ProviderResolution::ttl`] is `None`.
    ///
    /// The effective TTL is `value.ttl.or(default_ttl)`, treated as bypass
    /// when zero. Default is [`Duration::ZERO`] — i.e. the cache stores
    /// **only** resolutions that carry an explicit TTL (typical for leased
    /// or time-bounded secrets) and passes everything else straight through.
    pub default_ttl: Duration,
}

impl Default for ProviderCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            default_ttl: Duration::ZERO,
        }
    }
}

/// Cache hit / miss counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderCacheStats {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
}

impl ProviderCacheStats {
    /// Hit rate as a fraction in `[0.0, 1.0]`. Returns `0.0` with no requests.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Per-entry TTL policy.
///
/// Reads [`ProviderResolution::ttl`] first, falling back to the configured
/// default. A zero / missing effective TTL is reported as
/// [`Duration::ZERO`], which moka interprets as "expired on insert" — the
/// entry is briefly visible to concurrent single-flight waiters, then
/// evicted on the next access.
#[derive(Debug)]
struct ProviderExpiry {
    default_ttl: Duration,
}

impl ProviderExpiry {
    fn effective_ttl(&self, value_ttl: Option<Duration>) -> Duration {
        value_ttl
            .or_else(|| (!self.default_ttl.is_zero()).then_some(self.default_ttl))
            .filter(|d| !d.is_zero())
            .unwrap_or(Duration::ZERO)
    }
}

impl Expiry<CacheKey, Arc<ProviderResolution>> for ProviderExpiry {
    fn expire_after_create(
        &self,
        _key: &CacheKey,
        value: &Arc<ProviderResolution>,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(self.effective_ttl(value.ttl))
    }

    /// Mirror `expire_after_create` for the update path.
    ///
    /// `try_get_with` is the only insertion path today (one-shot init, so
    /// `expire_after_update` is unreachable through the public API), but a
    /// future proactive-refresh hook would otherwise inherit moka's default
    /// "keep current expiration" behaviour, silently breaking per-entry TTL
    /// semantics for the refreshed value. Explicitly delegating future-
    /// proofs the policy against that contributor accident.
    fn expire_after_update(
        &self,
        _key: &CacheKey,
        value: &Arc<ProviderResolution>,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        Some(self.effective_ttl(value.ttl))
    }
}

/// Caching layer wrapping an `Arc<dyn ExternalProvider>`.
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_credential::provider::{
///     ExternalProvider, ExternalReference, ProviderFuture, ProviderResolution,
/// };
/// use nebula_storage::credential::{ProviderCacheConfig, ProviderCacheLayer};
///
/// // Stand-in for a real backend (Vault, AWS SM, …). A production provider
/// // resolves the reference from its remote system.
/// #[derive(Debug)]
/// struct MyVaultProvider;
/// impl ExternalProvider for MyVaultProvider {
///     fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
///         ProviderFuture::ready(Ok(ProviderResolution::empty()))
///     }
///     fn provider_name(&self) -> &str {
///         "my-vault"
///     }
/// }
///
/// let inner: Arc<dyn ExternalProvider> = Arc::new(MyVaultProvider);
/// let cached = ProviderCacheLayer::new(
///     inner,
///     ProviderCacheConfig {
///         max_entries: 1_000,
///         default_ttl: Duration::from_secs(60),
///     },
/// );
///
/// // The cache layer composes its name over the wrapped provider for telemetry.
/// assert_eq!(cached.provider_name(), "cache(my-vault)");
/// assert_eq!(cached.stats().hits, 0);
/// ```
pub struct ProviderCacheLayer {
    inner: Arc<dyn ExternalProvider>,
    cache: Cache<CacheKey, Arc<ProviderResolution>>,
    /// Pre-formatted provider name (`"cache(<inner>)"`) returned by the
    /// trait impl so telemetry can dimension on the wrapped backend.
    /// `Box<str>` — heap-allocated once at construction, then handed out
    /// as borrowed `&str` slices for the (`'_`-lifetime) trait method.
    name: Box<str>,
    /// Cache hits — entries returned from the fast path before single-flight.
    hits: AtomicU64,
    /// Resolved inner calls — incremented from inside the single-flight init
    /// closure, so a fan-in of N concurrent waiters records exactly one miss
    /// per surviving inner call, matching `ExternalProvider::resolve` load.
    inner_calls: AtomicU64,
}

impl fmt::Debug for ProviderCacheLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderCacheLayer")
            .field("inner", &self.inner.provider_name())
            .field("entry_count", &self.cache.entry_count())
            .finish_non_exhaustive()
    }
}

impl ProviderCacheLayer {
    /// Create a new cache layer wrapping `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn ExternalProvider>, config: ProviderCacheConfig) -> Self {
        let expiry = ProviderExpiry {
            default_ttl: config.default_ttl,
        };
        let cache = Cache::builder()
            .max_capacity(config.max_entries)
            .expire_after(expiry)
            .build();
        let name = format!("cache({})", inner.provider_name()).into_boxed_str();
        Self {
            inner,
            cache,
            name,
            hits: AtomicU64::new(0),
            inner_calls: AtomicU64::new(0),
        }
    }

    /// Cache hit / miss statistics.
    ///
    /// `hits` counts lookups served from cache before single-flight engages;
    /// `misses` counts **inner provider calls** (single-flight survivors), so
    /// `hits + misses` is **not** the total lookup count under contention —
    /// (N − 1) concurrent waiters that subscribed to an in-flight resolve are
    /// invisible to both counters. This matches how operators read the
    /// `hit_rate`: as "fraction of lookups that avoided a backend round-trip".
    #[must_use]
    pub fn stats(&self) -> ProviderCacheStats {
        ProviderCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.inner_calls.load(Ordering::Relaxed),
        }
    }

    /// Drop a specific cached entry, if present.
    pub async fn invalidate(&self, reference: &ExternalReference) {
        // Bind the key to a local so the borrow is unambiguous across the
        // await (a temporary would live to end-of-statement, which is
        // sufficient — but the explicit binding is clearer and matches the
        // existing layer/cache.rs pattern).
        let key = CacheKey::from_reference(reference);
        self.cache.invalidate(&key).await;
    }

    /// Drop every cached entry.
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }
}

/// Rebuild a fresh `ProviderError` from a shared reference.
///
/// `moka::Cache::try_get_with` returns `Arc<E>` for the failure path so all
/// concurrent waiters share the error. The trait surface expects an owned
/// `ProviderError`, so we clone the variant fields. The `Backend` payload is
/// a `Box<dyn Error>` (not `Clone`), so we collapse it into its display
/// string — losing the source chain but preserving the message. The enum is
/// `#[non_exhaustive]`, so a `_` arm guards against future variants.
fn clone_provider_error(err: &ProviderError) -> ProviderError {
    match err {
        ProviderError::NotFound { path } => ProviderError::NotFound { path: path.clone() },
        ProviderError::Unavailable { reason } => ProviderError::Unavailable {
            reason: reason.clone(),
        },
        ProviderError::AccessDenied { reason } => ProviderError::AccessDenied {
            reason: reason.clone(),
        },
        ProviderError::Backend(inner) => ProviderError::Backend(format!("{inner}").into()),
        other => ProviderError::Unavailable {
            reason: format!("{other}"),
        },
    }
}

impl ExternalProvider for ProviderCacheLayer {
    fn resolve<'a>(&'a self, reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            let key = CacheKey::from_reference(reference);

            // Fast path: a fresh, unexpired entry is a hit. moka enforces
            // expiration lazily on access, so `get` returning `Some` already
            // implies "still alive".
            if let Some(arc) = self.cache.get(&key).await {
                self.hits.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    target: "nebula_storage::provider_cache",
                    provider = %self.inner.provider_name(),
                    cache_outcome = "hit",
                    "cache hit"
                );
                return Ok((*arc).clone());
            }

            // `try_get_with` deduplicates concurrent waiters (single-flight)
            // and skips insertion on `Err`, so failures are never cached.
            // The init future must be `'static`: clone the inner Arc and
            // own a copy of the reference so the closure borrows nothing.
            //
            // The `inner_calls` counter increments **inside** the init
            // closure rather than on the way in, so under fan-in N waiters
            // that subscribe to one in-flight resolve record exactly one
            // miss — matching real backend load (`inner.resolve` call
            // count) rather than over-counting lookup misses.
            let inner = Arc::clone(&self.inner);
            let reference_owned = reference.clone();
            let key_for_init = key.clone();
            let provider_label = inner.provider_name().to_owned();
            tracing::debug!(
                target: "nebula_storage::provider_cache",
                provider = %provider_label,
                cache_outcome = "miss",
                "cache miss; calling inner"
            );
            let inner_calls = &self.inner_calls;

            let result = self
                .cache
                .try_get_with(key_for_init, async move {
                    inner_calls.fetch_add(1, Ordering::Relaxed);
                    inner.resolve(&reference_owned).await.map(Arc::new)
                })
                .await;

            match result {
                Ok(arc) => Ok((*arc).clone()),
                Err(arc_err) => Err(clone_provider_error(&arc_err)),
            }
        })
    }

    fn health_check(&self) -> ProviderFuture<'_> {
        self.inner.health_check()
    }

    fn provider_name(&self) -> &str {
        &self.name
    }

    /// Surface the cache layer itself as the lease dispatcher when the
    /// wrapped provider advertises lease capability.
    ///
    /// Returning the inner's view directly (a tempting shortcut) is
    /// **wrong**: it lets callers issue `renew` / `revoke` straight to
    /// the backing provider, bypassing this cache layer entirely. A
    /// revoked lease would then remain visible from the cache until its
    /// TTL expired, serving a now-invalid secret to subsequent resolves.
    /// Routing through `self` lets [`LeasedProvider::revoke`] /
    /// [`LeasedProvider::renew`] invalidate cached entries before
    /// forwarding to the inner.
    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        self.inner
            .lease_renewal()
            .is_some()
            .then_some(self as &dyn LeasedProvider)
    }
}

impl ProviderCacheLayer {
    /// Collect cache keys whose stored resolution carries `lease_id`.
    ///
    /// Iteration is moka's lazy snapshot — entries inserted concurrently
    /// with the walk may not be observed. That is acceptable here:
    /// revoke / renew run *before* the next resolve repopulates the
    /// cache, so a concurrently-inserted entry will reach the cache after
    /// invalidation and naturally see the post-revoke state on its own
    /// next read. The contract guaranteed to the caller is "no stale
    /// entry persists past this call", which the post-await invalidation
    /// upholds for everything visible at iteration time.
    fn cache_keys_for_lease(&self, lease_id: &str) -> Vec<CacheKey> {
        self.cache
            .iter()
            .filter_map(|(key_arc, value_arc)| {
                value_arc
                    .lease
                    .as_ref()
                    .filter(|l| l.lease_id == lease_id)
                    .map(|_| (*key_arc).clone())
            })
            .collect()
    }

    async fn invalidate_lease_entries(&self, lease_id: &str) -> usize {
        let cache_keys = self.cache_keys_for_lease(lease_id);
        let invalidated_entries = cache_keys.len();
        for key in cache_keys {
            self.cache.invalidate(&key).await;
        }
        invalidated_entries
    }
}

impl LeasedProvider for ProviderCacheLayer {
    /// Delegate to the wrapped provider's leased view — the cache layer
    /// itself never issues leases, so attribution must come from inner.
    fn handles_lease(&self, lease: &LeaseHandle) -> bool {
        self.inner
            .lease_renewal()
            .is_some_and(|leased| leased.handles_lease(lease))
    }

    /// Renew via the wrapped provider, then invalidate any cached entry
    /// holding the renewed lease so the next resolve picks up the refreshed
    /// lease/TTL.
    ///
    /// We invalidate **after** the inner renew succeeds: a failed renew
    /// leaves the lease still valid (the caller will retry), so dropping
    /// the cached entry pre-emptively would force an unnecessary backend
    /// round-trip on the next resolve.
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            let Some(inner_leased) = self.inner.lease_renewal() else {
                return Err(ProviderError::Backend(
                    "ProviderCacheLayer::renew: wrapped provider is not leased"
                        .to_owned()
                        .into(),
                ));
            };
            let renewed = inner_leased.renew(lease).await?;
            let invalidated_entries = self.invalidate_lease_entries(&lease.lease_id).await;
            tracing::debug!(
                target: "nebula_storage::provider_cache",
                provider = %self.inner.provider_name(),
                lease_operation = "renew",
                invalidated_entries,
                "lease renewed; cache invalidated for matching entries"
            );
            Ok(renewed)
        })
    }

    /// Revoke via the wrapped provider, invalidating cached entries up-front
    /// so concurrent resolves cannot serve the revoked secret.
    ///
    /// Invalidation runs **before** the inner revoke so a slow revoke
    /// cannot keep stale entries reachable; if revoke itself errors, the
    /// next resolve will repopulate from inner (which now reflects the
    /// half-finished revoke state) — correct behaviour for a recoverable
    /// failure.
    fn revoke<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            let invalidated_entries = self.invalidate_lease_entries(&lease.lease_id).await;
            let Some(inner_leased) = self.inner.lease_renewal() else {
                return Err(ProviderError::Backend(
                    "ProviderCacheLayer::revoke: wrapped provider is not leased"
                        .to_owned()
                        .into(),
                ));
            };
            tracing::debug!(
                target: "nebula_storage::provider_cache",
                provider = %self.inner.provider_name(),
                lease_operation = "revoke",
                invalidated_entries,
                "cache invalidated for matching entries; forwarding revoke to inner"
            );
            inner_leased.revoke(lease).await
        })
    }
}

#[cfg(test)]
#[path = "provider_cache_tests.rs"]
mod tests;
