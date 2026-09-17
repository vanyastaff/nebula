use std::sync::Mutex;

use nebula_credential::{
    SecretString,
    provider::{LeaseHandle, ProviderError, ProviderKind, ProviderResolution},
};
use tracing::instrument::WithSubscriber as _;

use super::*;

// ────────────────────────────────────────────────────────────────────
// Test scaffolding
// ────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<String>>>);

impl EventCapture {
    fn captured(&self) -> String {
        self.0
            .lock()
            .expect("event capture lock poisoned")
            .join("\n")
    }
}

impl tracing::Subscriber for EventCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);

        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
                use fmt::Write as _;

                write!(&mut self.0, "{}={value:?};", field.name())
                    .expect("writing to a String cannot fail");
            }
        }

        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0
            .lock()
            .expect("event capture lock poisoned")
            .push(fields.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

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
    // moka uses `std::time::Instant` (or quanta) for expiration, not
    // `tokio::time::Instant`, so `tokio::time::pause` would not fast-
    // forward expiry here — we have to use the real wall clock. TTL
    // and sleep are sized so the margin (sleep − TTL = 200 ms) far
    // exceeds typical CI scheduler jitter; `inner.call_count` is
    // checked with `>=` rather than strict `==` so a still-cached
    // entry (slow CI hop) doesn't false-negative.
    let inner = MockProvider::new(
        "inner",
        vec![
            ok("fresh", Some(Duration::from_millis(100))),
            ok("after-expiry", Some(Duration::from_millis(100))),
        ],
    );
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    let r = refer("secret");

    let first = layer.resolve(&r).await.expect("first");
    assert_eq!(first.secret.expose_secret(), "fresh");

    // Wait well past the per-entry TTL; moka evicts lazily on next
    // access, so the next resolve sees a miss and re-runs init.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second = layer.resolve(&r).await.expect("second");
    assert_eq!(second.secret.expose_secret(), "after-expiry");
    assert!(
        inner.call_count() >= 2,
        "expected at least one fresh resolve after TTL expiry, got {}",
        inner.call_count()
    );
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
    //
    // Timings are sized for CI hostility: TTL=100 ms, inner delay=60 ms
    // (< TTL, so the first entry actually gets cached), and sleep=300 ms
    // (3× TTL margin). The post-expiry inner-call count is asserted
    // with `<=` and `>=` bounds rather than strict equality — single-
    // flight should dedupe the batch to one extra call, but a stalled
    // scheduler that splits the batch across the second TTL boundary
    // would legitimately drive it higher without invalidating the
    // single-flight contract.
    let inner = MockProvider::with_delay(
        "inner",
        Duration::from_millis(60),
        vec![
            ok("v1", Some(Duration::from_millis(100))),
            ok("v2", Some(Duration::from_millis(100))),
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

    // Wait well past the first TTL.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Fire a concurrent batch; all should observe the post-expiry
    // resolution and dedup to a small number of inner calls.
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
    let total = inner.call_count();
    assert!(
        (2..=3).contains(&total),
        "post-expiry batch should dedup the 8 waiters to ~1 extra inner call (total 2–3), got {total}"
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
async fn provider_name_composes_inner() {
    // `provider_name` preserves the wrapped provider for telemetry so
    // operators dimensioning on it can tell Vault, AWS SM, env-var,
    // etc. apart through the cache.
    let inner = MockProvider::new("vault-stub", vec![ok("v", None)]);
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    assert_eq!(layer.provider_name(), "cache(vault-stub)");
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

// ────────────────────────────────────────────────────────────────────
// A7/B5 cross-phase fold — lease capability propagation + invalidation.
// ────────────────────────────────────────────────────────────────────

/// Mock leased provider. `resolve` returns a resolution whose lease
/// uses the configured `lease_id` (so multiple instances can be
/// distinguished), and the renew / revoke paths track invocation
/// counts so tests can assert the cache layer actually delegated.
struct LeasedMock {
    name: &'static str,
    lease_id: String,
    lease_ttl: Duration,
    resolve_calls: AtomicU64,
    renew_calls: AtomicU64,
    revoke_calls: AtomicU64,
}

impl fmt::Debug for LeasedMock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedMock")
            .field("name", &self.name)
            .field("lease_id", &"[REDACTED]")
            .field("lease_ttl", &self.lease_ttl)
            .field("resolve_calls", &self.resolve_calls.load(Ordering::Relaxed))
            .field("renew_calls", &self.renew_calls.load(Ordering::Relaxed))
            .field("revoke_calls", &self.revoke_calls.load(Ordering::Relaxed))
            .finish()
    }
}

impl LeasedMock {
    fn new(name: &'static str, lease_id: &str, lease_ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            name,
            lease_id: lease_id.to_owned(),
            lease_ttl,
            resolve_calls: AtomicU64::new(0),
            renew_calls: AtomicU64::new(0),
            revoke_calls: AtomicU64::new(0),
        })
    }

    fn renew_count(&self) -> u64 {
        self.renew_calls.load(Ordering::Relaxed)
    }

    fn revoke_count(&self) -> u64 {
        self.revoke_calls.load(Ordering::Relaxed)
    }

    fn resolve_count(&self) -> u64 {
        self.resolve_calls.load(Ordering::Relaxed)
    }

    fn issued_lease(&self) -> LeaseHandle {
        LeaseHandle::new(
            self.name,
            self.lease_id.clone(),
            chrono::Utc::now(),
            self.lease_ttl,
        )
    }
}

impl ExternalProvider for LeasedMock {
    fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
        self.resolve_calls.fetch_add(1, Ordering::Relaxed);
        let lease = self.issued_lease();
        ProviderFuture::ready(Ok(ProviderResolution::with_lease(
            SecretString::new("leased-secret"),
            lease,
        )))
    }

    fn provider_name(&self) -> &str {
        self.name
    }

    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for LeasedMock {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        if !self.handles_lease(lease) {
            return ProviderFuture::ready(Err(ProviderError::NotFound {
                path: format!(
                    "misrouted to {} (lease.provider={})",
                    self.name, lease.provider
                ),
            }));
        }
        self.renew_calls.fetch_add(1, Ordering::Relaxed);
        ProviderFuture::ready(Ok(ProviderResolution::with_lease(
            SecretString::new("renewed-secret"),
            self.issued_lease(),
        )))
    }

    fn revoke<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        if !self.handles_lease(lease) {
            return ProviderFuture::ready(Err(ProviderError::NotFound {
                path: format!(
                    "misrouted to {} (lease.provider={})",
                    self.name, lease.provider
                ),
            }));
        }
        self.revoke_calls.fetch_add(1, Ordering::Relaxed);
        ProviderFuture::ready(Ok(ProviderResolution::empty()))
    }
}

