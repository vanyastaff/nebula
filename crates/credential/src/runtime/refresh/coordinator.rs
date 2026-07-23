//! Outer two-tier refresh coordinator.
//!
//! See `docs/INTEGRATION_MODEL.md` for the two-tier refresh diagram, parameter invariants, and
//! contention backoff.
//!
//! `RefreshCoordinator` composes:
//!
//! - **L1** -- `super::l1::L1RefreshCoalescer` (in-process oneshot coalesce
//!   + per-credential circuit breaker + global concurrency semaphore).
//! - **L2** -- `Arc<dyn nebula_storage_port::store::RefreshClaimStore>` (durable CAS-based claim
//!   with TTL + heartbeat).
//!
//! Callers invoke `refresh_coalesced(credential_id, do_refresh)`. The
//! coordinator acquires L1 first (fast in-process coalesce), then a
//! durable L2 claim with contention backoff, runs the user's refresh
//! closure under both locks, then finalizes L1 synchronously and L2 according
//! to the returned replay-safety disposition.

use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_core::CredentialId;
use nebula_storage_port::store::{
    ClaimAttempt, ClaimToken, HeartbeatError, RefreshClaim, RefreshClaimError as RepoError,
    RefreshClaimStore as RefreshClaimRepo, ReplicaId,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::audit::AuditSink;

use super::{
    audit::emit_claim_acquired,
    l1::{L1Completion, L1RefreshCoalescer},
    metrics::RefreshCoordMetrics,
};

// ──────────────────────────────────────────────────────────────────────────
// Configuration
// ──────────────────────────────────────────────────────────────────────────

/// Configuration knobs for the two-tier coordinator.
///
/// Per sub-spec the four time-related parameters carry interlocking
/// invariants verified by [`RefreshCoordConfig::validate`]:
///
/// - `heartbeat_interval × 3 <= claim_ttl` -- three heartbeat ticks must fit inside one claim TTL
///   so two consecutive missed heartbeats still leave the claim valid until the next tick.
/// - `refresh_timeout + 2 × heartbeat_interval <= claim_ttl` -- the caller-wait budget expires
///   while at least two heartbeat opportunities remain inside the original TTL. The owned
///   provider/persistence task is not cancelled at this point.
/// - `reclaim_sweep_interval <= claim_ttl` -- sweeps must run at least as often as a claim's TTL
///   so a crashed holder is accounted within one TTL window. Expired normal claims are reclaimed;
///   expired provider-side-effect claims are retained as poison.
///
/// The boundary case `heartbeat_interval × 3 == claim_ttl` is allowed
/// (mirrors the execution-lease shape: `ttl / 3 ==
/// heartbeat_interval`).
///
/// CI test asserts `RefreshCoordConfig::default().validate().is_ok()`.
#[derive(Clone, Debug)]
pub struct RefreshCoordConfig {
    /// Claim TTL applied to every L2 acquire/heartbeat call.
    pub claim_ttl: Duration,
    /// Cadence of background heartbeat ticks while a claim is held.
    pub heartbeat_interval: Duration,
    /// Per-phase wait budget for an L1 waiter, L2 contention, and the owned
    /// refresh task.
    ///
    /// Each phase consumes at most one such budget; this is not a single
    /// end-to-end deadline. Expiry never cancels provider/persistence work
    /// after the sentinel boundary.
    pub refresh_timeout: Duration,
    /// Cadence of the background reclaim sweep (Stage 3.3).
    pub reclaim_sweep_interval: Duration,
    /// Distinct accounted incidents inside `sentinel_window` required before
    /// emitting the `ReauthRequired` escalation decision/observation.
    ///
    /// The threshold does not mutate the credential aggregate; the
    /// owner-qualified durable command is K3 work.
    pub sentinel_threshold: u32,
    /// Rolling window for sentinel-event counting (Stage 3.2).
    pub sentinel_window: Duration,
}

impl Default for RefreshCoordConfig {
    fn default() -> Self {
        Self {
            claim_ttl: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            refresh_timeout: Duration::from_secs(8),
            reclaim_sweep_interval: Duration::from_secs(30),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        }
    }
}

/// Validation errors for [`RefreshCoordConfig`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A duration used as a lease or Tokio interval was zero.
    #[error("config field {field} must be greater than zero")]
    ZeroDuration {
        /// Field whose zero value would make lease semantics invalid or panic
        /// `tokio::time::interval`.
        field: &'static str,
    },
    /// A zero sentinel threshold would escalate every accounted event and is
    /// almost certainly a deployment mistake.
    #[error("sentinel_threshold must be greater than zero")]
    ZeroSentinelThreshold,
    /// `heartbeat_interval × 3` exceeds `claim_ttl` -- three heartbeat
    /// ticks would not fit inside one TTL window.
    #[error("heartbeat_interval \u{d7} 3 must be \u{2264} claim_ttl")]
    HeartbeatTooSlow,
    /// `refresh_timeout + 2 × heartbeat_interval` exceeds `claim_ttl` --
    /// a caller could stop waiting without two heartbeat opportunities left
    /// inside the original claim TTL.
    #[error("refresh_timeout + 2 \u{d7} heartbeat_interval must be \u{2264} claim_ttl")]
    RefreshTimeoutTooLong,
    /// `reclaim_sweep_interval` exceeds `claim_ttl`.
    #[error("reclaim_sweep_interval must be \u{2264} claim_ttl")]
    ReclaimTooSlow,
    /// A computed Duration overflowed during invariant validation. The
    /// surfaced field name lets operators spot which knob to bound (e.g.
    /// `heartbeat_interval × 3` or `refresh_timeout + 2 × heartbeat_interval`).
    /// `validate()` MUST surface bad config as a typed error rather than
    /// panicking inside the very fn meant to detect bad config.
    #[error("config field {field} overflowed Duration during invariant check (value: {value:?})")]
    Overflow {
        /// Logical name of the operand that overflowed.
        field: &'static str,
        /// The pre-overflow operand; useful in operator messages.
        value: Duration,
    },

    /// Metric primitive registration failed for the coordinator's series.
    #[error("telemetry metrics error: {0}")]
    Telemetry(#[from] nebula_metrics::MetricsError),
}

impl RefreshCoordConfig {
    /// Verify the per- interlocking invariants.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::*` whose variant names which invariant the
    /// configuration violates. Returns `ConfigError::Overflow` if any of
    /// the intermediate `Duration` arithmetic (e.g. `heartbeat_interval × 3`,
    /// `refresh_timeout + 2 × heartbeat_interval`) overflows `Duration::MAX`
    /// -- the canonical fix is to lower the offending knob.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.claim_ttl.is_zero() {
            return Err(ConfigError::ZeroDuration { field: "claim_ttl" });
        }
        if self.heartbeat_interval.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "heartbeat_interval",
            });
        }
        if self.refresh_timeout.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "refresh_timeout",
            });
        }
        if self.reclaim_sweep_interval.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "reclaim_sweep_interval",
            });
        }
        if self.sentinel_window.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "sentinel_window",
            });
        }
        if self.sentinel_threshold == 0 {
            return Err(ConfigError::ZeroSentinelThreshold);
        }

        // `Duration::checked_mul` and `checked_add` return `None` on
        // overflow rather than panicking -- surface that as a typed
        // `ConfigError::Overflow` so a user-supplied
        // `Duration::MAX / 2`-ish value doesn't blow up the config gate.
        let hb_x3 = self
            .heartbeat_interval
            .checked_mul(3)
            .ok_or(ConfigError::Overflow {
                field: "heartbeat_interval * 3",
                value: self.heartbeat_interval,
            })?;
        if hb_x3 > self.claim_ttl {
            return Err(ConfigError::HeartbeatTooSlow);
        }
        let hb_x2 = self
            .heartbeat_interval
            .checked_mul(2)
            .ok_or(ConfigError::Overflow {
                field: "heartbeat_interval * 2",
                value: self.heartbeat_interval,
            })?;
        let hold_budget = self
            .refresh_timeout
            .checked_add(hb_x2)
            .ok_or(ConfigError::Overflow {
                field: "refresh_timeout + heartbeat_interval * 2",
                value: self.refresh_timeout,
            })?;
        if hold_budget > self.claim_ttl {
            return Err(ConfigError::RefreshTimeoutTooLong);
        }
        if self.reclaim_sweep_interval > self.claim_ttl {
            return Err(ConfigError::ReclaimTooSlow);
        }
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Errors surfaced from refresh_coalesced
// ──────────────────────────────────────────────────────────────────────────

/// Failures returned by [`RefreshCoordinator::refresh_coalesced`].
///
/// `CoalescedByOtherReplica` is **success** for the caller (state was
/// already fresh after another replica's refresh; just re-read state). All
/// other variants are real errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshError {
    /// The caller's configured contention budget elapsed before an L2 claim
    /// could be acquired.
    ///
    /// This remains pre-provider and replay-safe. It is surfaced when the
    /// contender's claim keeps being heartbeat-extended while adaptive polls
    /// consume `refresh_timeout`.
    #[error("contention budget exhausted before claim acquisition")]
    ContentionExhausted,
    /// Another replica's refresh succeeded while we were waiting on L2;
    /// caller treats as success and re-reads state.
    #[error("refresh coalesced by another replica (success \u{2014} re-read state)")]
    CoalescedByOtherReplica,
    /// Storage repo error (e.g. DB connectivity loss).
    #[error("storage repo error: {0}")]
    Repo(#[from] RepoError),
    /// Background heartbeat ownership was lost before the provider boundary.
    ///
    /// The provider closure was never started. Once the sentinel transition
    /// confirms entry into the provider/persistence critical section,
    /// heartbeat loss can no longer cancel that section.
    #[error("L2 claim lost before provider dispatch \u{2014} refresh was not started")]
    ClaimLostBeforeProvider,
    /// The caller stopped waiting after the provider boundary, or the owned
    /// task terminated without returning an exact disposition.
    ///
    /// This is deliberately non-retryable at the resolver boundary. The owned
    /// task continues after an ordinary timeout. Panic/runtime cancellation
    /// retains the sentinel claim; once its lease expires, storage exposes it
    /// as durable fail-closed poison until explicit reconciliation.
    #[error("provider/persistence refresh outcome is pending or unknown; do not retry")]
    CriticalOutcomePending,
    /// Another in-process attempt reached an exact, replay-safe outcome but
    /// did not advance authoritative state.
    ///
    /// No provider or persistence operation remains pending, but the
    /// coordinator deliberately does not turn every waiter into an immediate
    /// retry. The caller may retry later under its normal backoff and circuit
    /// policy.
    #[error("a concurrent refresh attempt completed without advancing credential state")]
    PriorAttemptNoProgress,
    /// The authoritative credential state could not be rechecked after
    /// contention, so provider dispatch was denied.
    ///
    /// This failure occurs before the provider boundary and is therefore safe
    /// for the command owner to retry after the state source recovers.
    #[error(transparent)]
    StateRecheck(#[from] RefreshRecheckError),
}

/// Closed failure taxonomy for the pre-provider state recheck.
///
/// A recheck is mandatory after L1/L2 contention because the refresh closure
/// captures state loaded before waiting. Neither storage failure nor corrupt
/// state may be flattened into `true`: doing so would authorize provider
/// egress with a stale rotating grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshRecheckError {
    /// The authoritative state source could not be read.
    #[error("credential state recheck is unavailable")]
    Unavailable,
    /// The current persisted state could not be validated for the operation.
    #[error("credential state recheck found invalid state")]
    InvalidState,
}

