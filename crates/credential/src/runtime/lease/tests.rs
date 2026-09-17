use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::{
    CredentialId, ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider, ProviderError,
    ProviderFuture, ProviderResolution, SecretString,
};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::runtime::lease::scheduler::LeaseLifecycleConfig;

// ────────────────────────────────────────────────────────────────────
// Mock leased provider with configurable renew / revoke behaviour.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct CountingProvider {
    renew_calls: AtomicU32,
    revoke_calls: AtomicU32,
    renew_failures: AtomicU32,
    /// Number of times to fail renew with `Unavailable` before
    /// succeeding. 0 = always succeed.
    unavailable_until: AtomicU32,
    /// If true, every renew returns `NotFound`.
    permanent_not_found: AtomicU32,
    /// TTL reported on each renew success.
    renew_ttl_secs: AtomicU32,
    name: &'static str,
}

impl CountingProvider {
    fn new(name: &'static str, renew_ttl_secs: u32) -> Arc<Self> {
        let p = Arc::new(Self {
            name,
            ..Self::default()
        });
        p.renew_ttl_secs.store(renew_ttl_secs, Ordering::SeqCst);
        p
    }

    fn fail_n_then_succeed(self: &Arc<Self>, n: u32) {
        self.unavailable_until.store(n, Ordering::SeqCst);
    }

    fn always_not_found(self: &Arc<Self>) {
        self.permanent_not_found.store(1, Ordering::SeqCst);
    }
}

impl ExternalProvider for CountingProvider {
    fn resolve<'a>(&'a self, _r: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            "ignored",
        ))))
    }

    fn provider_name(&self) -> &str {
        self.name
    }

    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for CountingProvider {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        self.renew_calls.fetch_add(1, Ordering::SeqCst);
        if self.permanent_not_found.load(Ordering::SeqCst) > 0 {
            self.renew_failures.fetch_add(1, Ordering::SeqCst);
            return ProviderFuture::ready(Err(ProviderError::NotFound {
                path: format!("lease {} gone", lease.lease_id),
            }));
        }
        let remaining = self.unavailable_until.load(Ordering::SeqCst);
        if remaining > 0 {
            self.unavailable_until
                .store(remaining - 1, Ordering::SeqCst);
            self.renew_failures.fetch_add(1, Ordering::SeqCst);
            return ProviderFuture::ready(Err(ProviderError::Unavailable {
                reason: "transient".to_owned(),
            }));
        }
        let new_ttl = Duration::from_secs(u64::from(self.renew_ttl_secs.load(Ordering::SeqCst)));
        let refreshed = LeaseHandle::new(
            Cow::Borrowed(self.name),
            lease.lease_id.clone(),
            chrono::Utc::now(),
            new_ttl,
        );
        ProviderFuture::ready(Ok(ProviderResolution::with_lease(
            SecretString::new("renewed"),
            refreshed,
        )))
    }

    fn revoke<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        ProviderFuture::ready(Ok(ProviderResolution::empty()))
    }
}

fn make_resolution(provider: &str, ttl_secs: u64, id: &str) -> ProviderResolution {
    let lease = LeaseHandle::new(
        Cow::Owned(provider.to_owned()),
        id,
        chrono::Utc::now(),
        Duration::from_secs(ttl_secs),
    );
    ProviderResolution::with_lease(SecretString::new("secret"), lease)
}

// ────────────────────────────────────────────────────────────────────
// Tests — exercise the scheduler with `tokio::time::pause` so renew
// attempts fire deterministically.
// ────────────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn track_returns_token_and_increments_active_count() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);

    let token = lifecycle
        .track(
            provider as Arc<dyn LeasedProvider>,
            make_resolution("mock", 100, "lease-1"),
            None,
        )
        .await
        .expect("track succeeds");

    let count = lifecycle.active_lease_count().await;
    assert_eq!(count, 1);
    assert_eq!(token, LeaseToken(0));

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn track_rejects_resolution_without_lease() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);

    let result = lifecycle
        .track(
            provider as Arc<dyn LeasedProvider>,
            ProviderResolution::from_secret(SecretString::new("static")),
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(LeaseLifecycleError::ResolutionMissingLease)
    ));

    shutdown.cancel();
}

