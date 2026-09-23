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
//! Callers invoke `refresh_coalesced(selector, do_refresh)`. The
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
use nebula_storage_port::CredentialSelector;
use nebula_storage_port::store::{
    ClaimAttempt, ClaimToken, HeartbeatError, RefreshClaim, RefreshClaimStore as RefreshClaimRepo,
    ReplicaId,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::audit::AuditSink;

use super::{
    audit::emit_claim_acquired,
    l1::{L1Completion, L1RefreshCoalescer},
    metrics::RefreshCoordMetrics,
};

mod config;

pub use config::{ConfigError, RefreshCoordConfig};
mod errors;
mod lease;

pub use errors::{RefreshDisposition, RefreshError, RefreshRecheck, RefreshRecheckError};

use errors::ClaimFinalization;
use lease::{L1RefreshLease, RefreshLease};

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
    /// These events observe a transition already committed by the atomic
    /// reclaim boundary. Without a sink, audit emission is a no-op (the metric
    /// and tracing surfaces still observe).
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
    /// `RefreshCoordReauthThresholdReached` is emitted only after the durable
    /// transition result is returned.
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
            credential_id = %selector.credential_id(),
            replica_id = %self.replica_id,
            tier = tracing::field::Empty,
        ),
    )]
    pub async fn refresh_coalesced<F, Fut, T, P, PFut>(
        &self,
        selector: &CredentialSelector,
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
        let credential_id = selector.credential_id();
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
                        needs_refresh_after_backoff(&credential_id),
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
        // prevent. Only the legacy `String`-id refresh path in
        // `resolver/mod.rs` (since deleted) consumed permits, so
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
            .try_acquire_l2_with_backoff(selector, &needs_refresh_after_backoff)
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
            &credential_id,
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
            credential_id,
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
            needs_refresh_after_backoff(&credential_id),
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
        selector: &CredentialSelector,
        needs_refresh_after_backoff: &P,
    ) -> Result<RefreshClaim, RefreshError>
    where
        // Mirror the `Send`/`Sync` bounds on `refresh_coalesced` so the
        // helper's auto-trait inference does not silently relax the
        // public contract.
        P: Fn(&CredentialId) -> PFut + Sync,
        PFut: Future<Output = Result<RefreshRecheck, RefreshRecheckError>> + Send,
    {
        let credential_id = selector.credential_id();
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
                    .try_claim(selector, &self.replica_id, self.config.claim_ttl)
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
                    match needs_refresh_after_backoff(&credential_id).await {
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