/// Exact disposition of an owned provider/persistence refresh section.
///
/// The coordinator needs this distinction to finalize the durable L2 claim
/// safely and tell L1 waiters what the completion proves. A durable state
/// advance may release immediately and permits a later refresh epoch. An exact
/// replay-safe outcome without a state advance also releases, but waiters do
/// not automatically retry it. A post-provider finalization failure or unknown
/// provider/commit outcome stops heartbeats but deliberately leaves the
/// sentinel claim in place. Once its lease expires, storage keeps it as durable
/// fail-closed poison so provider work cannot replay before explicit
/// reconciliation.
#[derive(Debug)]
#[must_use = "the refresh disposition controls whether the durable claim may be released"]
#[non_exhaustive]
pub enum RefreshDisposition<T> {
    /// The operation durably advanced the authoritative state consulted by
    /// `needs_refresh_after_backoff`.
    ///
    /// A waiter still observing work after this completion is handling a later
    /// logical epoch and may enter a new winner election. Callers must use this
    /// variant only after an acknowledged state transition (including a
    /// durable `reauth_required` transition).
    StateAdvanced(T),
    /// The operation reached an exact, replay-safe outcome without advancing
    /// authoritative state.
    ///
    /// L2 is released, but L1 waiters receive
    /// [`RefreshError::PriorAttemptNoProgress`] instead of immediately
    /// replaying the operation as a herd.
    NoStateChange(T),
    /// The provider accepted the refresh, but its new state was definitely not
    /// persisted.
    ///
    /// The enclosed error is exact, yet another replica must not immediately
    /// re-POST the still-expired stored grant. Like an unknown acknowledgement,
    /// this retains the sentinel claim. Expiry converts it into durable poison;
    /// explicit reconcile authority is K3 work.
    RetryUnsafe(T),
    /// Provider dispatch or persistence commit completed without an exact
    /// acknowledgement.
    ///
    /// The enclosed value is still returned to the waiting caller (normally a
    /// typed `OutcomeUnknown` error), while the claim remains retained as
    /// durable poison after expiry.
    OutcomeUnknown(T),
}

impl<T> RefreshDisposition<T> {
    /// Construct a disposition backed by an acknowledged authoritative state
    /// transition.
    pub fn state_advanced(value: T) -> Self {
        Self::StateAdvanced(value)
    }

    /// Construct an exact, replay-safe disposition that did not change
    /// authoritative state.
    pub fn no_state_change(value: T) -> Self {
        Self::NoStateChange(value)
    }

    /// Construct a definite but unsafe-to-replay post-provider disposition.
    pub fn retry_unsafe(value: T) -> Self {
        Self::RetryUnsafe(value)
    }

    /// Construct an unknown provider-or-commit disposition.
    pub fn outcome_unknown(value: T) -> Self {
        Self::OutcomeUnknown(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaimFinalization {
    Release,
    RetainAsPoison,
}

struct L1RefreshLease {
    l1: Arc<L1RefreshCoalescer>,
    credential_id: Option<String>,
    completion: L1Completion,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl L1RefreshLease {
    fn new(l1: Arc<L1RefreshCoalescer>, credential_id: String) -> Self {
        Self {
            l1,
            credential_id: Some(credential_id),
            completion: L1Completion::NoStateChange,
            _permit: None,
        }
    }

    fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self._permit = Some(permit);
    }

    fn set_completion(&mut self, completion: L1Completion) {
        self.completion = completion;
    }
}

impl Drop for L1RefreshLease {
    fn drop(&mut self) {
        if let Some(credential_id) = self.credential_id.take() {
            self.l1.complete(&credential_id, self.completion);
        }
    }
}

/// Owned L2 lease transferred atomically into the provider/persistence task.
///
/// Before transfer, dropping the outer coordination future stops heartbeat and
/// best-effort releases the claim because no provider request has started.
/// After transfer, the detached task owns this guard, so caller cancellation or
/// timeout cannot release the claim before the critical section reports an
/// exact disposition.
struct RefreshLease {
    repo: Arc<dyn RefreshClaimRepo>,
    token: Option<ClaimToken>,
    heartbeat_stop: CancellationToken,
    heartbeat_task: Option<tokio::task::JoinHandle<()>>,
    metrics: RefreshCoordMetrics,
    hold_start: Instant,
    release_on_drop: bool,
    _l1: Option<L1RefreshLease>,
}

impl RefreshLease {
    fn new(
        repo: Arc<dyn RefreshClaimRepo>,
        token: ClaimToken,
        heartbeat_stop: CancellationToken,
        heartbeat_task: tokio::task::JoinHandle<()>,
        metrics: RefreshCoordMetrics,
        hold_start: Instant,
        l1: L1RefreshLease,
    ) -> Self {
        Self {
            repo,
            token: Some(token),
            heartbeat_stop,
            heartbeat_task: Some(heartbeat_task),
            metrics,
            hold_start,
            release_on_drop: true,
            _l1: Some(l1),
        }
    }

    fn enter_provider_critical_section(&mut self) {
        self.release_on_drop = false;
        if let Some(l1) = &mut self._l1 {
            // From the sentinel acknowledgement until an exact disposition,
            // any panic/runtime teardown must wake waiters fail-closed.
            l1.set_completion(L1Completion::ReplayUnsafe);
        }
    }

    async fn finish(mut self, finalization: ClaimFinalization, l1_completion: L1Completion) {
        if let Some(l1) = &mut self._l1 {
            l1.set_completion(l1_completion);
        }
        self.heartbeat_stop.cancel();
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.metrics
            .hold_duration
            .observe(self.hold_start.elapsed().as_secs_f64());

        // The provider/persistence section has an exact disposition. Wake L1
        // waiters and return the global permit *before* touching the L2 release
        // path: a wedged database/pool must not permanently poison the local
        // single-flight entry or consume one global refresh slot.
        drop(self._l1.take());

        let Some(token) = self.token.take() else {
            return;
        };
        if finalization == ClaimFinalization::Release {
            let repo = Arc::clone(&self.repo);
            // Release is best-effort and deliberately detached. The L2 row
            // continues to coalesce other replicas until this completes. If
            // it remains through expiry, storage fails closed instead of
            // treating the stale sentinel as replay authorization, while the exact
            // provider/persistence result can return without a hung release
            // wedging local progress.
            tokio::spawn(async move {
                if let Err(error) = repo.release(token).await {
                    // A release failure never changes an already-confirmed
                    // outcome. Replaying provider work would be less safe than
                    // waiting for claim expiry.
                    tracing::warn!(
                        ?error,
                        "L2 claim release after exact refresh disposition failed"
                    );
                }
            });
        } else {
            tracing::warn!("refresh disposition forbids replay; retaining claim as durable poison");
        }
    }
}

impl Drop for RefreshLease {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.heartbeat_stop.cancel();
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
        self.metrics
            .hold_duration
            .observe(self.hold_start.elapsed().as_secs_f64());

        if !self.release_on_drop {
            // Panic/runtime cancellation after the sentinel boundary has no
            // trustworthy commit disposition. Releasing here would allow an
            // immediate blind replay, so retain the row exactly like an
            tracing::warn!(
                "provider/persistence task dropped without an exact disposition; \
                 retaining refresh claim as durable poison"
            );
            return;
        }

        let repo = Arc::clone(&self.repo);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(error) = repo.release(token).await {
                        tracing::warn!(
                            ?error,
                            "L2 claim release after pre-provider cancellation or task failure failed"
                        );
                    }
                });
            },
            Err(error) => {
                // There is no executor on which an async release can run. The
                // stopped heartbeat guarantees the row expires naturally.
                tracing::warn!(
                    ?error,
                    "no Tokio runtime available for L2 claim release; claim will expire by TTL"
                );
            },
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Coordinator
// ──────────────────────────────────────────────────────────────────────────

/// Two-tier credential refresh coordinator (L1 in-process + L2 cross-replica).
pub struct RefreshCoordinator {
    l1: Arc<L1RefreshCoalescer>,
    repo: Arc<dyn RefreshClaimRepo>,
    replica_id: ReplicaId,
    config: RefreshCoordConfig,
    metrics: RefreshCoordMetrics,
    audit_sink: Option<Arc<dyn AuditSink>>,
}

impl fmt::Debug for RefreshCoordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshCoordinator")
            .field("replica_id", &self.replica_id)
            .field("config", &self.config)
            .field("l1", &self.l1)
            .field("audit_sink_present", &self.audit_sink.is_some())
            .finish_non_exhaustive()
    }
}

impl RefreshCoordinator {
    /// Maximum number of consecutive non-`ClaimLost` heartbeat failures
    /// tolerated before the heartbeat task signals claim loss (sub-spec
    /// wave-4 fix).
    ///
    /// At three failures the worst-case latency before cancellation
    /// is `3 × heartbeat_interval`, which is bounded by the
    /// invariant `heartbeat_interval × 3 <= claim_ttl` -- i.e. we
    /// never burn more than one TTL window absorbing transient
    /// noise. Not configurable: production tuning belongs in
    /// `RefreshCoordConfig` if a need emerges.
    const MAX_TRANSIENT_HEARTBEAT_FAILURES: u32 = 3;