/// Regression guard for Codex P1: a lease registered with an
/// `issued_at` already in the past (cached resolution, slow
/// composition root) must schedule its first renewal off the lease
/// issue time, not off "now". A 100s lease that was issued 60s ago
/// should fire after ~10s (70s renewal point minus 60s already
/// elapsed), not after a fresh 70s window.
#[tokio::test(start_paused = true)]
async fn scheduler_first_renew_accounts_for_lease_age() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    // Back-date the lease's `issued_at` by 60 seconds.
    let backdated = LeaseHandle::new(
        Cow::Borrowed("mock"),
        "aged-lease",
        chrono::Utc::now() - chrono::Duration::seconds(60),
        Duration::from_secs(100),
    );
    let resolution = ProviderResolution::with_lease(SecretString::new("secret"), backdated);

    lifecycle
        .track(provider_arc, resolution, None)
        .await
        .expect("track ok");

    // Only 10s of remaining-to-renew (70% of 100s = 70s, minus 60s
    // already aged). Advancing by 11s tokio-time should fire.
    tokio::time::advance(Duration::from_secs(11)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        provider.renew_calls.load(Ordering::SeqCst),
        1,
        "aged lease should renew quickly, not wait a full 70s window"
    );

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn scheduler_renews_at_seventy_percent_of_ttl() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    lifecycle
        .track(provider_arc, make_resolution("mock", 100, "lease-x"), None)
        .await
        .expect("track ok");

    // 70% of 100s = 70s. Advance just under: no renew yet.
    tokio::time::advance(Duration::from_secs(69)).await;
    assert_eq!(provider.renew_calls.load(Ordering::SeqCst), 0);

    // Advance past 70s — renew fires.
    tokio::time::advance(Duration::from_secs(2)).await;
    // Give the scheduler one yield to run the renew future.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(provider.renew_calls.load(Ordering::SeqCst), 1);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn unavailable_triggers_backoff_then_recovers() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    provider.fail_n_then_succeed(2);
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    lifecycle
        .track(provider_arc, make_resolution("mock", 100, "lease-y"), None)
        .await
        .expect("track ok");

    // First renew at 70s — fails (Unavailable).
    tokio::time::advance(Duration::from_secs(71)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.renew_failures.load(Ordering::SeqCst), 1);
    // Backoff = 1s; next attempt at ~71s + 1s.
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.renew_failures.load(Ordering::SeqCst), 2);
    // Backoff = 2s; next attempt succeeds.
    tokio::time::advance(Duration::from_secs(3)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.renew_calls.load(Ordering::SeqCst), 3);
    // Lease should still be active.
    assert_eq!(lifecycle.active_lease_count().await, 1);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn not_found_drops_lease_immediately() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    provider.always_not_found();
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    lifecycle
        .track(provider_arc, make_resolution("mock", 100, "lease-z"), None)
        .await
        .expect("track ok");

    tokio::time::advance(Duration::from_secs(71)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.renew_calls.load(Ordering::SeqCst), 1);
    assert_eq!(lifecycle.active_lease_count().await, 0);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn revoke_calls_provider_and_removes_from_registry() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    let token = lifecycle
        .track(provider_arc, make_resolution("mock", 100, "lease-r"), None)
        .await
        .expect("track ok");
    lifecycle.revoke(token).await.expect("revoke ok");
    assert_eq!(provider.revoke_calls.load(Ordering::SeqCst), 1);
    assert_eq!(lifecycle.active_lease_count().await, 0);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn revoke_for_credential_revokes_only_matching_entries() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    let target_id = CredentialId::new();
    let other_id = CredentialId::new();

    lifecycle
        .track(
            Arc::clone(&provider) as Arc<dyn LeasedProvider>,
            make_resolution("mock", 100, "lease-a"),
            Some(target_id),
        )
        .await
        .unwrap();
    lifecycle
        .track(
            Arc::clone(&provider) as Arc<dyn LeasedProvider>,
            make_resolution("mock", 100, "lease-b"),
            Some(target_id),
        )
        .await
        .unwrap();
    lifecycle
        .track(
            Arc::clone(&provider) as Arc<dyn LeasedProvider>,
            make_resolution("mock", 100, "lease-c"),
            Some(other_id),
        )
        .await
        .unwrap();

    let revoked = lifecycle.revoke_for_credential(target_id).await;
    assert_eq!(revoked, 2);
    // One unrelated lease should remain.
    assert_eq!(lifecycle.active_lease_count().await, 1);
    assert_eq!(provider.revoke_calls.load(Ordering::SeqCst), 2);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn shutdown_drains_registry_without_blocking() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = CountingProvider::new("mock", 100);
    let provider_arc = Arc::clone(&provider) as Arc<dyn LeasedProvider>;

    lifecycle
        .track(provider_arc, make_resolution("mock", 100, "lease-s"), None)
        .await
        .expect("track ok");

    shutdown.cancel();
    // Give the task one tick to observe cancellation.
    tokio::task::yield_now().await;
    // The lifecycle handle now returns 0 because the channel is
    // closed; the public surface degrades gracefully.
    let count = lifecycle.active_lease_count().await;
    assert_eq!(count, 0);
    // We did NOT attempt upstream revoke on shutdown.
    assert_eq!(provider.revoke_calls.load(Ordering::SeqCst), 0);
}

// budget-justified: bounded-channel saturation characterization
// harness — a hang-proof parked-consumer fixture plus the typed
// LifecycleBusy assertion form one cohesive scenario; splitting it
// would obscure the backpressure path under test.
/// Provider whose `revoke` never resolves, used to park the single
/// scheduler consumer so the bounded command channel can be filled
/// from the producer side.
#[derive(Debug)]
struct HangingRevokeProvider {
    name: &'static str,
}