#[tokio::test]
async fn cache_observability_redacts_provider_coordinates_and_lease_identifiers() {
    const REFERENCE_CANARY: &str = "reference-diagnostic-canary";
    const LEASE_CANARY: &str = "lease-diagnostic-canary";

    let inner = LeasedMock::new("vault-stub", LEASE_CANARY, Duration::from_mins(1));
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    let reference = refer(REFERENCE_CANARY);
    let capture = EventCapture::default();

    let _ = layer
        .resolve(&reference)
        .with_subscriber(capture.clone())
        .await
        .expect("initial resolve");
    let _ = layer
        .resolve(&reference)
        .with_subscriber(capture.clone())
        .await
        .expect("cached resolve");

    let leased = layer
        .lease_renewal()
        .expect("cache layer advertises lease capability");
    let _ = leased
        .renew(&inner.issued_lease())
        .with_subscriber(capture.clone())
        .await
        .expect("renew succeeds");

    let _ = layer
        .resolve(&reference)
        .with_subscriber(capture.clone())
        .await
        .expect("resolve after renewal");
    let _ = leased
        .revoke(&inner.issued_lease())
        .with_subscriber(capture.clone())
        .await
        .expect("revoke succeeds");

    let captured = capture.captured();
    assert!(!captured.is_empty(), "redaction gate must capture events");
    assert!(
        !captured.contains(REFERENCE_CANARY),
        "provider reference leaked into tracing output: {captured}"
    );
    assert!(
        !captured.contains(LEASE_CANARY),
        "lease identifier leaked into tracing output: {captured}"
    );
    assert!(captured.contains("cache_outcome=\"hit\""), "{captured}");
    assert!(captured.contains("cache_outcome=\"miss\""), "{captured}");
    assert!(captured.contains("lease_operation=\"renew\""), "{captured}");
    assert!(
        captured.contains("lease_operation=\"revoke\""),
        "{captured}"
    );
    assert!(captured.contains("invalidated_entries=1"), "{captured}");
}