    /// Construct a coordinator wired to a given `RefreshClaimRepo`.
    ///
    /// Metrics are bound to a fresh in-memory registry by default -- call
    /// [`Self::with_metrics`] post-construction to thread the engine-shared
    /// `MetricsRegistry`. Audit events are not emitted unless
    /// [`Self::with_audit_sink`] is called.
    ///
    /// # Errors
    ///
    /// Returns the corresponding [`ConfigError`] if `config.validate()`
    /// fails (see invariants) or metric handles cannot be bound.
    pub fn new_with(
        repo: Arc<dyn RefreshClaimRepo>,
        replica_id: ReplicaId,
        config: RefreshCoordConfig,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        // Bootstrap: a fresh private registry so the coordinator is fully
        // functional without composition. Production callers MUST follow
        // up with `with_metrics(engine_registry)` so a scraper actually
        // observes the series -- see `with_metrics` rustdoc.
        let metrics = RefreshCoordMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())?;
        Ok(Self {
            l1: Arc::new(L1RefreshCoalescer::new()),
            repo,
            replica_id,
            config,
            metrics,
            audit_sink: None,
        })
    }

    /// Replace the metric handles with ones bound to the engine-shared
    /// `MetricsRegistry`. Call once during composition; the coordinator
    /// emits all sub-spec series against this registry afterwards.
    #[must_use = "builder methods must be chained or used"]
    pub fn with_metrics(mut self, metrics: RefreshCoordMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Attach an [`AuditSink`] to receive refresh-coordination observations
    /// (`RefreshCoordClaimAcquired`, `RefreshCoordSentinelTriggered`, and
    /// `RefreshCoordReauthThresholdReached`).
    ///
    /// These events are non-authoritative: the sentinel threshold path
    /// publishes a lossy observation and does not itself durably set the
    /// credential reauth bit. That durable consumer/command seam is K3 work.
    /// Without a sink, audit emission is a no-op (the metric / tracing surfaces
    /// still observe).
    #[must_use = "builder methods must be chained or used"]
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    /// Borrow the pre-bound metric handles. Used by reclaim-sweep
    /// wiring so the sweep emits the same series.
    #[must_use]
    pub(crate) fn metrics(&self) -> &RefreshCoordMetrics {
        &self.metrics
    }

    /// Borrow the audit sink (`None` if not configured). Used by the
    /// reclaim sweep to emit sentinel/threshold observations. The
    /// `RefreshCoordReauthThresholdReached` is not proof of a durable
    /// credential-state transition.
    #[must_use]
    pub(crate) fn audit_sink(&self) -> Option<&Arc<dyn AuditSink>> {
        self.audit_sink.as_ref()
    }

    /// Borrow the validated config this coordinator was constructed
    /// with.
    #[must_use]
    pub(crate) fn config(&self) -> &RefreshCoordConfig {
        &self.config
    }

    /// Borrow the underlying claim repo for maintenance wiring such as the
    /// reclaim sweep (Stage 3.3).
    ///
    /// Normal provider work must enter through [`Self::refresh_coalesced`],
    /// which owns sentinel marking; callers must not reproduce that boundary.
    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }

    /// Acquire L1 mutex + L2 claim, run the refresh closure, release
    /// both. Returns `Err(CoalescedByOtherReplica)` if state was already
    /// fresh -- caller treats as success and re-reads.
    ///
    /// Sub-spec acquisition sequence:
    /// 1. L1 in-process coalesce (cheap fast-path; same-process concurrent calls collapse here).
    /// 2. L2 durable claim with backoff.
    /// 3. Background heartbeat task -- passes `self.config.claim_ttl` to each `repo.heartbeat(token,
    ///    ttl)` call (Stage 1 fix C2).
    /// 4. Confirm the sentinel transition that marks the irreversible provider boundary.
    /// 5. Transfer the heartbeat and claim into an owned provider/persistence task.
    /// 6. Release after `StateAdvanced`/`NoStateChange`, or retain as durable poison after
    ///    `RetryUnsafe`/`OutcomeUnknown`.
    ///
    /// The provider closure receives no claim or token. Durable claim authority
    /// is coordinator-private and cannot be released, heartbeated, or reused by
    /// integration code.
    ///
    /// `needs_refresh_after_backoff` is consulted after L1 completion and by
    /// the L2 backoff loop after a post-`Contended` sleep. `Ok(false)` means
    /// authoritative state changed or no longer needs this operation, so the
    /// caller re-reads it through
    /// [`RefreshError::CoalescedByOtherReplica`]. `Ok(true)` authorizes another
    /// claim attempt. `Err` denies provider dispatch with a typed,
    /// pre-provider [`RefreshError::StateRecheck`].
    ///
    /// Callers without an external state source may pass
    /// `|_| async { Ok(true) }`. Persistence-backed callers must perform a
    /// real version/state recheck; an unconditional predicate is not a safe
    /// substitute after contention.
    ///
    /// # Errors
    ///
    /// See [`RefreshError`]. `CoalescedByOtherReplica` is success-with-side-effect:
    /// another replica refreshed while we were waiting. Caller should
    /// re-read the credential state and proceed.
    ///
    /// # Cancel-safety
    ///
    /// The sentinel acknowledgement is the explicit point of no cancellation.
    /// Before it, caller cancellation or heartbeat loss releases the claim and
    /// the provider closure is never started. Immediately after it, the closure
    /// and the internal `RefreshLease` move into an owned Tokio task with no intervening
    /// await. Dropping this method's future, an outer timeout, or heartbeat loss
    /// after that boundary cannot cancel provider work, persistence commit, or
    /// release L2 early.
    ///
    /// `refresh_timeout` bounds each L1-wait, L2-contention, and owned-task
    /// wait phase; it does not abort an already-started critical section. After
    /// a critical-task timeout, that section remains protected by heartbeat
    /// and L2 until its exact disposition. A state-advanced or exact
    /// no-state-change outcome first wakes L1/returns the global permit, then
    /// dispatches a best-effort L2 release; the L2 row continues coalescing
    /// until that release completes. If an exact finalization's release is
    /// delayed beyond TTL, a matching token may still clear that row; no other
    /// holder may acquire it in the interim. An
    /// [`RefreshDisposition::OutcomeUnknown`] stops heartbeat and deliberately
    /// leaves the claim row in place. After TTL, the repository returns
    /// [`ClaimAttempt::OutcomeUnknown`] for that row rather than authorizing a
    /// blind replay of a commit whose acknowledgement was lost.
    ///
    /// There is intentionally no cancelling deadline on the owned critical
    /// task: after provider dispatch, cancellation cannot establish that the
    /// grant was not consumed. A genuinely non-terminating integration keeps
    /// its heartbeat and claim fail-closed until the process stops or an
    /// operator reconciles it; expiring that live lease and permitting another
    /// provider call would trade an operational stall for credential
    /// corruption. Provider transports should still use their own
    /// protocol-aware deadlines and return an exact or unknown disposition.
    #[tracing::instrument(
        name = "credential.refresh.coordinate",
        skip(self, needs_refresh_after_backoff, do_refresh),
        fields(
            credential_id = %credential_id,
            replica_id = %self.replica_id,
            tier = tracing::field::Empty,
        ),
    )]
    pub async fn refresh_coalesced<F, Fut, T, P, PFut>(
        &self,
        credential_id: &CredentialId,
        needs_refresh_after_backoff: P,
        do_refresh: F,
    ) -> Result<T, RefreshError>
    where
        // Explicit `Send` bounds (review I2): `do_refresh` moves into an
        // owned task and the predicate is awaited from the backoff loop.
        // Without these bounds a `!Send` body
        // (e.g. one that captures an `Rc<...>`) compiles cleanly here
        // and surfaces an obscure auto-trait error at the call site.
        // Locking the contract on the trait bound moves the diagnostic
        // back to the user closure.
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = RefreshDisposition<T>> + Send + 'static,
        T: Send + 'static,
        P: Fn(&CredentialId) -> PFut + Sync,
        PFut: Future<Output = Result<bool, RefreshRecheckError>> + Send,
    {
        // L1: in-process coalescing.
        //
        // The L1 layer is keyed by string, so we hash on the typed id's
        // canonical form. `try_refresh` returns Winner for the first
        // caller and Waiter (with a oneshot::Receiver) for every other
        // concurrent caller in the same process. Waiters await the Winner's
        // typed, payload-free completion policy for at most `refresh_timeout`,
        // then always recheck authoritative state. A proven state advance
        // coalesces this epoch; if the predicate is still true after that
        // advance, it represents newer work and the waiter re-enters election.
        // Exact no-progress and replay-unsafe outcomes remain distinct, so a
        // provider failure cannot turn a waiting herd into automatic retries.
        // Timeout or abnormal sender closure is `CriticalOutcomePending`.
        let cred_str = credential_id.to_string();
        loop {
            match self.l1.try_refresh(&cred_str) {
                super::l1::RefreshAttempt::Winner => {
                    // NOTE: do NOT record `tier="l2"` here -- the L2 path can
                    // still produce `CoalescedByOtherReplica` via the
                    // post-backoff recheck in
                    // `try_acquire_l2_with_backoff`. Recording the tier
                    // prematurely makes operators see "l2 acquired" when the
                    // actual outcome was "l2 coalesced" (review I1).
                    // The closed set
                    // `{l1, l1_no_progress, l1_outcome_unknown, l2_acquired,
                    // l2_coalesced, l2_outcome_unknown}` is recorded at the
                    // actual outcome sites below.
                    break;
                },
                super::l1::RefreshAttempt::Waiter(rx) => {
                    let completion =
                        match tokio::time::timeout(self.config.refresh_timeout, rx).await {
                            Ok(Ok(completion)) => completion,
                            Ok(Err(error)) => {
                                self.l1.prune_closed_waiters(&cred_str);
                                tracing::Span::current().record("tier", "l1_outcome_unknown");
                                tracing::error!(
                                    event = "credential.refresh.l1.wait.outcome_unknown",
                                    reason = "sender_closed",
                                    ?error,
                                    credential_id = %credential_id,
                                    "L1 winner ended without an exact completion signal"
                                );
                                return Err(RefreshError::CriticalOutcomePending);
                            },
                            Err(error) => {
                                self.l1.prune_closed_waiters(&cred_str);
                                tracing::Span::current().record("tier", "l1_outcome_unknown");
                                tracing::warn!(
                                    event = "credential.refresh.l1.wait.outcome_unknown",
                                    reason = "timeout",
                                    timeout_ms = self.config.refresh_timeout.as_millis(),
                                    ?error,
                                    credential_id = %credential_id,
                                    "L1 waiter stopped waiting for an unresolved owned refresh"
                                );
                                return Err(RefreshError::CriticalOutcomePending);
                            },
                        };

                    // A typed completion signal still cannot replace the
                    // authoritative row. In particular, an unknown provider
                    // acknowledgement may have committed successfully, while
                    // a nominal state advance can be followed by a later
                    // refresh epoch before this waiter runs.
                    let still_needs_refresh = match tokio::time::timeout(
                        self.config.refresh_timeout,
                        needs_refresh_after_backoff(credential_id),
                    )
                    .await
                    {
                        Ok(Ok(still_needs_refresh)) => still_needs_refresh,
                        Ok(Err(error)) => {
                            tracing::Span::current().record("tier", "l1_outcome_unknown");
                            tracing::warn!(
                                event = "credential.refresh.l1.wait.recheck_failed",
                                reason = %error,
                                credential_id = %credential_id,
                                "L1 completion could not be verified from authoritative state"
                            );
                            return Err(RefreshError::StateRecheck(error));
                        },
                        Err(error) => {
                            tracing::Span::current().record("tier", "l1_outcome_unknown");
                            tracing::warn!(
                                event = "credential.refresh.l1.wait.outcome_unknown",
                                reason = "state_recheck_timeout",
                                timeout_ms = self.config.refresh_timeout.as_millis(),
                                ?error,
                                credential_id = %credential_id,
                                "L1 completion state recheck did not finish"
                            );
                            return Err(RefreshError::CriticalOutcomePending);
                        },
                    };

                    if !still_needs_refresh {
                        tracing::Span::current().record("tier", "l1");
                        self.metrics.coalesced_l1.inc();
                        return Err(RefreshError::CoalescedByOtherReplica);
                    }

                    match completion {
                        L1Completion::StateAdvanced => {
                            // The caller contract promises that this signal
                            // follows an acknowledged authoritative transition.
                            // A still-true fresh predicate therefore denotes a
                            // later logical epoch. Re-entering election admits
                            // exactly one local winner for that newer work.
                            tracing::debug!(
                                event = "credential.refresh.l1.wait.new_epoch",
                                credential_id = %credential_id,
                                "authoritative state requires a newer refresh epoch after \
                                 confirmed L1 progress"
                            );
                        },
                        L1Completion::NoStateChange => {
                            tracing::Span::current().record("tier", "l1_no_progress");
                            tracing::debug!(
                                event = "credential.refresh.l1.wait.no_progress",
                                credential_id = %credential_id,
                                "exact L1 winner made no authoritative progress; automatic \
                                 waiter replay denied"
                            );
                            return Err(RefreshError::PriorAttemptNoProgress);
                        },
                        L1Completion::ReplayUnsafe => {
                            tracing::Span::current().record("tier", "l1_outcome_unknown");
                            tracing::warn!(
                                event = "credential.refresh.l1.wait.outcome_unknown",
                                reason = "authoritative_state_unchanged_after_unsafe_completion",
                                credential_id = %credential_id,
                                "L1 winner outcome cannot be replayed safely"
                            );
                            return Err(RefreshError::CriticalOutcomePending);
                        },
                    }
                },
            }
        }

        // The L1 completion and global permit are owned together. Before the
        // provider boundary this local guard completes on every early return.
        // At the boundary it moves into `RefreshLease`, so caller
        // timeout/cancellation cannot wake local waiters while the detached
        // provider/persistence section is still running.
        let mut l1_lease = L1RefreshLease::new(Arc::clone(&self.l1), cred_str);

        // Global rate-limit gate (audit B6 / wave-2 regression).
        //
        // Wave-2 introduced this typed entry point but silently bypassed
        // the L1 global concurrency semaphore (`refresh_semaphore`,
        // default 32 permits). Per-credential L1 coalescing alone does
        // not bound the case where many *distinct* credentials expire
        // near-simultaneously -- e.g. on a daily TTL boundary or after
        // a replica restart with stale tokens -- and a 200-credential
        // expiry burst would issue 200 concurrent IdP POSTs, recreating
        // the cascading-429 / refresh-storm pattern the cap is meant to
        // prevent. Only the legacy `String`-id path
        // (`resolver.rs::refresh_via_l1_only`) consumed permits, so
        // typed callers were unprotected.
        //
        // Acquired AFTER `try_refresh` (Winner-only -- Waiters already
        // park on the oneshot above and do not need a permit) and BEFORE
        // L2 backoff so the bound covers the entire IdP POST window.
        // `l1_lease` was constructed first, so it completes on every
        // cancel/Drop path even if `acquire_permit` itself is cancelled
        // (its `await` is cancel-safe per
        // `L1RefreshCoalescer::acquire_permit` rustdoc -- dropping the
        // future does not consume a permit).
        //
        // RAII: attaching the permit to `l1_lease` keeps the global cap
        // occupied for the owned critical task as well as the outer wait.
        let permit = self.l1.acquire_permit().await;
        l1_lease.attach_permit(permit);

        // L2: durable claim with backoff.
        let claim = self
            .try_acquire_l2_with_backoff(credential_id, &needs_refresh_after_backoff)
            .await?;

        // Sub-spec -- record the claim acquisition once we know we own
        // the L2 row. `acquired` counter, audit event, and start of the
        // hold-duration measurement happen here so they are paired
        // with the matching `release` site below.
        //
        // Span tier (review I1) -- record `l2_acquired` at the outcome
        // site so operators distinguish from the `l2_coalesced` path
        // (post-backoff recheck), which is recorded inside
        // `try_acquire_l2_with_backoff` below.
        tracing::Span::current().record("tier", "l2_acquired");
        self.metrics.claims_acquired.inc();
        emit_claim_acquired(
            self.audit_sink.as_deref(),
            credential_id,
            self.replica_id.as_str(),
            self.config.claim_ttl.as_secs(),
        );
        let hold_start = Instant::now();

        // Heartbeat has two independent signals:
        //
        // - `heartbeat_stop` belongs to the lease owner and terminates the task
        //   only after an exact critical-section disposition;
        // - `claim_lost` is emitted by heartbeat failures. It may prevent entry
        //   before the provider boundary, but cannot cancel work afterwards.
        let heartbeat_stop = CancellationToken::new();
        let claim_lost = CancellationToken::new();
        let heartbeat_task = self.spawn_heartbeat(
            claim.token.clone(),
            heartbeat_stop.clone(),
            claim_lost.clone(),
            *credential_id,
        );
        let mut lease = RefreshLease::new(
            Arc::clone(&self.repo),
            claim.token.clone(),
            heartbeat_stop,
            heartbeat_task,
            self.metrics.clone(),
            hold_start,
            l1_lease,
        );

        // This durable sentinel acknowledgement is the point of no
        // cancellation. Bias toward a claim-loss signal if both branches are
        // ready: in that case the provider closure has not started, so stopping
        // is the only safe outcome. Dropping the outer future while this await
        // is pending drops `lease`, which releases L2 and still starts no
        // provider work.
        let sentinel_result = tokio::select! {
            biased;
            () = claim_lost.cancelled() => Err(RefreshError::ClaimLostBeforeProvider),
            result = self.repo.mark_sentinel(&claim.token) => result.map_err(RefreshError::Repo),
        };
        if let Err(error) = sentinel_result {
            lease
                .finish(ClaimFinalization::Release, L1Completion::NoStateChange)
                .await;
            return Err(error);
        }

        // No await may appear between the confirmed sentinel and this spawn.
        // Moving both closure and lease into the task is the atomic ownership
        // transfer that makes caller Drop/timeout harmless to the irreversible
        // provider -> persistence section.
        lease.enter_provider_critical_section();
        let mut critical_task = tokio::spawn(async move {
            let disposition = do_refresh().await;
            let (finalization, l1_completion, result) = match disposition {
                RefreshDisposition::StateAdvanced(result) => (
                    ClaimFinalization::Release,
                    L1Completion::StateAdvanced,
                    result,
                ),
                RefreshDisposition::NoStateChange(result) => (
                    ClaimFinalization::Release,
                    L1Completion::NoStateChange,
                    result,
                ),
                RefreshDisposition::RetryUnsafe(result) => (
                    ClaimFinalization::RetainAsPoison,
                    L1Completion::ReplayUnsafe,
                    result,
                ),
                RefreshDisposition::OutcomeUnknown(result) => (
                    ClaimFinalization::RetainAsPoison,
                    L1Completion::ReplayUnsafe,
                    result,
                ),
            };
            lease.finish(finalization, l1_completion).await;
            result
        });

        // The timeout controls caller latency only. Dropping a Tokio
        // `JoinHandle` detaches rather than aborts, so both this timeout path and
        // arbitrary cancellation of the outer future leave the owned task
        // running with its heartbeat and L2 lease.
        let timeout = self.config.refresh_timeout;
        match tokio::time::timeout(timeout, &mut critical_task).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) | Err(_) => Err(RefreshError::CriticalOutcomePending),
        }
    }

    /// L2 acquisition retry loop per sub-spec.
    ///
    /// On `Contended` we use an adaptive 25 → 50 → 100 → 200 ms poll cadence
    /// (plus bounded jitter), capped by both the observed claim expiry and this
    /// call's `refresh_timeout` budget, then consult
    /// `needs_refresh_after_backoff(credential_id)`. If the predicate
    /// returns `false` we surface
    /// [`RefreshError::CoalescedByOtherReplica`] -- another replica
    /// completed the refresh while we were waiting, and the caller
    /// should re-read state from storage. Otherwise we retry
    /// `try_claim` until we win the claim or exhaust the contention budget.
    async fn try_acquire_l2_with_backoff<P, PFut>(
        &self,
        credential_id: &CredentialId,
        needs_refresh_after_backoff: &P,
    ) -> Result<RefreshClaim, RefreshError>
    where
        // Mirror the `Send`/`Sync` bounds on `refresh_coalesced` so the
        // helper's auto-trait inference does not silently relax the
        // public contract.
        P: Fn(&CredentialId) -> PFut + Sync,
        PFut: Future<Output = Result<bool, RefreshRecheckError>> + Send,
    {
        const POLL_CADENCE: [Duration; 4] = [
            Duration::from_millis(25),
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_millis(200),
        ];
        const MAX_JITTER_MS: u64 = 10;

        let contention_deadline = tokio::time::Instant::now() + self.config.refresh_timeout;
        let mut attempt = 0usize;
        loop {
            // Sub-spec per-attempt tracing span: `attempt` and
            // `credential_id` so operators correlate contention storms
            // across replicas.
            let span = tracing::info_span!(
                "credential.refresh.claim.acquire",
                credential_id = %credential_id,
                replica_id = %self.replica_id,
                attempt = attempt,
            );
            let outcome = async {
                self.repo
                    .try_claim(credential_id, &self.replica_id, self.config.claim_ttl)
                    .await
            }
            .instrument(span)
            .await?;
            match outcome {
                ClaimAttempt::Acquired(claim) => return Ok(claim),
                ClaimAttempt::OutcomeUnknown { expired_at } => {
                    tracing::Span::current().record("tier", "l2_outcome_unknown");
                    self.metrics.claims_outcome_unknown.inc();
                    tracing::error!(
                        event = "credential.refresh.claim.outcome_unknown",
                        claim_outcome = "outcome_unknown",
                        credential_id = %credential_id,
                        replica_id = %self.replica_id,
                        %expired_at,
                        "expired RefreshInFlight claim is durable outcome-unknown poison; \
                         provider dispatch denied pending explicit reconciliation"
                    );
                    // The retained periodic ReclaimSweepHandle is the sole
                    // owner of evidence accounting and threshold observation.
                    // Request-path one-shots must not consume an idempotent
                    // accounting row without the configured event bus.
                    return Err(RefreshError::CriticalOutcomePending);
                },
                ClaimAttempt::Contended {
                    existing_expires_at,
                } => {
                    // Sub-spec -- bump the contended counter for every
                    // try_claim that returned Contended, regardless of
                    // whether the post-backoff recheck eventually
                    // short-circuits.
                    self.metrics.claims_contended.inc();
                    // Poll well before the full claim TTL. A healthy winner
                    // usually releases in milliseconds; sleeping until its
                    // advertised expiry made same-process waiters time out
                    // behind a claim that was already gone. The cadence backs
                    // off to cap database pressure for genuinely long-running
                    // owners, while the caller budget prevents unbounded
                    // pre-provider latency.
                    let remaining_budget =
                        contention_deadline.saturating_duration_since(tokio::time::Instant::now());
                    let until_expiry = (existing_expires_at - chrono::Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO);
                    let cadence = POLL_CADENCE
                        .get(attempt.min(POLL_CADENCE.len() - 1))
                        .copied()
                        .unwrap_or(Duration::from_millis(200));
                    let poll_delay = if until_expiry.is_zero() {
                        POLL_CADENCE[0]
                    } else {
                        cadence.min(until_expiry)
                    }
                    .min(remaining_budget);
                    let jitter =
                        jitter_ms(MAX_JITTER_MS).min(remaining_budget.saturating_sub(poll_delay));
                    tokio::time::sleep(poll_delay + jitter).await;
                    // CRITICAL: post-backoff state recheck per sub-spec. If
                    // the contender finished the refresh while we slept,
                    // the credential is now fresh -- short-circuit with
                    // CoalescedByOtherReplica so the caller re-reads
                    // state instead of running another IdP POST. Without
                    // this check, two replicas racing through L2 each
                    // run the closure (one wins try_claim now that the
                    // contender's row is gone), invalidating any
                    // refresh_token rotation the contender just
                    // committed (n8n #13088 lineage).
                    match needs_refresh_after_backoff(credential_id).await {
                        Ok(true) => {
                            if tokio::time::Instant::now() >= contention_deadline {
                                break;
                            }
                            attempt = attempt.saturating_add(1);
                        },
                        Ok(false) => {
                            // Sub-spec -- L2 coalesce: another replica
                            // refreshed while we waited.
                            //
                            // Span tier (review I1) -- record `l2_coalesced`
                            // at the outcome site. We are now outside the
                            // per-attempt `instrument(span)` block (which
                            // wrapped only the `try_claim` future), so
                            // `Span::current()` resolves to the parent
                            // `credential.refresh.coordinate` span -- the
                            // intended target. The closed set
                            // `{l1, l1_no_progress, l1_outcome_unknown,
                            // l2_acquired, l2_coalesced, l2_outcome_unknown}` is
                            // documented in OBSERVABILITY.md.
                            tracing::Span::current().record("tier", "l2_coalesced");
                            self.metrics.coalesced_l2.inc();
                            return Err(RefreshError::CoalescedByOtherReplica);
                        },
                        Err(error) => {
                            tracing::warn!(
                                event = "credential.refresh.l2.recheck_failed",
                                reason = %error,
                                credential_id = %credential_id,
                                replica_id = %self.replica_id,
                                "post-contention state could not be verified; provider dispatch denied"
                            );
                            return Err(RefreshError::StateRecheck(error));
                        },
                    }
                },
            }
        }
        // Sub-spec -- the time budget elapsed without acquiring the L2 row.
        // `claims_total{outcome=exhausted} > 0` is a real production signal
        // worth alerting on.
        self.metrics.claims_exhausted.inc();
        Err(RefreshError::ContentionExhausted)
    }

    /// Spawn the background heartbeat task that refreshes the L2 claim
    /// TTL on a fixed interval. Per Stage 1 fix C2 the trait's
    /// `heartbeat(token, ttl)` takes the same TTL passed to
    /// `try_claim`, so the invariants
    /// (`heartbeat_interval × 3 < claim_ttl`,
    /// `reclaim_sweep_interval <= claim_ttl`) hold across heartbeats.
    ///
    /// Exits and signals claim loss via the supplied `claim_lost`
    /// [`CancellationToken`] in two cases:
    ///
    /// 1. **Claim lost** (`HeartbeatError::ClaimLost`): a different replica reclaimed the row
    ///    (generation bumped or row deleted). Before the sentinel boundary this prevents provider
    ///    dispatch. After that boundary it is observation-only: the owned provider/persistence task
    ///    must run to an exact disposition.
    ///
    /// 2. **Transient errors past budget**: any non-`ClaimLost` heartbeat error (e.g. transient
    ///    backend hiccup wrapped in `HeartbeatError::Repo`) retries up to
    ///    [`Self::MAX_TRANSIENT_HEARTBEAT_FAILURES`] times. Single transient hiccups are absorbed
    ///    silently so storage backpressure does not amplify into refresh storms. After the budget
    ///    is exhausted, cancellation fires.
    fn spawn_heartbeat(
        &self,
        token: ClaimToken,
        heartbeat_stop: CancellationToken,
        claim_lost: CancellationToken,
        credential_id: CredentialId,
    ) -> tokio::task::JoinHandle<()> {
        let repo = Arc::clone(&self.repo);
        let interval = self.config.heartbeat_interval;
        let ttl = self.config.claim_ttl;
        let replica_id = self.replica_id.as_str().to_string();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Avoid heartbeat amplification under storage backpressure:
            // if a heartbeat call exceeds `interval`, drop missed ticks
            // rather than firing them back-to-back when the call returns.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Burn the initial immediate tick -- the claim was just
            // acquired and already has a fresh `expires_at`.
            ticker.tick().await;
            // Transient-failure budget per sub-spec (wave-4 fix).
            // Resets on every successful heartbeat so a long-running
            // refresh can absorb intermittent backend noise without
            // cancelling. Only `HeartbeatError::ClaimLost` is treated
            // as immediate-cancel -- that is the unambiguous "your
            // claim is gone" signal.
            let mut transient_failures: u32 = 0;
            loop {
                tokio::select! {
                    biased;
                    () = heartbeat_stop.cancelled() => {
                        // The lease owner reached an exact disposition (or its
                        // guard is tearing down) -- heartbeat exits cleanly.
                        break;
                    }
                    _ = ticker.tick() => {
                        match repo.heartbeat(&token, ttl).await {
                            Ok(()) => {
                                // Reset the transient-failure budget on
                                // every success so a long refresh can
                                // absorb intermittent noise.
                                transient_failures = 0;
                            }
                            Err(HeartbeatError::ClaimLost) => {
                                // ERROR-level: claim loss is the
                                // unambiguous "another replica reclaimed
                                // the row" signal. Promoting from WARN
                                // keeps it distinguishable from
                                // transient retry noise on dashboards
                                // filtering on level.
                                tracing::error!(
                                    %credential_id,
                                    replica_id = %replica_id,
                                    "credential refresh heartbeat lost claim; signaling coordinator"
                                );
                                // The coordinator consumes this signal only
                                // before the sentinel boundary. Once the owned
                                // task starts, loss cannot cancel the
                                // provider/persistence critical section.
                                claim_lost.cancel();
                                break;
                            }
                            Err(HeartbeatError::Repo(repo_err)) => {
                                // Variant-explicit on purpose:
                                // `HeartbeatError` is NOT `#[non_exhaustive]`,
                                // so a wildcard `Err(other)` would silently
                                // bucket any future variant (e.g.
                                // `Unauthorized`, `Throttled`) as transient.
                                // Matching `Repo(_)` explicitly forces a
                                // compiler error when a new variant is
                                // added so the next maintainer makes a
                                // per-variant policy decision rather than
                                // inheriting "treat as transient" by accident.
                                transient_failures += 1;
                                if transient_failures >= Self::MAX_TRANSIENT_HEARTBEAT_FAILURES {
                                    tracing::error!(
                                        error = ?repo_err,
                                        %credential_id,
                                        replica_id = %replica_id,
                                        attempts = transient_failures,
                                        max_attempts = Self::MAX_TRANSIENT_HEARTBEAT_FAILURES,
                                        "credential refresh heartbeat exceeded transient-failure \
                                         budget; signaling coordinator"
                                    );
                                    claim_lost.cancel();
                                    break;
                                }
                                // Log at WARN -- single hiccups are
                                // absorbed silently from a level-filter
                                // perspective. Operators can still see
                                // them on noisy-log dashboards.
                                tracing::warn!(
                                    error = ?repo_err,
                                    %credential_id,
                                    replica_id = %replica_id,
                                    attempt = transient_failures,
                                    max_attempts = Self::MAX_TRANSIENT_HEARTBEAT_FAILURES,
                                    "credential refresh heartbeat transient error; retrying \
                                     within budget"
                                );
                                // Continue -- next ticker tick will retry.
                            }
                        }
                    }
                }
            }
        })
    }

    /// Record a refresh failure for the resolver-owned L1 circuit breaker.
    pub(crate) fn record_failure(&self, credential_id: &str) {
        self.l1.record_failure(credential_id);
    }

    /// Record a refresh success for the resolver-owned L1 circuit breaker.
    pub(crate) fn record_success(&self, credential_id: &str) {
        self.l1.record_success(credential_id);
    }

    /// Report whether the resolver-owned per-credential circuit is open.
    pub(crate) fn is_circuit_open(&self, credential_id: &str) -> bool {
        self.l1.is_circuit_open(credential_id)
    }
}