impl ExternalProvider for HangingRevokeProvider {
    fn resolve<'a>(&'a self, _r: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            "ignored",
        ))))
    }

    fn provider_name(&self) -> &str {
        self.name
    }

    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for HangingRevokeProvider {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        // Renew is irrelevant here; succeed trivially.
        let refreshed = LeaseHandle::new(
            Cow::Borrowed("hanging"),
            lease.lease_id.clone(),
            chrono::Utc::now(),
            Duration::from_secs(100),
        );
        ProviderFuture::ready(Ok(ProviderResolution::with_lease(
            SecretString::new("renewed"),
            refreshed,
        )))
    }

    fn revoke<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        // Never resolves: the consumer parks here for the whole
        // `provider_call_timeout`, modelling a wedged backend.
        ProviderFuture::new(async {
            std::future::pending::<()>().await;
            Ok(ProviderResolution::empty())
        })
    }
}

/// A registration burst against a wedged backend must surface a typed
/// saturation error to the producer instead of growing the command
/// queue without bound.
///
/// The single scheduler consumer is parked inside a never-resolving
/// `revoke` (the per-call timeout is set huge so it cannot rescue the
/// consumer within the test). With the consumer parked, the producer
/// keeps issuing `track` commands; once the bounded channel is full a
/// further `track` must fail fast with
/// [`LeaseLifecycleError::LifecycleBusy`] rather than block or grow
/// memory unbounded. (Red against an unbounded channel: every `track`
/// send succeeds, so the busy error is never observed.)
#[tokio::test]
async fn track_surfaces_typed_saturation_when_consumer_parked() {
    let shutdown = CancellationToken::new();
    // Default `provider_call_timeout` (30s) is far longer than the
    // detect window below: the parked `revoke` cannot time out and
    // free the consumer before the test has observed saturation and
    // torn down.
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = Arc::new(HangingRevokeProvider { name: "hanging" }) as Arc<dyn LeasedProvider>;

    // Track one lease, then revoke it: the revoke parks the single
    // consumer permanently (its `revoke` future never resolves).
    let first = lifecycle
        .track(
            Arc::clone(&provider),
            make_resolution("hanging", 100, "park-lease"),
            None,
        )
        .await;
    assert!(
        first.is_ok(),
        "first track must succeed before the consumer is parked: {first:?}"
    );
    let Ok(token) = first else { return };
    let revoke_handle = {
        let lifecycle = lifecycle.clone();
        tokio::spawn(async move { lifecycle.revoke(token).await })
    };
    // Yield enough for the scheduler to dequeue the Revoke command and
    // enter the never-resolving provider call.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    // The consumer is now wedged. Flood `track` from many tasks. A
    // `track` whose `try_send` *succeeds* parks forever on its reply
    // (the consumer never drains), so those join handles never
    // resolve — the test must NOT await them. A `track` whose
    // `try_send` is rejected returns `LifecycleBusy` **synchronously**
    // (before any `.await`), so that task finishes almost
    // immediately. We therefore detect saturation by polling for any
    // *finished* task that returned `LifecycleBusy`, under an overall
    // bounded timeout so this can never hang (an unbounded — buggy —
    // channel never rejects, so no task finishes with the busy error
    // and the timeout makes the test fail loudly instead of growing
    // the queue without limit).
    let mut pending = Vec::new();
    for i in 0..(LEASE_COMMAND_CHANNEL_CAPACITY * 4 + 64) {
        let lifecycle = lifecycle.clone();
        let provider = Arc::clone(&provider);
        let id = format!("burst-{i}");
        let h = tokio::spawn(async move {
            lifecycle
                .track(provider, make_resolution("hanging", 100, &id), None)
                .await
        });
        pending.push(h);
        // Let the send actually attempt against the bounded channel.
        tokio::task::yield_now().await;
    }

    let detect = async {
        loop {
            // Drain only the *finished* handles. A `LifecycleBusy`
            // result is produced before any await, so its task is
            // finished here; parked-on-reply tasks are not and are
            // left untouched (never awaited).
            let mut found = false;
            let mut still_pending = Vec::with_capacity(pending.len());
            for h in std::mem::take(&mut pending) {
                if h.is_finished() {
                    if matches!(h.await, Ok(Err(LeaseLifecycleError::LifecycleBusy))) {
                        found = true;
                    }
                } else {
                    still_pending.push(h);
                }
            }
            pending = still_pending;
            if found {
                break;
            }
            tokio::task::yield_now().await;
        }
    };

    let saw_busy = tokio::time::timeout(Duration::from_secs(20), detect)
        .await
        .is_ok();
    assert!(
        saw_busy,
        "a registration burst against a parked consumer must surface \
         LeaseLifecycleError::LifecycleBusy (bounded channel), not grow \
         the queue unbounded"
    );

    // Tear down without awaiting the forever-parked tasks.
    shutdown.cancel();
    revoke_handle.abort();
    for h in pending {
        h.abort();
    }
}
