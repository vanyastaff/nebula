//! Lease lifecycle subsystem — engine-side consumption of the
//! [`LeasedProvider`] capability introduced by.
//!
//! # Role
//!
//! Phases A–C of the follow-up landed the lease primitives
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
//! ```rust
//! use nebula_credential::runtime::{LeaseLifecycle, LeaseLifecycleConfig};
//! use tokio_util::sync::CancellationToken;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let shutdown = CancellationToken::new();
//! let lifecycle = LeaseLifecycle::spawn(
//!     LeaseLifecycleConfig::default(),
//!     None, // optional EventBus<LeaseEvent>
//!     None, // optional MetricsEmitter
//!     shutdown.clone(),
//! );
//!
//! // When a credential resolution carries a lease, register it:
//! // let token = lifecycle.track(provider, resolution, Some(credential_id)).await?;
//!
//! // On credential rotation commit:
//! // lifecycle.revoke_for_credential(credential_id).await;
//!
//! // Stop the background scheduler; dropping the runtime aborts its task.
//! shutdown.cancel();
//! # let _ = lifecycle;
//! # }
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

use crate::{CredentialId, LeaseEvent, LeasedProvider, ProviderError, ProviderResolution};
use nebula_core::accessor::MetricsEmitter;
use nebula_eventbus::EventBus;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub use policy::{RenewalPolicy, StalenessCeiling, StalenessCeilingError};
pub use registry::LeaseToken;
pub use scheduler::LeaseLifecycleConfig;

use scheduler::{Command, RevokeOutcome, SchedulerInputs};

/// Bound on the in-flight lease-lifecycle command queue.
///
/// The channel has a single consumer (the scheduler task), and each
/// `Track` / `Revoke` it dequeues can block that consumer for up to one
/// `provider_call_timeout` (default 30s) inside `provider.renew` /
/// `provider.revoke`. An unbounded queue therefore lets a registration
/// burst against a slow or wedged backend grow without limit → OOM.
///
/// This capacity is sized to absorb the expected concurrent
/// lease-acquisition burst that can pile up while the consumer is busy
/// with one provider call: a few hundred slots is far more than any
/// realistic number of distinct dynamic-secret leases acquired
/// simultaneously on one engine, while still being a hard ceiling that
/// converts a pathological backend stall into a fast typed error
/// ([`LeaseLifecycleError::LifecycleBusy`]) instead of unbounded memory
/// growth. It is a deliberate, documented bound — not a tuning knob and
/// not a magic literal.
pub(super) const LEASE_COMMAND_CHANNEL_CAPACITY: usize = 256;

/// Public handle to the lease lifecycle scheduler.
///
/// Cheap to clone — internally an `Arc` over a **bounded** command
/// channel (`LEASE_COMMAND_CHANNEL_CAPACITY`). The scheduler task is
/// spawned at construction time and runs until the supplied
/// [`CancellationToken`] fires. Backpressure is **fail-fast, not
/// blocking**: when the queue is full the producing call returns
/// [`LeaseLifecycleError::LifecycleBusy`] immediately (`try_send`)
/// rather than parking the caller behind a wedged backend — a stalled
/// lease subsystem must never stall the credential-resolution path that
/// feeds it.
#[derive(Clone)]
pub struct LeaseLifecycle {
    inner: Arc<LeaseLifecycleInner>,
}

struct LeaseLifecycleInner {
    commands: mpsc::Sender<Command>,
    shutdown: CancellationToken,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LeaseLifecycle {
    /// Spawn the lease lifecycle scheduler.
    pub fn spawn(
        config: LeaseLifecycleConfig,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        metrics: Option<Arc<dyn MetricsEmitter>>,
        shutdown: CancellationToken,
    ) -> Self {
        let (tx, rx) = mpsc::channel(LEASE_COMMAND_CHANNEL_CAPACITY);
        let inputs = SchedulerInputs {
            config,
            commands: rx,
            lease_bus,
            metrics,
            shutdown: shutdown.clone(),
        };
        let task = tokio::spawn(scheduler::run(inputs));
        Self {
            inner: Arc::new(LeaseLifecycleInner {
                commands: tx,
                shutdown,
                task: tokio::sync::Mutex::new(Some(task)),
            }),
        }
    }

    /// Cancel and join the scheduler task.
    ///
    /// Provider futures are cancelled by aborting the task after signalling
    /// cooperative shutdown, so this method is bounded even when a provider
    /// call is stalled.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let task = self.inner.task.lock().await.take();
        if let Some(task) = task {
            task.abort();
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                tracing::error!(%error, "credential lease scheduler task failed during shutdown");
            }
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
            .try_send(Command::Track {
                provider,
                lease,
                credential_id,
                reply: reply_tx,
            })
            .map_err(send_error)?;
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
            .try_send(Command::Revoke {
                token,
                reply: reply_tx,
            })
            .map_err(send_error)?;
        let outcome = reply_rx.await.map_err(|_| LeaseLifecycleError::Shutdown)?;
        match outcome {
            RevokeOutcome::Revoked | RevokeOutcome::Unknown => Ok(()),
            RevokeOutcome::ProviderFailed(err) => Err(LeaseLifecycleError::Revoke(err)),
        }
    }