// `Default` deliberately not implemented: the only constructor without an
// explicit repo arg would need `InMemoryRefreshClaimRepo` from `nebula-storage`,
// which is outside this crate's dep graph. Callers construct via
// `RefreshCoordinator::new_with(repo, replica_id, config)`.

// ──────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────

fn jitter_ms(max_ms: u64) -> Duration {
    if max_ms == 0 {
        return Duration::ZERO;
    }
    let amount = rand::random_range(0..max_ms);
    Duration::from_millis(amount)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

    use chrono::Utc;
    use nebula_storage_port::store::ExpiredClaim;
    use tokio::sync::Notify;

    use super::*;

    const HEARTBEAT_OK: u8 = 0;
    const HEARTBEAT_LOST: u8 = 1;

    struct ScriptedClaimRepo {
        active: AtomicBool,
        try_claim_count: AtomicUsize,
        release_count: AtomicUsize,
        heartbeat_count: AtomicUsize,
        heartbeat_mode: AtomicU8,
        block_sentinel: AtomicBool,
        block_release: AtomicBool,
        try_claim_entered: Notify,
        sentinel_entered: Notify,
        sentinel_continue: Notify,
        release_entered: Notify,
        release_continue: Notify,
        release_completed: Notify,
    }

    struct PoisonClaimRepo {
        credential_id: CredentialId,
        try_claim_count: AtomicUsize,
        release_count: AtomicUsize,
        reclaim_count: AtomicUsize,
        evidence_count: AtomicUsize,
        evidence_recorded: AtomicBool,
    }

    impl PoisonClaimRepo {
        fn new(credential_id: CredentialId) -> Self {
            Self {
                credential_id,
                try_claim_count: AtomicUsize::new(0),
                release_count: AtomicUsize::new(0),
                reclaim_count: AtomicUsize::new(0),
                evidence_count: AtomicUsize::new(0),
                evidence_recorded: AtomicBool::new(false),
            }
        }
    }

    impl ScriptedClaimRepo {
        fn new() -> Self {
            Self {
                active: AtomicBool::new(false),
                try_claim_count: AtomicUsize::new(0),
                release_count: AtomicUsize::new(0),
                heartbeat_count: AtomicUsize::new(0),
                heartbeat_mode: AtomicU8::new(HEARTBEAT_OK),
                block_sentinel: AtomicBool::new(false),
                block_release: AtomicBool::new(false),
                try_claim_entered: Notify::new(),
                sentinel_entered: Notify::new(),
                sentinel_continue: Notify::new(),
                release_entered: Notify::new(),
                release_continue: Notify::new(),
                release_completed: Notify::new(),
            }
        }

        async fn wait_for_release(&self) {
            if self.release_count.load(Ordering::SeqCst) == 0 {
                self.release_completed.notified().await;
            }
        }

        async fn wait_for_try_claim_count(&self, target: usize) {
            while self.try_claim_count.load(Ordering::SeqCst) < target {
                self.try_claim_entered.notified().await;
            }
        }
    }

    #[async_trait::async_trait]
    impl RefreshClaimRepo for ScriptedClaimRepo {
        async fn try_claim(
            &self,
            credential_id: &CredentialId,
            _holder: &ReplicaId,
            ttl: Duration,
        ) -> Result<ClaimAttempt, RepoError> {
            self.try_claim_count.fetch_add(1, Ordering::SeqCst);
            self.try_claim_entered.notify_one();
            let now = Utc::now();
            let ttl = chrono::Duration::from_std(ttl).map_err(|_| RepoError::InvalidState)?;
            if self.active.swap(true, Ordering::SeqCst) {
                return Ok(ClaimAttempt::Contended {
                    existing_expires_at: now + ttl,
                });
            }
            Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: *credential_id,
                token: ClaimToken {
                    claim_id: "00000000-0000-0000-0000-000000000001"
                        .parse()
                        .expect("test claim id is a UUID"),
                    generation: 1,
                },
                acquired_at: now,
                expires_at: now + ttl,
            }))
        }

        async fn heartbeat(
            &self,
            _token: &ClaimToken,
            _ttl: Duration,
        ) -> Result<(), HeartbeatError> {
            self.heartbeat_count.fetch_add(1, Ordering::SeqCst);
            if self.heartbeat_mode.load(Ordering::SeqCst) == HEARTBEAT_LOST {
                Err(HeartbeatError::ClaimLost)
            } else {
                Ok(())
            }
        }

        async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
            self.release_entered.notify_one();
            if self.block_release.load(Ordering::SeqCst) {
                self.release_continue.notified().await;
            }
            self.active.store(false, Ordering::SeqCst);
            self.release_count.fetch_add(1, Ordering::SeqCst);
            self.release_completed.notify_one();
            Ok(())
        }

        async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RepoError> {
            self.sentinel_entered.notify_one();
            if self.block_sentinel.load(Ordering::SeqCst) {
                self.sentinel_continue.notified().await;
            }
            Ok(())
        }

        async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
            Ok(Vec::new())
        }

        async fn count_sentinel_events_in_window(
            &self,
            _credential_id: &CredentialId,
            _window: Duration,
        ) -> Result<u32, RepoError> {
            Ok(0)
        }
    }

    #[async_trait::async_trait]
    impl RefreshClaimRepo for PoisonClaimRepo {
        async fn try_claim(
            &self,
            credential_id: &CredentialId,
            _holder: &ReplicaId,
            _ttl: Duration,
        ) -> Result<ClaimAttempt, RepoError> {
            assert_eq!(*credential_id, self.credential_id);
            self.try_claim_count.fetch_add(1, Ordering::SeqCst);
            Ok(ClaimAttempt::OutcomeUnknown {
                expired_at: Utc::now() - chrono::Duration::seconds(1),
            })
        }

        async fn heartbeat(
            &self,
            _token: &ClaimToken,
            _ttl: Duration,
        ) -> Result<(), HeartbeatError> {
            Err(HeartbeatError::ClaimLost)
        }

        async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
            self.release_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RepoError> {
            Err(RepoError::InvalidState)
        }

        async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
            self.reclaim_count.fetch_add(1, Ordering::SeqCst);
            let newly_accounted = !self.evidence_recorded.swap(true, Ordering::SeqCst);
            let result = if newly_accounted {
                self.evidence_count.fetch_add(1, Ordering::SeqCst);
                vec![ExpiredClaim::OutcomeUnknownAccounted {
                    credential_id: self.credential_id,
                    previous_holder: ReplicaId::new("crashed-provider-holder"),
                    previous_generation: 7,
                }]
            } else {
                Vec::new()
            };
            Ok(result)
        }

        async fn count_sentinel_events_in_window(
            &self,
            credential_id: &CredentialId,
            _window: Duration,
        ) -> Result<u32, RepoError> {
            assert_eq!(*credential_id, self.credential_id);
            Ok(u32::try_from(self.evidence_count.load(Ordering::SeqCst)).unwrap_or(u32::MAX))
        }
    }

    fn coordinator(
        repo: Arc<ScriptedClaimRepo>,
        config: RefreshCoordConfig,
    ) -> Arc<RefreshCoordinator> {
        Arc::new(
            RefreshCoordinator::new_with(repo, ReplicaId::new("test-replica"), config)
                .expect("test coordinator config is valid"),
        )
    }

    fn paused_config() -> RefreshCoordConfig {
        RefreshCoordConfig {
            claim_ttl: Duration::from_millis(60),
            heartbeat_interval: Duration::from_millis(10),
            refresh_timeout: Duration::from_millis(30),
            reclaim_sweep_interval: Duration::from_millis(60),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_mins(1),
        }
    }

    async fn advance_until_heartbeat(repo: &ScriptedClaimRepo, interval: Duration) {
        for _ in 0..3 {
            tokio::time::advance(interval).await;
            tokio::task::yield_now().await;
            if repo.heartbeat_count.load(Ordering::SeqCst) > 0 {
                return;
            }
        }
        panic!("heartbeat task did not reach the scripted repository");
    }

    async fn wait_until_l1_empty(coordinator: &RefreshCoordinator) {
        for _ in 0..8 {
            if coordinator.l1.in_flight_count() == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("L1 completion was not released after exact disposition");
    }

    #[test]
    fn zero_durations_are_rejected_before_provider_work() {
        for (field, config) in [
            (
                "claim_ttl",
                RefreshCoordConfig {
                    claim_ttl: Duration::ZERO,
                    ..RefreshCoordConfig::default()
                },
            ),
            (
                "heartbeat_interval",
                RefreshCoordConfig {
                    heartbeat_interval: Duration::ZERO,
                    ..RefreshCoordConfig::default()
                },
            ),
            (
                "refresh_timeout",
                RefreshCoordConfig {
                    refresh_timeout: Duration::ZERO,
                    ..RefreshCoordConfig::default()
                },
            ),
            (
                "reclaim_sweep_interval",
                RefreshCoordConfig {
                    reclaim_sweep_interval: Duration::ZERO,
                    ..RefreshCoordConfig::default()
                },
            ),
            (
                "sentinel_window",
                RefreshCoordConfig {
                    sentinel_window: Duration::ZERO,
                    ..RefreshCoordConfig::default()
                },
            ),
        ] {
            let error = RefreshCoordinator::new_with(
                Arc::new(ScriptedClaimRepo::new()),
                ReplicaId::new("zero-config-test"),
                config,
            )
            .expect_err("zero duration must fail at construction");
            assert!(matches!(
                error,
                ConfigError::ZeroDuration {
                    field: actual
                } if actual == field
            ));
        }
    }

    #[test]
    fn zero_sentinel_threshold_is_rejected_before_provider_work() {
        let error = RefreshCoordinator::new_with(
            Arc::new(ScriptedClaimRepo::new()),
            ReplicaId::new("test-replica"),
            RefreshCoordConfig {
                sentinel_threshold: 0,
                ..RefreshCoordConfig::default()
            },
        )
        .expect_err("zero sentinel threshold must fail construction");

        assert!(matches!(error, ConfigError::ZeroSentinelThreshold));
    }

    #[tokio::test]
    async fn repeated_poison_denials_leave_threshold_observation_to_periodic_owner() {
        use nebula_eventbus::EventBus;

        use crate::{CredentialEvent, contract::resolve::ReauthReason};

        use super::super::{
            reclaim::run_one_sweep,
            sentinel::{SentinelThresholdConfig, SentinelTrigger},
        };

        let credential_id = CredentialId::new();
        let repo = Arc::new(PoisonClaimRepo::new(credential_id));
        let repo_port: Arc<dyn RefreshClaimRepo> = repo.clone();
        let coordinator = Arc::new(
            RefreshCoordinator::new_with(
                Arc::clone(&repo_port),
                ReplicaId::new("test-replica"),
                RefreshCoordConfig {
                    sentinel_threshold: 1,
                    ..RefreshCoordConfig::default()
                },
            )
            .expect("test coordinator config is valid"),
        );
        let provider_calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let calls = Arc::clone(&provider_calls);
            let outcome = coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await;
            assert!(matches!(outcome, Err(RefreshError::CriticalOutcomePending)));
        }

        assert_eq!(
            repo.reclaim_count.load(Ordering::SeqCst),
            0,
            "request paths must not consume periodic accounting work"
        );
        assert_eq!(repo.evidence_count.load(Ordering::SeqCst), 0);

        let event_bus = Arc::new(EventBus::new(8));
        let mut events = event_bus.subscribe();
        let sentinel = Arc::new(SentinelTrigger::new(
            Arc::clone(&repo_port),
            SentinelThresholdConfig {
                threshold: coordinator.config.sentinel_threshold,
                window: coordinator.config.sentinel_window,
            },
        ));
        run_one_sweep(
            &repo_port,
            &sentinel,
            Some(&event_bus),
            coordinator.metrics(),
            None,
        )
        .await
        .expect("periodic owner should account poison");

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("threshold observation should be emitted")
            .expect("event bus should remain open");
        assert!(matches!(
            event,
            CredentialEvent::ReauthRequired {
                credential_id: observed,
                reason: ReauthReason::SentinelRepeated {
                    event_count: 1,
                    ..
                },
            } if observed == credential_id
        ));

        run_one_sweep(
            &repo_port,
            &sentinel,
            Some(&event_bus),
            coordinator.metrics(),
            None,
        )
        .await
        .expect("repeated periodic sweep should be idempotent");

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 2);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert_eq!(repo.reclaim_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            repo.evidence_count.load(Ordering::SeqCst),
            1,
            "periodic reclaim must record one durable event per poisoned generation"
        );
        assert_eq!(coordinator.metrics.claims_outcome_unknown.get(), 2);
        assert_eq!(
            coordinator.metrics.reclaim_outcome_unknown_accounted.get(),
            1
        );
        assert_eq!(coordinator.metrics.reclaim_reclaimed.get(), 0);
        assert_eq!(coordinator.metrics.reclaim_no_work.get(), 1);
        assert!(
            events.try_recv().is_none(),
            "already-accounted poison must not emit a duplicate threshold observation"
        );
    }

    #[tokio::test]
    async fn caller_drop_cannot_release_or_duplicate_owned_provider_commit() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(AtomicUsize::new(0));
        let provider_entered = Arc::new(Notify::new());
        let commit_continue = Arc::new(Notify::new());

        let winner_coordinator = Arc::clone(&coordinator);
        let winner_provider_calls = Arc::clone(&provider_calls);
        let winner_writes = Arc::clone(&writes);
        let winner_provider_entered = Arc::clone(&provider_entered);
        let winner_commit_continue = Arc::clone(&commit_continue);
        let winner = tokio::spawn(async move {
            winner_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        winner_provider_calls.fetch_add(1, Ordering::SeqCst);
                        winner_provider_entered.notify_one();
                        winner_commit_continue.notified().await;
                        winner_writes.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(7_u8)
                    },
                )
                .await
        });

        provider_entered.notified().await;
        winner.abort();
        assert!(winner.await.is_err(), "the outer waiter was cancelled");
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert!(repo.active.load(Ordering::SeqCst));
        assert_eq!(coordinator.l1.in_flight_count(), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 0);

        let duplicate_calls = Arc::new(AtomicUsize::new(0));
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
        let waiter_writes = Arc::clone(&writes);
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &credential_id,
                    move |_| {
                        let writes = Arc::clone(&waiter_writes);
                        async move { Ok(writes.load(Ordering::SeqCst) == 0) }
                    },
                    move || async move {
                        waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(9_u8)
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "L1 waiter woke before commit disposition"
        );

        commit_continue.notify_one();
        repo.wait_for_release().await;
        let waiter_result = waiter.await.expect("waiter task joins");
        assert!(matches!(
            waiter_result,
            Err(RefreshError::CoalescedByOtherReplica)
        ));
        wait_until_l1_empty(&coordinator).await;
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(duplicate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn state_advanced_completion_can_elect_one_winner_for_a_newer_epoch() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();
        let first_entered = Arc::new(Notify::new());
        let first_continue = Arc::new(Notify::new());

        let winner_coordinator = Arc::clone(&coordinator);
        let winner_entered = Arc::clone(&first_entered);
        let winner_continue = Arc::clone(&first_continue);
        let winner_id = credential_id;
        let winner = tokio::spawn(async move {
            winner_coordinator
                .refresh_coalesced(
                    &winner_id,
                    |_| async { Ok(true) },
                    move || async move {
                        winner_entered.notify_one();
                        winner_continue.notified().await;
                        RefreshDisposition::state_advanced(1_u8)
                    },
                )
                .await
        });
        first_entered.notified().await;

        let recheck_entered = Arc::new(Notify::new());
        let recheck_continue = Arc::new(Notify::new());
        let second_provider_calls = Arc::new(AtomicUsize::new(0));
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_recheck_entered = Arc::clone(&recheck_entered);
        let waiter_recheck_continue = Arc::clone(&recheck_continue);
        let waiter_provider_calls = Arc::clone(&second_provider_calls);
        let waiter_id = credential_id;
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &waiter_id,
                    move |_| {
                        let entered = Arc::clone(&waiter_recheck_entered);
                        let continue_recheck = Arc::clone(&waiter_recheck_continue);
                        async move {
                            entered.notify_one();
                            continue_recheck.notified().await;
                            // The first winner advanced its epoch, but newer
                            // authoritative work arrived before this waiter
                            // rechecked state.
                            Ok(true)
                        }
                    },
                    move || async move {
                        waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(2_u8)
                    },
                )
                .await
        });

        while coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string())
            == 0
        {
            tokio::task::yield_now().await;
        }
        first_continue.notify_one();
        recheck_entered.notified().await;
        repo.wait_for_release().await;
        recheck_continue.notify_one();

        assert_eq!(
            winner
                .await
                .expect("first winner task must join")
                .expect("first epoch must complete"),
            1
        );
        assert_eq!(
            waiter
                .await
                .expect("new-epoch waiter task must join")
                .expect("newer epoch must elect a winner"),
            2
        );
        assert_eq!(second_provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn exact_no_progress_completion_does_not_turn_waiters_into_a_retry_herd() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();
        let first_entered = Arc::new(Notify::new());
        let first_continue = Arc::new(Notify::new());

        let winner_coordinator = Arc::clone(&coordinator);
        let winner_entered = Arc::clone(&first_entered);
        let winner_continue = Arc::clone(&first_continue);
        let winner_id = credential_id;
        let winner = tokio::spawn(async move {
            winner_coordinator
                .refresh_coalesced(
                    &winner_id,
                    |_| async { Ok(true) },
                    move || async move {
                        winner_entered.notify_one();
                        winner_continue.notified().await;
                        RefreshDisposition::no_state_change(1_u8)
                    },
                )
                .await
        });
        first_entered.notified().await;

        let duplicate_provider_calls = Arc::new(AtomicUsize::new(0));
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_provider_calls = Arc::clone(&duplicate_provider_calls);
        let waiter_id = credential_id;
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &waiter_id,
                    |_| async { Ok(true) },
                    move || async move {
                        waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(2_u8)
                    },
                )
                .await
        });

        while coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string())
            == 0
        {
            tokio::task::yield_now().await;
        }
        first_continue.notify_one();

        assert_eq!(
            winner
                .await
                .expect("first winner task must join")
                .expect("exact first outcome must be returned"),
            1
        );
        assert!(matches!(
            waiter.await.expect("waiter task must join"),
            Err(RefreshError::PriorAttemptNoProgress)
        ));
        assert_eq!(duplicate_provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 1);
        repo.wait_for_release().await;
    }

    #[tokio::test]
    async fn replay_unsafe_completion_keeps_waiters_fail_closed() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();
        let first_entered = Arc::new(Notify::new());
        let first_continue = Arc::new(Notify::new());

        let winner_coordinator = Arc::clone(&coordinator);
        let winner_entered = Arc::clone(&first_entered);
        let winner_continue = Arc::clone(&first_continue);
        let winner_id = credential_id;
        let winner = tokio::spawn(async move {
            winner_coordinator
                .refresh_coalesced(
                    &winner_id,
                    |_| async { Ok(true) },
                    move || async move {
                        winner_entered.notify_one();
                        winner_continue.notified().await;
                        RefreshDisposition::retry_unsafe(1_u8)
                    },
                )
                .await
        });
        first_entered.notified().await;

        let duplicate_provider_calls = Arc::new(AtomicUsize::new(0));
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_provider_calls = Arc::clone(&duplicate_provider_calls);
        let waiter_id = credential_id;
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &waiter_id,
                    |_| async { Ok(true) },
                    move || async move {
                        waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(2_u8)
                    },
                )
                .await
        });

        while coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string())
            == 0
        {
            tokio::task::yield_now().await;
        }
        first_continue.notify_one();

        assert_eq!(
            winner
                .await
                .expect("first winner task must join")
                .expect("exact unsafe outcome must reach its owner"),
            1
        );
        assert!(matches!(
            waiter.await.expect("waiter task must join"),
            Err(RefreshError::CriticalOutcomePending)
        ));
        assert_eq!(duplicate_provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert!(
            repo.active.load(Ordering::SeqCst),
            "retry-unsafe completion must retain the sentinel claim"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn released_contended_claim_is_polled_well_before_its_ttl() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        repo.active.store(true, Ordering::SeqCst);
        let config = RefreshCoordConfig {
            claim_ttl: Duration::from_millis(300),
            heartbeat_interval: Duration::from_millis(25),
            refresh_timeout: Duration::from_millis(100),
            reclaim_sweep_interval: Duration::from_millis(300),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_mins(1),
        };
        let coordinator = coordinator(Arc::clone(&repo), config);
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let task_provider_calls = Arc::clone(&provider_calls);
        let task_coordinator = Arc::clone(&coordinator);

        let task = tokio::spawn(async move {
            task_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        task_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        repo.wait_for_try_claim_count(1).await;
        repo.active.store(false, Ordering::SeqCst);
        tokio::time::advance(Duration::from_millis(40)).await;
        tokio::task::yield_now().await;

        task.await
            .expect("coordinator task must join")
            .expect("released claim must be acquired on the next adaptive poll");
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            repo.try_claim_count.load(Ordering::SeqCst),
            2,
            "claim release must be observed without sleeping to the 300 ms TTL"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn post_contention_recheck_failure_denies_provider_dispatch() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        repo.active.store(true, Ordering::SeqCst);
        let coordinator = coordinator(Arc::clone(&repo), paused_config());
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let task_provider_calls = Arc::clone(&provider_calls);
        let task_coordinator = Arc::clone(&coordinator);

        let task = tokio::spawn(async move {
            task_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Err(RefreshRecheckError::Unavailable) },
                    move || async move {
                        task_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        repo.wait_for_try_claim_count(1).await;
        tokio::time::advance(Duration::from_millis(200)).await;
        let result = task.await.expect("coordinator task joins");

        assert!(matches!(
            result,
            Err(RefreshError::StateRecheck(RefreshRecheckError::Unavailable))
        ));
        assert_eq!(
            provider_calls.load(Ordering::SeqCst),
            0,
            "an unavailable authoritative recheck must never authorize provider egress"
        );
        assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 1);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert_eq!(coordinator.l1.in_flight_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn l1_waiter_timeout_is_bounded_without_false_coalesced_success() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let config = paused_config();
        let coordinator = coordinator(Arc::clone(&repo), config.clone());
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let duplicate_calls = Arc::new(AtomicUsize::new(0));
        let provider_entered = Arc::new(Notify::new());
        let provider_continue = Arc::new(Notify::new());

        let winner_coordinator = Arc::clone(&coordinator);
        let winner_calls = Arc::clone(&provider_calls);
        let winner_entered = Arc::clone(&provider_entered);
        let winner_continue = Arc::clone(&provider_continue);
        let winner_id = credential_id;
        let winner = tokio::spawn(async move {
            winner_coordinator
                .refresh_coalesced(
                    &winner_id,
                    |_| async { Ok(true) },
                    move || async move {
                        winner_calls.fetch_add(1, Ordering::SeqCst);
                        winner_entered.notify_one();
                        winner_continue.notified().await;
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        provider_entered.notified().await;
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
        let waiter_id = credential_id;
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &waiter_id,
                    |_| async { Ok(true) },
                    move || async move {
                        waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        for _ in 0..8 {
            if coordinator
                .l1
                .waiter_count_for_test(&credential_id.to_string())
                == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            coordinator
                .l1
                .waiter_count_for_test(&credential_id.to_string()),
            1,
            "the second caller must register as an L1 waiter"
        );

        tokio::time::advance(config.refresh_timeout).await;
        let winner_outcome = winner.await.expect("winner caller joins");
        let waiter_outcome = waiter.await.expect("L1 waiter joins");
        assert!(matches!(
            winner_outcome,
            Err(RefreshError::CriticalOutcomePending)
        ));
        assert!(matches!(
            waiter_outcome,
            Err(RefreshError::CriticalOutcomePending)
        ));
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(duplicate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            coordinator
                .l1
                .waiter_count_for_test(&credential_id.to_string()),
            0,
            "timed-out waiters must not accumulate behind a stuck winner"
        );
        assert_eq!(
            coordinator.metrics.coalesced_l1.get(),
            0,
            "an unresolved waiter timeout is not a coalesced success"
        );
        assert_eq!(
            coordinator.l1.in_flight_count(),
            1,
            "the owned critical task must retain the L1 winner entry"
        );

        provider_continue.notify_one();
        repo.wait_for_release().await;
        wait_until_l1_empty(&coordinator).await;
    }

    #[tokio::test(start_paused = true)]
    async fn caller_timeout_detaches_and_heartbeat_keeps_lease_past_original_ttl() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let config = paused_config();
        let coordinator = coordinator(Arc::clone(&repo), config.clone());
        let credential_id = CredentialId::new();
        let provider_entered = Arc::new(Notify::new());
        let commit_continue = Arc::new(Notify::new());
        let writes = Arc::new(AtomicUsize::new(0));

        let task_coordinator = Arc::clone(&coordinator);
        let task_entered = Arc::clone(&provider_entered);
        let task_continue = Arc::clone(&commit_continue);
        let task_writes = Arc::clone(&writes);
        let waiter = tokio::spawn(async move {
            task_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        task_entered.notify_one();
                        task_continue.notified().await;
                        task_writes.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        provider_entered.notified().await;
        tokio::time::advance(config.refresh_timeout).await;
        let outcome = waiter.await.expect("timeout waiter joins");
        assert!(matches!(outcome, Err(RefreshError::CriticalOutcomePending)));
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert_eq!(coordinator.l1.in_flight_count(), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 0);

        // The caller has already gone away, but the owned critical task still
        // holds and heartbeats L2. Advancing past the acquisition's original
        // TTL must not turn elapsed time into replay authority.
        tokio::time::advance(config.claim_ttl).await;
        tokio::task::yield_now().await;
        assert!(
            repo.heartbeat_count.load(Ordering::SeqCst) > 0,
            "the detached exact-outcome task must renew its lease past the original TTL"
        );
        assert!(repo.active.load(Ordering::SeqCst));

        commit_continue.notify_one();
        repo.wait_for_release().await;
        wait_until_l1_empty(&coordinator).await;
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_loss_after_provider_boundary_cannot_cancel_commit() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let config = paused_config();
        let coordinator = coordinator(Arc::clone(&repo), config.clone());
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(AtomicUsize::new(0));
        let provider_entered = Arc::new(Notify::new());
        let commit_continue = Arc::new(Notify::new());

        let task_coordinator = Arc::clone(&coordinator);
        let task_provider_calls = Arc::clone(&provider_calls);
        let task_writes = Arc::clone(&writes);
        let task_entered = Arc::clone(&provider_entered);
        let task_continue = Arc::clone(&commit_continue);
        let waiter = tokio::spawn(async move {
            task_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        task_provider_calls.fetch_add(1, Ordering::SeqCst);
                        task_entered.notify_one();
                        task_continue.notified().await;
                        task_writes.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(13_u8)
                    },
                )
                .await
        });

        provider_entered.notified().await;
        repo.heartbeat_mode.store(HEARTBEAT_LOST, Ordering::SeqCst);
        advance_until_heartbeat(&repo, config.heartbeat_interval).await;
        assert!(!waiter.is_finished());
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert_eq!(writes.load(Ordering::SeqCst), 0);

        commit_continue.notify_one();
        assert_eq!(waiter.await.expect("waiter joins").expect("confirmed"), 13);
        repo.wait_for_release().await;
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_loss_before_provider_boundary_starts_no_provider_work() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        repo.block_sentinel.store(true, Ordering::SeqCst);
        repo.heartbeat_mode.store(HEARTBEAT_LOST, Ordering::SeqCst);
        let config = paused_config();
        let coordinator = coordinator(Arc::clone(&repo), config.clone());
        let credential_id = CredentialId::new();
        let provider_calls = Arc::new(AtomicUsize::new(0));

        let task_coordinator = Arc::clone(&coordinator);
        let task_provider_calls = Arc::clone(&provider_calls);
        let waiter = tokio::spawn(async move {
            task_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        task_provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });

        repo.sentinel_entered.notified().await;
        advance_until_heartbeat(&repo, config.heartbeat_interval).await;
        let outcome = waiter.await.expect("waiter joins");
        assert!(matches!(
            outcome,
            Err(RefreshError::ClaimLostBeforeProvider)
        ));
        repo.wait_for_release().await;
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
        assert!(!repo.active.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn hung_l2_release_cannot_wedge_l1_or_global_permit() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        repo.block_release.store(true, Ordering::SeqCst);
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();
        let baseline_permits = coordinator.l1.available_permits();

        let result = coordinator
            .refresh_coalesced(
                &credential_id,
                |_| async { Ok(true) },
                || async { RefreshDisposition::state_advanced(55_u8) },
            )
            .await
            .expect("exact result must not wait on L2 release");
        assert_eq!(result, 55);
        assert_eq!(
            coordinator.l1.in_flight_count(),
            0,
            "exact disposition must wake local waiters before L2 release"
        );
        assert_eq!(
            coordinator.l1.available_permits(),
            baseline_permits,
            "exact disposition must return the global permit before L2 release"
        );
        repo.release_entered.notified().await;

        let duplicate_calls = Arc::new(AtomicUsize::new(0));
        let waiter_coordinator = Arc::clone(&coordinator);
        let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
        let waiter = tokio::spawn(async move {
            waiter_coordinator
                .refresh_coalesced(
                    &credential_id,
                    |_| async { Ok(true) },
                    move || async move {
                        waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        });
        repo.wait_for_try_claim_count(2).await;
        assert_eq!(
            duplicate_calls.load(Ordering::SeqCst),
            0,
            "the still-live L2 row must coalesce the local waiter"
        );

        waiter.abort();
        assert!(waiter.await.is_err());
        wait_until_l1_empty(&coordinator).await;
        assert_eq!(coordinator.l1.available_permits(), baseline_permits);

        repo.release_continue.notify_one();
        repo.wait_for_release().await;
        assert!(!repo.active.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unknown_commit_ack_retains_claim_as_durable_poison() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();

        let result = coordinator
            .refresh_coalesced(
                &credential_id,
                |_| async { Ok(true) },
                || async { RefreshDisposition::outcome_unknown(21_u8) },
            )
            .await
            .expect("the enclosed typed outcome is returned");

        assert_eq!(result, 21);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert!(repo.active.load(Ordering::SeqCst));
        assert_eq!(coordinator.l1.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn definite_post_provider_failure_also_blocks_immediate_replay() {
        let repo = Arc::new(ScriptedClaimRepo::new());
        let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
        let credential_id = CredentialId::new();

        let result = coordinator
            .refresh_coalesced(
                &credential_id,
                |_| async { Ok(true) },
                || async { RefreshDisposition::retry_unsafe(34_u8) },
            )
            .await
            .expect("the exact enclosed failure is returned");

        assert_eq!(result, 34);
        assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
        assert!(
            repo.active.load(Ordering::SeqCst),
            "another replica must not immediately re-POST the persisted stale grant"
        );
        assert_eq!(coordinator.l1.in_flight_count(), 0);
    }
}
