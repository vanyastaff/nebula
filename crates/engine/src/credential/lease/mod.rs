//! Lease lifecycle subsystem — engine-side consumption of the
//! [`LeasedProvider`] capability introduced by ADR-0051.
//!
//! # Role
//!
//! Phases A–C of the ADR-0051 follow-up landed the lease primitives
//! (envelope, capability sub-trait, cache layer, first concrete Vault
//! impl). Nothing in the engine called `renew` or `revoke` — a Vault
//! dynamic-secret lease would silently expire unless someone happened to
//! resolve the credential again before the TTL elapsed. This module is
//! the missing consumer: a single background task per engine that
//! tracks leases, renews them at 70% of TTL, and revokes them on
//! credential rotation.
//!
//! # Surface
//!
//! - [`LeaseLifecycle`] — `Arc`-shared public handle. Construction is
//!   via [`LeaseLifecycle::spawn`]; callers send work through the
//!   `track` / `revoke` / `revoke_for_credential` async methods.
//! - [`LeaseLifecycleError`] — typed errors returned to the caller. The
//!   scheduler task itself never panics; transient renewal failures are
//!   absorbed into the backoff schedule.
//! - [`LeaseLifecycleConfig`] / [`RenewalPolicy`] — tuning knobs. Defaults
//!   match Vault Agent guidance.
//! - [`LeaseToken`] — opaque registry key returned by `track`.
//!
//! # Wiring
//!
//! ```ignore
//! use std::sync::Arc;
//! use nebula_engine::credential::lease::{
//!     LeaseLifecycle, LeaseLifecycleConfig,
//! };
//! use tokio_util::sync::CancellationToken;
//!
//! let shutdown = CancellationToken::new();
//! let lifecycle = LeaseLifecycle::spawn(
//!     LeaseLifecycleConfig::default(),
//!     None,           // optional EventBus<LeaseEvent>
//!     None,           // optional MetricsEmitter
//!     shutdown.clone(),
//! );
//!
//! // When a credential resolution carries a lease, register it:
//! // let token = lifecycle.track(provider, resolution, Some(credential_id)).await?;
//!
//! // On credential rotation commit:
//! // lifecycle.revoke_for_credential(credential_id).await;
//! ```
//!
//! # Single-replica
//!
//! Phase D ships single-replica semantics — every engine that calls
//! `track` will independently try to renew. Cross-replica coordination
//! (only one replica per lease renews) is a separate follow-up.

mod policy;
mod registry;
mod scheduler;

use std::sync::Arc;

use nebula_core::accessor::MetricsEmitter;
use nebula_credential::{CredentialId, LeaseEvent, LeasedProvider, ProviderResolution};
use nebula_eventbus::EventBus;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub use policy::RenewalPolicy;
pub use registry::LeaseToken;
pub use scheduler::LeaseLifecycleConfig;

use scheduler::{Command, RevokeOutcome, SchedulerInputs};

/// Public handle to the lease lifecycle scheduler.
///
/// Cheap to clone — internally an `Arc` over an unbounded command
/// channel. The scheduler task is spawned at construction time and
/// runs until the supplied [`CancellationToken`] fires.
#[derive(Clone)]
pub struct LeaseLifecycle {
    inner: Arc<LeaseLifecycleInner>,
}

struct LeaseLifecycleInner {
    commands: mpsc::UnboundedSender<Command>,
}

impl LeaseLifecycle {
    /// Spawn the lease lifecycle scheduler.
    pub fn spawn(
        config: LeaseLifecycleConfig,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        metrics: Option<Arc<dyn MetricsEmitter>>,
        shutdown: CancellationToken,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let inputs = SchedulerInputs {
            config,
            commands: rx,
            lease_bus,
            metrics,
            shutdown,
        };
        tokio::spawn(scheduler::run(inputs));
        Self {
            inner: Arc::new(LeaseLifecycleInner { commands: tx }),
        }
    }