    /// Revoke every tracked lease attributed to `credential_id`.
    ///
    /// Returns the number of leases successfully revoked. Failed revokes
    /// do not propagate — they are logged + emitted as audit events but
    /// the rotation pipeline that called this MUST NOT block on them.
    /// See spec : revoke-on-rotate is best-effort cleanup.
    pub async fn revoke_for_credential(&self, credential_id: CredentialId) -> usize {
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(err) = self.inner.commands.try_send(Command::RevokeForCredential {
            credential_id,
            reply: reply_tx,
        }) {
            // Best-effort cleanup per the revoke-on-rotate contract: a
            // saturated or shut-down lease subsystem must not block (or
            // fail) the rotation pipeline that called this. The degraded
            // state is observable via this warn (the queue-full and
            // shutdown cases are distinguished so an operator can tell a
            // wedged backend from a stopped lifecycle).
            let reason = match err {
                mpsc::error::TrySendError::Full(_) => {
                    "command queue saturated (slow/wedged backend)"
                },
                mpsc::error::TrySendError::Closed(_) => "lease lifecycle is shut down",
            };
            tracing::warn!(
                target: "nebula_credential::runtime::lease",
                %credential_id,
                reason,
                "revoke_for_credential is a no-op"
            );
            return 0;
        }
        reply_rx.await.unwrap_or_else(|_| {
            tracing::warn!(
                target: "nebula_credential::runtime::lease",
                %credential_id,
                "lease lifecycle dropped reply during revoke_for_credential"
            );
            0
        })
    }

    /// Current count of tracked leases. Useful for observability and
    /// shutdown drains. A return value of `0` is overloaded — it can
    /// mean either "no leases" or "scheduler is gone"; the latter case
    /// emits a `warn` at this site so the degraded state is observable.
    pub async fn active_lease_count(&self) -> usize {
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(err) = self
            .inner
            .commands
            .try_send(Command::Snapshot { reply: reply_tx })
        {
            let reason = match err {
                mpsc::error::TrySendError::Full(_) => {
                    "command queue saturated (slow/wedged backend)"
                },
                mpsc::error::TrySendError::Closed(_) => "lease lifecycle is shut down",
            };
            tracing::warn!(
                target: "nebula_credential::runtime::lease",
                reason,
                "active_lease_count returns 0"
            );
            return 0;
        }
        reply_rx.await.unwrap_or_else(|_| {
            tracing::warn!(
                target: "nebula_credential::runtime::lease",
                "lease lifecycle dropped reply during active_lease_count"
            );
            0
        })
    }
}

impl Drop for LeaseLifecycleInner {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
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
    ///
    /// Carries the underlying [`ProviderError`] so callers can
    /// distinguish transient (`Unavailable`) from auth
    /// (`AccessDenied`) failure modes and walk the source chain for
    /// observability, rather than parsing a stringified message.
    #[error("provider revoke failed: {0}")]
    Revoke(#[source] ProviderError),

    /// The scheduler task has shut down (cancellation token fired) and
    /// is no longer accepting commands.
    #[error("lease lifecycle is shut down")]
    Shutdown,

    /// The bounded lease-lifecycle command queue
    /// (`LEASE_COMMAND_CHANNEL_CAPACITY` in flight) is full because the
    /// single scheduler consumer is blocked on a slow or wedged provider
    /// `renew` / `revoke`. This is explicit fail-fast backpressure: the
    /// command was **not** enqueued and the lease was **not** tracked /
    /// revoked. The caller decides how to react (retry later, surface a
    /// degraded-mode error) — the lease subsystem must never block the
    /// credential path behind a stalled backend, nor grow this queue
    /// without bound. Transient: a later attempt can succeed once the
    /// consumer drains.
    #[error(
        "lease lifecycle command queue is full ({LEASE_COMMAND_CHANNEL_CAPACITY} in flight) — \
         scheduler consumer is blocked on a slow provider; command not enqueued"
    )]
    LifecycleBusy,
}

/// Maps a bounded-channel `try_send` failure to the typed public error.
///
/// `Full` is explicit fail-fast backpressure
/// ([`LeaseLifecycleError::LifecycleBusy`]); `Closed` means the scheduler
/// task is gone ([`LeaseLifecycleError::Shutdown`]). The dropped command
/// (and its `oneshot` reply) are discarded with the error — the caller
/// sees a typed failure rather than a lost in-flight request.
fn send_error<T>(err: mpsc::error::TrySendError<T>) -> LeaseLifecycleError {
    match err {
        mpsc::error::TrySendError::Full(_) => LeaseLifecycleError::LifecycleBusy,
        mpsc::error::TrySendError::Closed(_) => LeaseLifecycleError::Shutdown,
    }
}

#[cfg(test)]
mod tests;
