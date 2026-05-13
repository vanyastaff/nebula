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
//! Sibling to the existing [`CacheLayer`](super::CacheLayer) /
//! [`EncryptionLayer`](super::EncryptionLayer) /
//! [`AuditLayer`](super::AuditLayer) / [`ScopeLayer`](super::ScopeLayer),
//! but wraps the [`ExternalProvider`] trait rather than
//! [`CredentialStore`](nebula_credential::CredentialStore) — hence the
//! disambiguating `Provider` prefix in the type name.
//!
//! See `docs/adr/0051-external-provider-redesign.md` for the design that
//! motivated this layer.

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
    ExternalProvider, ExternalReference, ProviderError, ProviderFuture, ProviderKind,
    ProviderResolution,
};

/// Cache key derived from [`ExternalReference`].
///
/// Owned so it can live inside the cache (moka requires `K: 'static`). All
/// four reference fields participate in equality: providers are free to
/// interpret `version` / `field` differently (Vault treats a missing
/// `version` as "latest", distinct from any pinned version), so they cannot
/// collapse safely.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
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
            .or((!self.default_ttl.is_zero()).then_some(self.default_ttl))
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
}

/// Caching layer wrapping an `Arc<dyn ExternalProvider>`.
///
/// # Examples
///
/// ```rust,ignore
/// use std::{sync::Arc, time::Duration};
/// use nebula_credential::provider::ExternalProvider;
/// use nebula_storage::credential::{ProviderCacheLayer, ProviderCacheConfig};
///
/// let inner: Arc<dyn ExternalProvider> = Arc::new(my_vault_provider);
/// let cached = ProviderCacheLayer::new(
///     inner,
///     ProviderCacheConfig {
///         max_entries: 1_000,
///         default_ttl: Duration::from_mins(1),
///     },
/// );
/// ```
pub struct ProviderCacheLayer {
    inner: Arc<dyn ExternalProvider>,
    cache: Cache<CacheKey, Arc<ProviderResolution>>,
    hits: AtomicU64,
    misses: AtomicU64,
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
        Self {
            inner,
            cache,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Cache hit / miss statistics.
    #[must_use]
    pub fn stats(&self) -> ProviderCacheStats {
        ProviderCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    /// Drop a specific cached entry, if present.
    pub async fn invalidate(&self, reference: &ExternalReference) {
        self.cache
            .invalidate(&CacheKey::from_reference(reference))
            .await;
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
                    path = %reference.path,
                    "cache hit"
                );
                return Ok((*arc).clone());
            }

            self.misses.fetch_add(1, Ordering::Relaxed);

            // `try_get_with` deduplicates concurrent waiters (single-flight)
            // and skips insertion on `Err`, so failures are never cached.
            // The init future must be `'static`: clone the inner Arc and
            // own a copy of the reference so the closure borrows nothing.
            let inner = Arc::clone(&self.inner);
            let reference_owned = reference.clone();
            let key_for_init = key.clone();
            let provider_label = inner.provider_name().to_owned();
            tracing::debug!(
                target: "nebula_storage::provider_cache",
                provider = %provider_label,
                path = %reference.path,
                "cache miss; calling inner"
            );

            let result = self
                .cache
                .try_get_with(key_for_init, async move {
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

    fn provider_name(&self) -> &'static str {
        "cache"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nebula_credential::{
        SecretString,
        provider::{LeaseHandle, ProviderError, ProviderKind, ProviderResolution},
    };

    use super::*;

    // ────────────────────────────────────────────────────────────────────
    // Test scaffolding
    // ────────────────────────────────────────────────────────────────────

    /// One step of mock behaviour.
    enum Step {
        Ok {
            secret: String,
            ttl: Option<Duration>,
            lease: Option<LeaseHandle>,
        },
        Err(ProviderError),
    }

    /// Mock provider with a deterministic outcome script.
    ///
    /// Each call pops the head of `script`; if the script is exhausted the
    /// last step is replayed. The optional `delay` lets us widen the
    /// concurrency window for single-flight tests.
    struct MockProvider {
        name: &'static str,
        calls: AtomicU64,
        script: Mutex<Vec<Step>>,
        delay: Duration,
    }

    impl fmt::Debug for MockProvider {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MockProvider")
                .field("name", &self.name)
                .field("calls", &self.calls.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl MockProvider {
        fn new(name: &'static str, script: Vec<Step>) -> Arc<Self> {
            Arc::new(Self {
                name,
                calls: AtomicU64::new(0),
                script: Mutex::new(script),
                delay: Duration::ZERO,
            })
        }

        fn with_delay(name: &'static str, delay: Duration, script: Vec<Step>) -> Arc<Self> {
            Arc::new(Self {
                name,
                calls: AtomicU64::new(0),
                script: Mutex::new(script),
                delay,
            })
        }

        fn call_count(&self) -> u64 {
            self.calls.load(Ordering::Relaxed)
        }

        fn pop_step(&self) -> Step {
            let mut script = self.script.lock().expect("script lock poisoned");
            if script.len() > 1 {
                script.remove(0)
            } else {
                // Replay the last step so callers can rely on a "steady
                // state" once the script is drained.
                let last = script.last().expect("script must have at least one step");
                match last {
                    Step::Ok { secret, ttl, lease } => Step::Ok {
                        secret: secret.clone(),
                        ttl: *ttl,
                        lease: lease.clone(),
                    },
                    Step::Err(e) => Step::Err(clone_provider_error(e)),
                }
            }
        }
    }

    impl ExternalProvider for MockProvider {
        fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let step = self.pop_step();
            let delay = self.delay;
            ProviderFuture::new(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                match step {
                    Step::Ok { secret, ttl, lease } => {
                        let mut r = ProviderResolution::from_secret(SecretString::new(secret));
                        r.ttl = ttl;
                        r.lease = lease;
                        Ok(r)
                    },
                    Step::Err(e) => Err(e),
                }
            })
        }

        fn provider_name(&self) -> &str {
            self.name
        }
    }

    fn refer(path: &str) -> ExternalReference {
        ExternalReference {
            provider: ProviderKind::Custom("test".to_owned()),
            path: path.to_owned(),
            version: None,
            field: None,
        }
    }

    fn ok(secret: &str, ttl: Option<Duration>) -> Step {
        Step::Ok {
            secret: secret.to_owned(),
            ttl,
            lease: None,
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Tests
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn second_resolve_is_a_cache_hit() {
        let inner = MockProvider::new("inner", vec![ok("v1", Some(Duration::from_mins(1)))]);
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        let r = refer("secret/foo");

        let first = layer.resolve(&r).await.expect("first resolve");
        let second = layer.resolve(&r).await.expect("second resolve");

        assert_eq!(first.secret.expose_secret(), "v1");
        assert_eq!(second.secret.expose_secret(), "v1");
        assert_eq!(
            inner.call_count(),
            1,
            "inner called once across two resolves"
        );
        assert!(layer.stats().hits >= 1, "hits should be recorded");
    }

    #[tokio::test]
    async fn different_keys_miss_independently() {
        let inner = MockProvider::new(
            "inner",
            vec![
                ok("first", Some(Duration::from_mins(1))),
                ok("second", Some(Duration::from_mins(1))),
            ],
        );
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );

        let a = layer.resolve(&refer("a")).await.expect("a");
        let b = layer.resolve(&refer("b")).await.expect("b");

        assert_eq!(a.secret.expose_secret(), "first");
        assert_eq!(b.secret.expose_secret(), "second");
        assert_eq!(inner.call_count(), 2);
    }

    #[tokio::test]
    async fn expired_entry_triggers_fresh_resolve() {
        let inner = MockProvider::new(
            "inner",
            vec![
                ok("fresh", Some(Duration::from_millis(40))),
                ok("after-expiry", Some(Duration::from_millis(40))),
            ],
        );
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        let r = refer("secret");

        let first = layer.resolve(&r).await.expect("first");
        assert_eq!(first.secret.expose_secret(), "fresh");

        // Wait past the per-entry TTL; moka evicts lazily on next access.
        tokio::time::sleep(Duration::from_millis(120)).await;

        let second = layer.resolve(&r).await.expect("second");
        assert_eq!(second.secret.expose_secret(), "after-expiry");
        assert_eq!(inner.call_count(), 2);
    }

    #[tokio::test]
    async fn concurrent_resolves_single_flight() {
        // Slow the inner so all spawned tasks queue while the first is in
        // flight. moka's `try_get_with` should dedup them.
        let inner = MockProvider::with_delay(
            "inner",
            Duration::from_millis(80),
            vec![ok("shared", Some(Duration::from_mins(1)))],
        );
        let layer = Arc::new(ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        ));

        let mut handles = Vec::new();
        for _ in 0..16 {
            let layer = Arc::clone(&layer);
            handles.push(tokio::spawn(async move {
                layer.resolve(&refer("secret")).await
            }));
        }
        for h in handles {
            let r = h.await.expect("task join").expect("resolve");
            assert_eq!(r.secret.expose_secret(), "shared");
        }
        assert_eq!(
            inner.call_count(),
            1,
            "single-flight: 16 concurrent resolves → 1 inner call"
        );
    }

    #[tokio::test]
    async fn no_ttl_with_zero_default_does_not_cache() {
        // `ttl: None` + `default_ttl: ZERO` ⇒ effective TTL is ZERO ⇒
        // entry expires immediately, second resolve calls inner again.
        let inner = MockProvider::new("inner", vec![ok("once", None), ok("twice", None)]);
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig {
                max_entries: 10,
                default_ttl: Duration::ZERO,
            },
        );
        let r = refer("secret");

        let first = layer.resolve(&r).await.expect("first");
        let second = layer.resolve(&r).await.expect("second");

        assert_eq!(first.secret.expose_secret(), "once");
        assert_eq!(second.secret.expose_secret(), "twice");
        assert_eq!(inner.call_count(), 2, "bypass: each resolve hits inner");
    }

    #[tokio::test]
    async fn default_ttl_applies_when_resolution_has_none() {
        // `ttl: None` + `default_ttl > 0` ⇒ cache for default_ttl.
        let inner = MockProvider::new("inner", vec![ok("cached", None), ok("never", None)]);
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig {
                max_entries: 10,
                default_ttl: Duration::from_mins(1),
            },
        );
        let r = refer("secret");

        let first = layer.resolve(&r).await.expect("first");
        let second = layer.resolve(&r).await.expect("second");

        assert_eq!(first.secret.expose_secret(), "cached");
        assert_eq!(second.secret.expose_secret(), "cached");
        assert_eq!(
            inner.call_count(),
            1,
            "default TTL caches even with ttl=None"
        );
    }

    #[tokio::test]
    async fn error_is_not_cached() {
        let inner = MockProvider::new(
            "inner",
            vec![
                Step::Err(ProviderError::Unavailable {
                    reason: "network down".to_owned(),
                }),
                ok("recovered", Some(Duration::from_mins(1))),
            ],
        );
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        let r = refer("secret");

        let first = layer.resolve(&r).await.expect_err("first should fail");
        assert!(matches!(first, ProviderError::Unavailable { .. }));

        let second = layer.resolve(&r).await.expect("second should succeed");
        assert_eq!(second.secret.expose_secret(), "recovered");
        assert_eq!(
            inner.call_count(),
            2,
            "error path re-attempts on next resolve"
        );
    }

    #[tokio::test]
    async fn race_on_expired_entry_resolves_freshly() {
        // Regression guard for the TOCTOU window: an entry expires between
        // a concurrent batch checking the cache and actually awaiting the
        // resolution. moka's lazy eviction must produce a single fresh
        // resolve for the post-expiry wave.
        let inner = MockProvider::with_delay(
            "inner",
            Duration::from_millis(40),
            vec![
                ok("v1", Some(Duration::from_millis(30))),
                ok("v2", Some(Duration::from_millis(30))),
            ],
        );
        let layer = Arc::new(ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        ));

        let r = refer("secret");
        let first = layer.resolve(&r).await.expect("first");
        assert_eq!(first.secret.expose_secret(), "v1");
        assert_eq!(inner.call_count(), 1);

        // Wait for the entry to expire.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Fire a concurrent batch; all should observe the post-expiry
        // resolution and dedup to a single inner call.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let layer = Arc::clone(&layer);
            handles.push(tokio::spawn(async move {
                layer.resolve(&refer("secret")).await
            }));
        }
        for h in handles {
            let v = h.await.expect("join").expect("resolve");
            assert_eq!(v.secret.expose_secret(), "v2");
        }
        assert_eq!(
            inner.call_count(),
            2,
            "post-expiry batch should dedup to one inner call"
        );
    }

    #[tokio::test]
    async fn health_check_delegates_to_inner() {
        let inner = MockProvider::new("vault-stub", vec![ok("unused", None)]);
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        // Default `health_check` returns the no-secret success resolution.
        let _ = layer.health_check().await.expect("health ok");
    }

    #[tokio::test]
    async fn provider_name_is_cache() {
        let inner = MockProvider::new("inner", vec![ok("v", None)]);
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        assert_eq!(layer.provider_name(), "cache");
    }

    #[tokio::test]
    async fn invalidate_drops_specific_entry() {
        let inner = MockProvider::new(
            "inner",
            vec![
                ok("first", Some(Duration::from_mins(1))),
                ok("second", Some(Duration::from_mins(1))),
            ],
        );
        let layer = ProviderCacheLayer::new(
            Arc::clone(&inner) as Arc<dyn ExternalProvider>,
            ProviderCacheConfig::default(),
        );
        let r = refer("secret");

        let _ = layer.resolve(&r).await.expect("first");
        layer.invalidate(&r).await;
        let second = layer.resolve(&r).await.expect("second");

        assert_eq!(second.secret.expose_secret(), "second");
        assert_eq!(inner.call_count(), 2);
    }

    #[test]
    fn stats_hit_rate_handles_empty() {
        let stats = ProviderCacheStats::default();
        assert!((stats.hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn stats_hit_rate_is_correct_fraction() {
        let stats = ProviderCacheStats { hits: 3, misses: 1 };
        assert!((stats.hit_rate() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn expiry_picks_value_ttl_over_default() {
        let policy = ProviderExpiry {
            default_ttl: Duration::from_secs(10),
        };
        assert_eq!(
            policy.effective_ttl(Some(Duration::from_secs(3))),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn expiry_falls_back_to_default_when_value_has_none() {
        let policy = ProviderExpiry {
            default_ttl: Duration::from_secs(10),
        };
        assert_eq!(policy.effective_ttl(None), Duration::from_secs(10));
    }

    #[test]
    fn expiry_is_zero_when_both_unset() {
        let policy = ProviderExpiry {
            default_ttl: Duration::ZERO,
        };
        assert_eq!(policy.effective_ttl(None), Duration::ZERO);
    }

    #[test]
    fn expiry_treats_explicit_zero_value_ttl_as_bypass() {
        let policy = ProviderExpiry {
            default_ttl: Duration::from_secs(5),
        };
        // `value.ttl == Some(ZERO)` matches the plan's formula
        // `value.ttl.or(default_ttl).filter(|d| d > ZERO)` — `.or()` short-
        // circuits on `Some(_)` so the default never participates, and the
        // filter then drops the zero. Result: do not cache.
        assert_eq!(policy.effective_ttl(Some(Duration::ZERO)), Duration::ZERO);
    }
}