    /// Register a lease for proactive renewal.
    ///
    /// `resolution` must carry a `Some(lease)`; orphan resolutions
    /// (`lease.is_none()`) are rejected with
    /// [`LeaseLifecycleError::ResolutionMissingLease`].
    ///
    /// `credential_id` attributes the lease to a nebula credential
    /// record so [`revoke_for_credential`](Self::revoke_for_credential)
    /// can scan-and-revoke on rotation. Pass `None` for ad-hoc
    /// provider use without a credential record.
    pub async fn track(
        &self,
        provider: Arc<dyn LeasedProvider>,
        resolution: ProviderResolution,
        credential_id: Option<CredentialId>,
    ) -> Result<LeaseToken, LeaseLifecycleError> {
        let lease = resolution
            .lease
            .ok_or(LeaseLifecycleError::ResolutionMissingLease)?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.inner
            .commands
            .send(Command::Track {
                provider,
                lease,
                credential_id,
                reply: reply_tx,
            })
            .map_err(|_| LeaseLifecycleError::Shutdown)?;
        reply_rx.await.map_err(|_| LeaseLifecycleError::Shutdown)
    }

    /// Revoke a tracked lease through its issuing provider and remove
    /// it from the registry. Returns `Ok(())` for both success and the
    /// "lease already gone" case; provider-side failures are surfaced
    /// as [`LeaseLifecycleError::Revoke`].
    pub async fn revoke(&self, token: LeaseToken) -> Result<(), LeaseLifecycleError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.inner
            .commands
            .send(Command::Revoke {
                token,
                reply: reply_tx,
            })
            .map_err(|_| LeaseLifecycleError::Shutdown)?;
        let outcome = reply_rx.await.map_err(|_| LeaseLifecycleError::Shutdown)?;
        match outcome {
            RevokeOutcome::Revoked | RevokeOutcome::Unknown => Ok(()),
            RevokeOutcome::ProviderFailed(reason) => Err(LeaseLifecycleError::Revoke { reason }),
        }
    }

    /// Revoke every tracked lease attributed to `credential_id`.
    ///
    /// Returns the number of leases successfully revoked. Failed revokes
    /// do not propagate — they are logged + emitted as audit events but
    /// the rotation pipeline that called this MUST NOT block on them.
    /// See spec §4: revoke-on-rotate is best-effort cleanup.
    pub async fn revoke_for_credential(&self, credential_id: CredentialId) -> usize {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .inner
            .commands
            .send(Command::RevokeForCredential {
                credential_id,
                reply: reply_tx,
            })
            .is_err()
        {
            return 0;
        }
        reply_rx.await.unwrap_or(0)
    }

    /// Current count of tracked leases. Useful for observability and
    /// shutdown drains.
    pub async fn active_lease_count(&self) -> usize {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .inner
            .commands
            .send(Command::Snapshot { reply: reply_tx })
            .is_err()
        {
            return 0;
        }
        reply_rx.await.unwrap_or(0)
    }
}

/// Errors returned from the [`LeaseLifecycle`] public surface.
///
/// The scheduler task itself never panics and absorbs transient renewal
/// failures into its backoff schedule; the errors here are only the
/// public-facing ones a caller can act on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeaseLifecycleError {
    /// `track` was called with a `ProviderResolution` whose `lease`
    /// field was `None`. Caller should resolve through a leased
    /// provider before invoking the lifecycle.
    #[error("provider resolution carries no lease — nothing to track")]
    ResolutionMissingLease,

    /// The provider returned an error from `revoke`. The lease has
    /// still been removed from the registry; future `track` calls with
    /// the same lease id will create a fresh entry.
    #[error("provider revoke failed: {reason}")]
    Revoke {
        /// Provider error message.
        reason: String,
    },

    /// The scheduler task has shut down (cancellation token fired) and
    /// is no longer accepting commands.
    #[error("lease lifecycle is shut down")]
    Shutdown,
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use nebula_credential::{
        CredentialId, ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider,
        ProviderError, ProviderFuture, ProviderResolution, SecretString,
    };
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::credential::lease::scheduler::LeaseLifecycleConfig;

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
            let new_ttl =
                Duration::from_secs(u64::from(self.renew_ttl_secs.load(Ordering::SeqCst)));
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
}
