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

use crate::RefreshNotAppliedContext;
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
        /// Property whose zero value would make lease semantics invalid or panic
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
    /// Another in-process attempt reached an exact finalization failure that
    /// cannot safely be replayed.
    ///
    /// The winner retained the durable claim as poison. Unlike
    /// [`Self::CriticalOutcomePending`], the operation outcome is known; the
    /// command owner must surface its operation-specific reconciliation
    /// contract.
    #[error("a concurrent refresh operation requires reconciliation before retrying")]
    ReconciliationRequired,
    /// Another in-process attempt reached an exact, replay-safe outcome but
    /// did not advance authoritative state.
    ///
    /// No provider or persistence operation remains pending, but the
    /// coordinator deliberately does not turn every waiter into an immediate
    /// retry. The caller may retry later under its normal backoff and circuit
    /// policy.
    #[error("a concurrent refresh attempt completed without advancing credential state")]
    PriorAttemptNoProgress,
    /// A backend-authoritative retry gate forbids provider dispatch for the
    /// current credential epoch.
    ///
    /// The caller must re-read the typed gate evidence rather than treating
    /// this as successful coalescing or a generic retryable failure.
    #[error("credential refresh retry is suppressed by durable aggregate state: {0}")]
    RetrySuppressed(Box<RefreshNotAppliedContext>),
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

/// Authoritative result of rechecking a refresh contender's captured epoch.
#[derive(Debug)]
#[non_exhaustive]
pub enum RefreshRecheck {
    /// The same credential epoch still needs provider/local refresh work.
    Needed,
    /// Authoritative state advanced or no longer needs this operation.
    Satisfied,
    /// A durable retry gate forbids dispatch for the current epoch.
    Suppressed(Box<RefreshNotAppliedContext>),
}