#[tokio::test]
async fn cache_layer_propagates_inner_lease_renewal() {
    // The cache layer surfaces itself (not the inner) as the lease
    // dispatcher so renew/revoke can invalidate cached entries before
    // forwarding to the inner provider. Without this override the
    // base-trait `None` default would shadow the inner's capability;
    // returning the inner verbatim — a tempting alternative — would
    // let renew/revoke bypass the cache.
    let inner = LeasedMock::new("vault-stub", "lease-xyz", Duration::from_mins(1));
    let layer_leased = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );

    let view = layer_leased
        .lease_renewal()
        .expect("cache layer must surface lease capability");
    // The dispatcher is the cache layer itself; `provider_name` reflects
    // the wrapper. Routing into the inner happens via `handles_lease`,
    // which the cache delegates to the wrapped provider.
    assert_eq!(view.provider_name(), "cache(vault-stub)");
    assert!(
        view.handles_lease(&inner.issued_lease()),
        "cache must report it can handle the inner-issued lease"
    );

    // Wrapping a non-leased provider continues to report no capability.
    // `MockProvider` (the existing scaffolding) does not override
    // `lease_renewal`, so it inherits the trait default `None`.
    let plain: Arc<dyn ExternalProvider> =
        MockProvider::new("plain", vec![ok("v", None)]) as Arc<dyn ExternalProvider>;
    let layer_plain = ProviderCacheLayer::new(plain, ProviderCacheConfig::default());
    assert!(
        layer_plain.lease_renewal().is_none(),
        "cache layer over non-leased provider must report no lease capability"
    );
}

#[tokio::test]
async fn cache_layer_revoke_invalidates_matching_entry_and_delegates() {
    // Regression guard for the Codex P1 finding on cache coherence:
    // revoke must drop any cached resolution carrying the revoked
    // lease id, *and* forward to the inner provider.
    let inner = LeasedMock::new("vault-stub", "lease-1", Duration::from_mins(1));
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    let r = refer("secret/leased");

    // Prime the cache with a leased resolution.
    let first = layer.resolve(&r).await.expect("first resolve");
    assert_eq!(first.secret.expose_secret(), "leased-secret");
    assert_eq!(inner.resolve_count(), 1);

    // A second resolve hits the cache (single inner call so far).
    let _ = layer.resolve(&r).await.expect("second resolve cached");
    assert_eq!(inner.resolve_count(), 1, "second resolve served from cache");

    // Revoke the lease via the cache layer's `LeasedProvider` view.
    let view = layer
        .lease_renewal()
        .expect("cache layer advertises lease capability");
    let revoked = view.revoke(&inner.issued_lease()).await.expect("revoke ok");
    assert!(
        revoked.secret.expose_secret().is_empty(),
        "revoke success returns the empty marker (no usable secret)"
    );
    assert_eq!(inner.revoke_count(), 1, "revoke delegated to inner");

    // The next resolve must miss the cache and hit the inner again —
    // proves the revoked entry was actually dropped, not still served.
    let _ = layer.resolve(&r).await.expect("third resolve after revoke");
    assert_eq!(
        inner.resolve_count(),
        2,
        "post-revoke resolve hits the inner provider again"
    );
}