/// Exact disposition of an owned provider/persistence refresh section.
///
/// The coordinator needs this distinction to finalize the durable L2 claim
/// safely and tell L1 waiters what the completion proves. A durable state
/// advance may release immediately and permits a later refresh epoch. An exact
/// replay-safe outcome without a state advance also releases, but waiters do
/// not automatically retry it. An exact finalization failure after either a
/// provider or local refresh, or an unknown provider/commit outcome, stops
/// heartbeats but deliberately leaves the sentinel claim in place. Once its
/// lease expires, storage keeps it as durable fail-closed poison so refresh
/// work cannot replay before explicit reconciliation.
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
    /// Refresh work completed, but its new state was definitely not persisted.
    ///
    /// The enclosed error is exact, yet another replica must not immediately
    /// repeat the refresh against stale authoritative state. Like an unknown
    /// acknowledgement, this retains the sentinel claim. Expiry converts it
    /// into durable poison, which only an owner-qualified reconciliation
    /// command may clear.
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

    /// Construct a definite finalization failure that is unsafe to replay.
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
            // any panic/runtime teardown must wake waiters as genuinely
            // outcome-unknown.
            l1.set_completion(L1Completion::OutcomeUnknown);
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
                    // outcome. This branch is `finalization ==
                    // ClaimFinalization::Release`, which both the pre-provider
                    // cleanup sites and the post-provider state-disposition
                    // sites choose, so the error carries no provider outcome of
                    // its own: only `ReleaseRefused` means the sweep already
                    // accounted the claim's incident. That refusal leaves the
                    // row poison until `adjudicate` records the provider
                    // outcome, and waiting for claim expiry cannot clear it
                    // because the retained row outlives its expiry.
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
            // `OutcomeUnknown` disposition does, leaving the claim durable
            // poison until `adjudicate` records the provider outcome.
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
    /// 4. Recheck authoritative state after every successful L2 acquisition, including an
    ///    immediate acquisition.
    /// 5. Confirm the sentinel transition that marks the irreversible provider boundary.
    /// 6. Transfer the heartbeat and claim into an owned provider/persistence task.
    /// 7. Release after `StateAdvanced`/`NoStateChange`, or retain as durable poison after
    ///    `RetryUnsafe`/`OutcomeUnknown`.
    ///
    /// The provider closure receives no claim or token. Durable claim authority
    /// is coordinator-private and cannot be released, heartbeated, or reused by
    /// integration code.
    ///
    /// `needs_refresh_after_backoff` is consulted after L1 completion, by the
    /// L2 backoff loop after a post-`Contended` sleep, and once more after any
    /// successful L2 acquisition before the sentinel transition.
    /// [`RefreshRecheck::Satisfied`] means authoritative state changed or no
    /// longer needs this operation, so the caller re-reads it through
    /// [`RefreshError::CoalescedByOtherReplica`].
    /// [`RefreshRecheck::Needed`] authorizes another claim attempt, while
    /// [`RefreshRecheck::Suppressed`] reports a durable retry gate without
    /// flattening it into coalesced success. `Err` denies provider dispatch with a typed,
    /// pre-provider [`RefreshError::StateRecheck`].
    ///
    /// Callers without an external state source may pass
    /// `|_| async { Ok(RefreshRecheck::Needed) }`. Persistence-backed callers must perform a
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
        PFut: Future<Output = Result<RefreshRecheck, RefreshRecheckError>> + Send,
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
        // Exact no-progress, retry-unsafe, and outcome-unknown completions
        // remain distinct, so a provider failure cannot turn a waiting herd
        // into automatic retries or erase exact reconciliation evidence.
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
                    // `{l1, l1_no_progress, l1_reconciliation_required,
                    // l1_outcome_unknown, l2_acquired, l2_coalesced,
                    // l2_outcome_unknown}` is recorded at the actual outcome
                    // sites below.
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

                    match still_needs_refresh {
                        RefreshRecheck::Satisfied => {
                            tracing::Span::current().record("tier", "l1");
                            self.metrics.coalesced_l1.inc();
                            return Err(RefreshError::CoalescedByOtherReplica);
                        },
                        RefreshRecheck::Suppressed(context) => {
                            tracing::Span::current().record("tier", "l1_no_progress");
                            return Err(RefreshError::RetrySuppressed(context));
                        },
                        RefreshRecheck::Needed => {},
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
                        L1Completion::RetryUnsafe => {
                            tracing::Span::current().record("tier", "l1_reconciliation_required");
                            tracing::warn!(
                                event = "credential.refresh.l1.wait.reconciliation_required",
                                completion = "retry_unsafe",
                                credential_id = %credential_id,
                                "exact L1 winner outcome requires reconciliation before replay"
                            );
                            return Err(RefreshError::ReconciliationRequired);
                        },
                        L1Completion::OutcomeUnknown => {
                            tracing::Span::current().record("tier", "l1_outcome_unknown");
                            tracing::warn!(
                                event = "credential.refresh.l1.wait.outcome_unknown",
                                reason = "authoritative_state_unchanged_after_unknown_completion",
                                credential_id = %credential_id,
                                "L1 winner outcome is unknown and cannot be replayed safely"
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
        // (`resolver/mod.rs::refresh_via_l1_only`) consumed permits, so
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

        // Close the stale-preflight window after claim acquisition. A caller
        // can observe `Open`, pause, then acquire immediately after another
        // replica durably installs a retry gate and releases L2. Rechecking
        // only after `Contended` would let that stale caller cross the
        // sentinel/provider boundary without ever observing the gate.
        let post_claim_recheck = tokio::time::timeout(
            self.config.refresh_timeout,
            needs_refresh_after_backoff(credential_id),
        )
        .await;
        match post_claim_recheck {
            Ok(Ok(RefreshRecheck::Needed)) => {},
            Ok(Ok(RefreshRecheck::Satisfied)) => {
                lease
                    .finish(ClaimFinalization::Release, L1Completion::StateAdvanced)
                    .await;
                self.metrics.coalesced_l2.inc();
                return Err(RefreshError::CoalescedByOtherReplica);
            },
            Ok(Ok(RefreshRecheck::Suppressed(context))) => {
                lease
                    .finish(ClaimFinalization::Release, L1Completion::StateAdvanced)
                    .await;
                self.metrics.coalesced_l2.inc();
                return Err(RefreshError::RetrySuppressed(context));
            },
            Ok(Err(error)) => {
                lease
                    .finish(ClaimFinalization::Release, L1Completion::NoStateChange)
                    .await;
                return Err(RefreshError::StateRecheck(error));
            },
            Err(error) => {
                tracing::warn!(
                    event = "credential.refresh.l2.post_claim_recheck_timeout",
                    timeout_ms = self.config.refresh_timeout.as_millis(),
                    ?error,
                    credential_id = %credential_id,
                    "post-claim authoritative recheck timed out; provider dispatch denied"
                );
                lease
                    .finish(ClaimFinalization::Release, L1Completion::NoStateChange)
                    .await;
                return Err(RefreshError::StateRecheck(RefreshRecheckError::Unavailable));
            },
        }

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
                    L1Completion::RetryUnsafe,
                    result,
                ),
                RefreshDisposition::OutcomeUnknown(result) => (
                    ClaimFinalization::RetainAsPoison,
                    L1Completion::OutcomeUnknown,
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
        PFut: Future<Output = Result<RefreshRecheck, RefreshRecheckError>> + Send,
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
                        Ok(RefreshRecheck::Needed) => {
                            if tokio::time::Instant::now() >= contention_deadline {
                                break;
                            }
                            attempt = attempt.saturating_add(1);
                        },
                        Ok(RefreshRecheck::Satisfied) => {
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
                        Ok(RefreshRecheck::Suppressed(context)) => {
                            tracing::Span::current().record("tier", "l2_coalesced");
                            self.metrics.coalesced_l2.inc();
                            return Err(RefreshError::RetrySuppressed(context));
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
mod tests;