#[tokio::test]
async fn cache_layer_renew_invalidates_matching_entry_and_delegates() {
    // Renew should refresh the cached lease metadata: the simplest
    // correct behaviour is to drop the cached entry so the next
    // resolve picks up the renewed lease/TTL.
    let inner = LeasedMock::new("vault-stub", "lease-2", Duration::from_mins(1));
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    let r = refer("secret/leased-2");

    let _ = layer.resolve(&r).await.expect("first resolve");
    let _ = layer.resolve(&r).await.expect("second resolve cached");
    assert_eq!(inner.resolve_count(), 1);

    let view = layer.lease_renewal().expect("cache leased view");
    let renewed = view.renew(&inner.issued_lease()).await.expect("renew ok");
    assert_eq!(renewed.secret.expose_secret(), "renewed-secret");
    assert_eq!(inner.renew_count(), 1, "renew delegated to inner");

    let _ = layer.resolve(&r).await.expect("resolve post-renew");
    assert_eq!(
        inner.resolve_count(),
        2,
        "post-renew resolve refreshes from inner"
    );
}

#[tokio::test]
async fn cache_layer_revoke_only_invalidates_matching_lease_id() {
    // A revoke must NOT scorch unrelated cached entries — only the
    // ones whose stored resolution carries the same lease id.
    let inner_a = LeasedMock::new("vault-a", "lease-A", Duration::from_mins(1));
    let r_a = ExternalReference {
        provider: ProviderKind::Custom("vault-a".to_owned()),
        path: "secret/a".to_owned(),
        version: None,
        field: None,
    };
    let r_b = ExternalReference {
        provider: ProviderKind::Custom("vault-a".to_owned()),
        path: "secret/b".to_owned(),
        version: None,
        field: None,
    };
    let layer = ProviderCacheLayer::new(
        Arc::clone(&inner_a) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );

    // Two distinct cache keys, same provider + lease id (every resolve
    // returns the same lease in this mock — typical of a Vault dynamic
    // secret bound by lease id, not by path).
    let _ = layer.resolve(&r_a).await.expect("resolve a");
    let _ = layer.resolve(&r_b).await.expect("resolve b");
    assert_eq!(inner_a.resolve_count(), 2);

    // Revoke that lease — both entries share it, so both should drop.
    let view = layer.lease_renewal().expect("leased view");
    view.revoke(&inner_a.issued_lease())
        .await
        .expect("revoke ok");

    // Both keys re-resolve from inner.
    let _ = layer.resolve(&r_a).await.expect("resolve a 2");
    let _ = layer.resolve(&r_b).await.expect("resolve b 2");
    assert_eq!(
        inner_a.resolve_count(),
        4,
        "both shared-lease entries dropped on revoke"
    );

    // Now seed a cached entry with a different lease id and verify a
    // revoke of *that* lease does NOT drop the original mock's entries.
    let inner_c = LeasedMock::new("vault-c", "lease-C", Duration::from_mins(1));
    let layer_c = ProviderCacheLayer::new(
        Arc::clone(&inner_c) as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );
    let _ = layer_c
        .resolve(&refer("secret/c"))
        .await
        .expect("resolve c");
    assert_eq!(inner_c.resolve_count(), 1);

    // Revoke "lease-A" against layer_c — the cache has only
    // "lease-C" entries, so nothing matches; inner_c.revoke gets
    // forwarded the unrelated lease and returns NotFound (handles_lease
    // false). The cache should still be intact.
    let view_c = layer_c.lease_renewal().expect("leased view c");
    let err = view_c
        .revoke(&inner_a.issued_lease())
        .await
        .expect_err("misrouted revoke surfaces NotFound");
    assert!(matches!(err, ProviderError::NotFound { .. }));

    let _ = layer_c
        .resolve(&refer("secret/c"))
        .await
        .expect("resolve c 2");
    assert_eq!(
        inner_c.resolve_count(),
        1,
        "unrelated revoke did not invalidate the c entry"
    );
}
