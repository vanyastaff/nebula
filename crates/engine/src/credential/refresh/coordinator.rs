//! Outer two-tier refresh coordinator.
//!
//! Per ADR-0041 + sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
//! §3.1 (two-tier diagram), §3.5 (parameter invariants), §3.6 (contention
//! backoff).
//!
//! `RefreshCoordinator` composes:
//!
//! - **L1** — `super::l1::L1RefreshCoalescer` (in-process oneshot coalesce
//!   + per-credential circuit breaker + global concurrency semaphore).
//! - **L2** — `Arc<dyn nebula_storage::credential::RefreshClaimRepo>` (durable CAS-based claim with
//!   TTL + heartbeat).
//!
//! Callers invoke `refresh_coalesced(credential_id, do_refresh)`. The
//! coordinator acquires L1 first (fast in-process coalesce), then a
//! durable L2 claim with backoff per §3.6, runs the user's refresh
//! closure under both locks, and releases both on the way out.

use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_core::CredentialId;
use nebula_storage::credential::{
    AuditSink, ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, RefreshClaim,
    RefreshClaimRepo, ReplicaId, RepoError,
};
use tracing::Instrument;

use super::{
    audit::emit_claim_acquired,
    l1::{L1RefreshCoalescer, RefreshAttempt as L1Attempt, RefreshConfigError as L1ConfigError},
    metrics::RefreshCoordMetrics,
};

// ──────────────────────────────────────────────────────────────────────────
// Configuration
// ──────────────────────────────────────────────────────────────────────────

/// Configuration knobs for the two-tier coordinator.
///
/// Per sub-spec §3.5 the four time-related parameters carry interlocking
/// invariants verified by [`RefreshCoordConfig::validate`]:
///
/// - `heartbeat_interval × 3 ≤ claim_ttl` — three heartbeat ticks must fit inside one claim TTL so
///   two consecutive missed heartbeats still leave the claim valid until the next tick.
/// - `refresh_timeout + 2 × heartbeat_interval ≤ claim_ttl` — the holder must finish (or time out)
///   before its claim can expire.
/// - `reclaim_sweep_interval ≤ claim_ttl` — sweeps must run at least as often as a claim's TTL so a
///   crashed holder is reclaimed within one TTL window.
///
/// The boundary case `heartbeat_interval × 3 == claim_ttl` is allowed
/// (mirrors the ADR-0008 execution-lease shape: `ttl / 3 ==
/// heartbeat_interval`).
///
/// CI test asserts `RefreshCoordConfig::default().validate().is_ok()`.
#[derive(Clone, Debug)]
pub struct RefreshCoordConfig {
    /// Claim TTL applied to every L2 acquire/heartbeat call.
    pub claim_ttl: Duration,
    /// Cadence of background heartbeat ticks while a claim is held.
    pub heartbeat_interval: Duration,
    /// Maximum duration the user's refresh closure may run.
    pub refresh_timeout: Duration,
    /// Cadence of the background reclaim sweep (Stage 3.3).
    pub reclaim_sweep_interval: Duration,
    /// Sentinel events allowed inside `sentinel_window` before the
    /// credential is escalated to `ReauthRequired` (Stage 3.2).
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
    /// `heartbeat_interval × 3` exceeds `claim_ttl` — three heartbeat
    /// ticks would not fit inside one TTL window.
    #[error("heartbeat_interval × 3 must be ≤ claim_ttl")]
    HeartbeatTooSlow,
    /// `refresh_timeout + 2 × heartbeat_interval` exceeds `claim_ttl` —
    /// the holder cannot finish before its claim can expire.
    #[error("refresh_timeout + 2 × heartbeat_interval must be ≤ claim_ttl")]
    RefreshTimeoutTooLong,
    /// `reclaim_sweep_interval` exceeds `claim_ttl`.
    #[error("reclaim_sweep_interval must be ≤ claim_ttl")]
    ReclaimTooSlow,
}

impl RefreshCoordConfig {
    /// Verify the per-§3.5 interlocking invariants.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::*` whose variant names which invariant the
    /// configuration violates.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.heartbeat_interval * 3 > self.claim_ttl {
            return Err(ConfigError::HeartbeatTooSlow);
        }
        if self.refresh_timeout + self.heartbeat_interval * 2 > self.claim_ttl {
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
    /// Backoff retries exhausted before an L2 claim could be acquired.
    /// Surfaced when the contender's claim keeps being heartbeat-extended.
    #[error("contention exhausted after retries")]
    ContentionExhausted,
    /// Another replica's refresh succeeded while we were waiting on L2;
    /// caller treats as success and re-reads state.
    #[error("refresh coalesced by another replica (success — re-read state)")]
    CoalescedByOtherReplica,
    /// User closure exceeded `RefreshCoordConfig::refresh_timeout`.
    /// Distinct from `ContentionExhausted` so caller-side metrics and
    /// retry policy can differentiate "no claim could be acquired"
    /// from "we held the claim but the IdP call timed out".
    #[error("refresh timeout: closure exceeded {0:?}")]
    Timeout(Duration),
    /// Storage repo error (e.g. DB connectivity loss).
    #[error("storage repo error: {0}")]
    Repo(#[from] RepoError),
    /// Heartbeat task failure — claim lost or repo error.
    #[error("heartbeat error: {0}")]
    Heartbeat(#[from] HeartbeatError),
    /// Configuration invariant violated at construction time.
    #[error("config invalid: {0}")]
    Config(#[from] ConfigError),
}

// ──────────────────────────────────────────────────────────────────────────
// Coordinator
// ──────────────────────────────────────────────────────────────────────────

/// Two-tier credential refresh coordinator (L1 in-process + L2 cross-replica).
pub struct RefreshCoordinator {
    l1: L1RefreshCoalescer,
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

/// Result of attempting to begin a refresh for a credential — *legacy
/// L1-only API*.
///
/// Re-exported for the existing `CredentialResolver` call sites until
/// Stage 2.3 migrates them to the closure-based [`RefreshCoordinator::refresh_coalesced`]
/// surface. Once that migration lands, this enum (and the L1-delegate
/// methods on `RefreshCoordinator`) can be removed.
#[derive(Debug)]
pub enum RefreshAttempt {
    /// This caller won the race; perform the refresh, then call
    /// `RefreshCoordinator::complete()` to wake waiters.
    Winner,
    /// Another caller is already refreshing. Await the receiver; it
    /// resolves once the winner completes.
    Waiter(tokio::sync::oneshot::Receiver<()>),
}

impl From<L1Attempt> for RefreshAttempt {
    fn from(attempt: L1Attempt) -> Self {
        match attempt {
            L1Attempt::Winner => RefreshAttempt::Winner,
            L1Attempt::Waiter(rx) => RefreshAttempt::Waiter(rx),
        }
    }
}

/// Configuration errors for the legacy concurrency knob.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshConfigError {
    /// `max_concurrent` must be at least `1`.
    #[error("RefreshCoordinator::max_concurrent must be >= 1, got 0")]
    ZeroConcurrency,
}

impl From<L1ConfigError> for RefreshConfigError {
    fn from(err: L1ConfigError) -> Self {
        match err {
            L1ConfigError::ZeroConcurrency => RefreshConfigError::ZeroConcurrency,
        }
    }
}

impl RefreshCoordinator {
    /// Construct a coordinator wired to a given `RefreshClaimRepo`.
    ///
    /// Metrics are bound to a fresh in-memory registry by default — call
    /// [`Self::with_metrics`] post-construction to thread the engine-shared
    /// `MetricsRegistry`. Audit events are not emitted unless
    /// [`Self::with_audit_sink`] is called.
    ///
    /// # Errors
    ///
    /// Returns the corresponding [`ConfigError`] if `config.validate()`
    /// fails (see §3.5 invariants).
    pub fn new_with(
        repo: Arc<dyn RefreshClaimRepo>,
        replica_id: ReplicaId,
        config: RefreshCoordConfig,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            l1: L1RefreshCoalescer::new(),
            repo,
            replica_id,
            config,
            metrics: RefreshCoordMetrics::default(),
            audit_sink: None,
        })
    }

    /// Construct a default coordinator backed by an in-memory claim repo
    /// (suitable for tests and single-replica desktop mode).
    ///
    /// Production deployments must thread a real `RefreshClaimRepo`
    /// (Postgres or SQLite) through [`Self::new_with`].
    ///
    /// `Default` is intentionally not implemented (review I9): this
    /// constructor calls `expect()` on the default config validation,
    /// and convention says `Default::default()` must not panic. Callers
    /// pick `new()` (panicking semantics openly named) or `new_with(...)`
    /// (typed error).
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let config = RefreshCoordConfig::default();
        // Defaults are validated by the property test
        // `default_config_validates`; failures here would be a static bug,
        // so panic loudly rather than papering over.
        config
            .validate()
            .expect("RefreshCoordConfig::default() must satisfy §3.5 invariants");
        Self {
            l1: L1RefreshCoalescer::new(),
            repo,
            replica_id: ReplicaId::new(default_replica_id_string()),
            config,
            metrics: RefreshCoordMetrics::default(),
            audit_sink: None,
        }
    }

    /// Construct a coordinator with a custom L1 concurrency limit and an
    /// in-memory claim repo. Retained for the legacy `CredentialResolver`
    /// API — Stage 2.3 migrates the resolver to [`Self::new_with`] and
    /// this constructor goes away.
    ///
    /// # Errors
    ///
    /// Returns [`RefreshConfigError::ZeroConcurrency`] if `max == 0`.
    pub fn with_max_concurrent(max: usize) -> Result<Self, RefreshConfigError> {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let config = RefreshCoordConfig::default();
        config
            .validate()
            .expect("RefreshCoordConfig::default() must satisfy §3.5 invariants");
        Ok(Self {
            l1: L1RefreshCoalescer::with_max_concurrent(max)?,
            repo,
            replica_id: ReplicaId::new(default_replica_id_string()),
            config,
            metrics: RefreshCoordMetrics::default(),
            audit_sink: None,
        })
    }

    /// Replace the metric handles with ones bound to the engine-shared
    /// `MetricsRegistry`. Call once during composition; the coordinator
    /// emits all sub-spec §6 series against this registry afterwards.
    #[must_use = "builder methods must be chained or used"]
    pub fn with_metrics(mut self, metrics: RefreshCoordMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Attach an [`AuditSink`] to receive sub-spec §6 audit events
    /// (`RefreshCoordClaimAcquired`, `SentinelTriggered`,
    /// `ReauthFlagged`). Without a sink, audit emission is a no-op (the
    /// metric / tracing surfaces still observe).
    #[must_use = "builder methods must be chained or used"]
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    /// Borrow the pre-bound metric handles. Used by reclaim-sweep
    /// wiring so the sweep emits the same series.
    #[must_use]
    pub fn metrics(&self) -> &RefreshCoordMetrics {
        &self.metrics
    }

    /// Borrow the audit sink (`None` if not configured). Used by the
    /// reclaim sweep to emit `RefreshCoordSentinelTriggered` /
    /// `RefreshCoordReauthFlagged` events.
    #[must_use]
    pub fn audit_sink(&self) -> Option<&Arc<dyn AuditSink>> {
        self.audit_sink.as_ref()
    }

    /// Borrow the replica identifier this coordinator was constructed
    /// with.
    #[must_use]
    pub fn replica_id(&self) -> &ReplicaId {
        &self.replica_id
    }

    /// Borrow the validated config this coordinator was constructed
    /// with.
    #[must_use]
    pub fn config(&self) -> &RefreshCoordConfig {
        &self.config
    }

    /// Borrow the underlying claim repo. Used by call sites that need
    /// to mark the sentinel before performing the IdP POST (Stage 2.4)
    /// and by the reclaim sweep landing in Stage 3.3.
    pub fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }

    /// Acquire L1 mutex + L2 claim, run the refresh closure, release
    /// both. Returns `Err(CoalescedByOtherReplica)` if state was already
    /// fresh — caller treats as success and re-reads.
    ///
    /// Sub-spec §3.1 acquisition sequence:
    /// 1. L1 in-process coalesce (cheap fast-path; same-process concurrent calls collapse here).
    /// 2. L2 durable claim with backoff per §3.6.
    /// 3. Background heartbeat task — passes `self.config.claim_ttl` to each `repo.heartbeat(token,
    ///    ttl)` call (Stage 1 fix C2).
    /// 4. User-supplied `do_refresh(claim)` closure.
    /// 5. Stop heartbeat + release the claim row.
    ///
    /// `needs_refresh_after_backoff` is consulted by the L2 backoff loop
    /// per sub-spec §3.6 after the post-`Contended` sleep: if the
    /// predicate returns `false` the credential was refreshed by another
    /// replica while this caller was waiting and we short-circuit with
    /// [`RefreshError::CoalescedByOtherReplica`]. Callers that don't
    /// have a state-check available pass `|_| async { true }` — that
    /// preserves the legacy "always retry" behavior at the cost of
    /// occasionally running the refresh closure on a freshly-refreshed
    /// credential.
    ///
    /// # Errors
    ///
    /// See [`RefreshError`]. `CoalescedByOtherReplica` is success-with-side-effect:
    /// another replica refreshed while we were waiting. Caller should
    /// re-read the credential state and proceed.
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
        // Explicit `Send` bounds (review I2): the inner futures cross
        // task boundaries because `do_refresh` runs under
        // `tokio::time::timeout` and the predicate is awaited from the
        // spawn'd backoff loop. Without these bounds a `!Send` body
        // (e.g. one that captures an `Rc<...>`) compiles cleanly here
        // and surfaces an obscure auto-trait error at the call site.
        // Locking the contract on the trait bound moves the diagnostic
        // back to the user closure.
        F: FnOnce(RefreshClaim) -> Fut + Send,
        Fut: Future<Output = Result<T, RefreshError>> + Send,
        T: Send,
        P: Fn(&CredentialId) -> PFut + Sync,
        PFut: Future<Output = bool> + Send,
    {
        // L1: in-process coalescing.
        //
        // The L1 layer is keyed by string, so we hash on the typed id's
        // canonical form. `try_refresh` returns Winner for the first
        // caller and Waiter (with a oneshot::Receiver) for every other
        // concurrent caller in the same process. Waiters await the
        // Winner's `complete()` call, then surface
        // `CoalescedByOtherReplica` so the caller re-reads state — by
        // construction the Winner has already released the L2 claim and
        // committed the refreshed state to storage.
        let cred_str = credential_id.to_string();
        match self.l1.try_refresh(&cred_str) {
            super::l1::RefreshAttempt::Winner => {
                // NOTE: do NOT record `tier="l2"` here — the L2 path can
                // still produce `CoalescedByOtherReplica` via the
                // post-backoff recheck in
                // `try_acquire_l2_with_backoff`. Recording the tier
                // prematurely makes operators see "l2 acquired" when the
                // actual outcome was "l2 coalesced" (review I1).
                // The closed set `{l1, l2_acquired, l2_coalesced}` is
                // recorded at the actual outcome sites below.
                // (Fall through to acquire L2 + run the user closure.)
            },
            super::l1::RefreshAttempt::Waiter(rx) => {
                tracing::Span::current().record("tier", "l1");
                self.metrics.coalesced_l1.inc();
                // Wait for the Winner to finish. If the receiver errors
                // (Winner panicked / dropped without complete()) we still
                // re-read state — pessimistic safety.
                let _ = rx.await;
                return Err(RefreshError::CoalescedByOtherReplica);
            },
        }

        // Make sure `complete()` runs even on early return / panic so
        // future callers can re-acquire the L1 slot.
        let credential_id_for_guard = cred_str.clone();
        let l1 = &self.l1;
        let _l1_complete = scopeguard::guard((), move |()| {
            l1.complete(&credential_id_for_guard);
        });

        // L2: durable claim with backoff per §3.6.
        let claim = self
            .try_acquire_l2_with_backoff(credential_id, &needs_refresh_after_backoff)
            .await?;

        // Sub-spec §6 — record the claim acquisition once we know we own
        // the L2 row. `acquired` counter, audit event, and start of the
        // hold-duration measurement happen here so they are paired
        // with the matching `release` site below.
        //
        // Span tier (review I1) — record `l2_acquired` at the outcome
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

        // Heartbeat task in background.
        let hb_task = self.spawn_heartbeat(claim.token.clone());

        // Panic-safety guard (review C1, sub-spec §3.4). Fires ONLY on
        // panic unwind — `guard_on_unwind` does not fire on normal exit.
        // Without this, a panic in `do_refresh` would leak the heartbeat
        // task (extending L2 expiry forever) and skip `release()`,
        // blocking Stage 3.3 reclaim and cross-replica callers
        // (`ContentionExhausted` indefinitely). `release()` is
        // idempotent and the spawned task is detached because Drop is
        // synchronous; we cannot `.await` here.
        let token_for_unwind = claim.token.clone();
        let repo_for_unwind = Arc::clone(&self.repo);
        let hb_task_for_unwind = hb_task.abort_handle();
        let hold_duration_for_unwind = self.metrics.hold_duration.clone();
        let _l2_unwind_guard = scopeguard::guard_on_unwind((), move |()| {
            hb_task_for_unwind.abort();
            // Even on panic the hold time is observable — observe it
            // before the spawn so the histogram never drops a sample.
            hold_duration_for_unwind.observe(hold_start.elapsed().as_secs_f64());
            tokio::spawn(async move {
                if let Err(e) = repo_for_unwind.release(token_for_unwind).await {
                    tracing::warn!(?e, "L2 claim release on unwind failed");
                }
            });
        });

        // Keep the token for the normal-exit release; `do_refresh`
        // takes the full `RefreshClaim` (which it may inspect for
        // `expires_at`), so we hand it a clone and retain `token` here.
        let token_for_release = claim.token.clone();

        // Run user's refresh closure under `refresh_timeout` per §3.5.
        // The timeout is shorter than the claim TTL by construction, so
        // the heartbeat keeps the L2 row alive while the closure runs.
        let timeout = self.config.refresh_timeout;
        let result = tokio::time::timeout(timeout, do_refresh(claim))
            .await
            .map_err(|_elapsed| RefreshError::Timeout(timeout))
            .and_then(std::convert::identity);

        // Normal-exit release (review I1, sub-spec §3.4). We DO NOT
        // propagate release errors — propagating them with `?` would
        // mask a successful refresh: caller would observe
        // `RefreshError::Repo(...)`, route to `record_failure`, then
        // retry → ANOTHER IdP POST → invalidates the just-issued
        // refresh token (n8n #13088 spec lineage). Log at warn level
        // instead. The unwind guard above does NOT fire on normal exit
        // (per `guard_on_unwind` semantics), so this synchronous
        // release runs once, deterministically.
        hb_task.abort();
        // Sub-spec §6 — observe the hold duration on the normal-exit
        // release path. Symmetric with the unwind guard above.
        self.metrics
            .hold_duration
            .observe(hold_start.elapsed().as_secs_f64());
        if let Err(e) = self.repo.release(token_for_release).await {
            tracing::warn!(?e, "L2 claim release after successful refresh failed");
        }

        result
    }

    /// L2 acquisition retry loop per sub-spec §3.6.
    ///
    /// On `Contended` we sleep until the contender's claim is expected
    /// to expire (capped + jitter) then consult
    /// `needs_refresh_after_backoff(credential_id)`. If the predicate
    /// returns `false` we surface
    /// [`RefreshError::CoalescedByOtherReplica`] — another replica
    /// completed the refresh while we were waiting, and the caller
    /// should re-read state from storage. Otherwise we retry
    /// `try_claim` until we win the claim or exhaust attempts.
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
        PFut: Future<Output = bool> + Send,
    {
        const MAX_ATTEMPTS: usize = 5;
        for attempt in 0..MAX_ATTEMPTS {
            // Sub-spec §6 per-attempt tracing span: `attempt` and
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
                ClaimAttempt::Contended {
                    existing_expires_at,
                } => {
                    // Sub-spec §6 — bump the contended counter for every
                    // try_claim that returned Contended, regardless of
                    // whether the post-backoff recheck eventually
                    // short-circuits.
                    self.metrics.claims_contended.inc();
                    // Sleep until the contender's claim expires (capped
                    // at 5s so we don't sleep forever if their TTL is
                    // somehow much longer than ours), plus a jitter to
                    // de-correlate retries across replicas.
                    let now = chrono::Utc::now();
                    let cap = now + chrono::Duration::seconds(5);
                    let wait_until = existing_expires_at.min(cap);
                    let delay = (wait_until - now)
                        .to_std()
                        .unwrap_or(Duration::from_millis(200));
                    tokio::time::sleep(delay + jitter_ms(100)).await;
                    // CRITICAL: post-backoff state recheck per §3.6. If
                    // the contender finished the refresh while we slept,
                    // the credential is now fresh — short-circuit with
                    // CoalescedByOtherReplica so the caller re-reads
                    // state instead of running another IdP POST. Without
                    // this check, two replicas racing through L2 each
                    // run the closure (one wins try_claim now that the
                    // contender's row is gone), invalidating any
                    // refresh_token rotation the contender just
                    // committed (n8n #13088 lineage).
                    if !needs_refresh_after_backoff(credential_id).await {
                        // Sub-spec §6 — L2 coalesce: another replica
                        // refreshed while we waited.
                        //
                        // Span tier (review I1) — record `l2_coalesced`
                        // at the outcome site. We are now outside the
                        // per-attempt `instrument(span)` block (which
                        // wrapped only the `try_claim` future), so
                        // `Span::current()` resolves to the parent
                        // `credential.refresh.coordinate` span — the
                        // intended target. The closed set
                        // `{l1, l2_acquired, l2_coalesced}` is
                        // documented in OBSERVABILITY.md §7.2.
                        tracing::Span::current().record("tier", "l2_coalesced");
                        self.metrics.coalesced_l2.inc();
                        return Err(RefreshError::CoalescedByOtherReplica);
                    }
                },
            }
        }
        // Sub-spec §6 — every retry exhausted without acquiring the L2
        // row. `claims_total{outcome=exhausted} > 0` is a real production
        // signal worth alerting on.
        self.metrics.claims_exhausted.inc();
        Err(RefreshError::ContentionExhausted)
    }

    /// Spawn the background heartbeat task. Per Stage 1 fix C2 the
    /// trait's `heartbeat(token, ttl)` takes the same TTL passed to
    /// `try_claim`, so the §3.5 invariants
    /// (`heartbeat_interval × 3 < claim_ttl`,
    /// `reclaim_sweep_interval ≤ claim_ttl`) hold across heartbeats.
    fn spawn_heartbeat(&self, token: ClaimToken) -> tokio::task::JoinHandle<()> {
        let repo = Arc::clone(&self.repo);
        let interval = self.config.heartbeat_interval;
        let ttl = self.config.claim_ttl;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Avoid heartbeat amplification under storage backpressure:
            // if a heartbeat call exceeds `interval`, drop missed ticks
            // rather than firing them back-to-back when the call returns.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Burn the initial immediate tick — the claim was just
            // acquired and already has a fresh `expires_at`.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = repo.heartbeat(&token, ttl).await {
                    tracing::warn!(
                        ?e,
                        "credential refresh heartbeat failed; coordinator will release on next loop"
                    );
                    break;
                }
            }
        })
    }

    // ──────────────────────────────────────────────────────────────────
    // Legacy L1 surface — kept until Stage 2.3 migrates the resolver
    // to `refresh_coalesced`. Each method delegates to the inner
    // L1 coalescer.
    // ──────────────────────────────────────────────────────────────────

    /// **Legacy.** Begin a refresh attempt against the L1 coalescer.
    /// Stage 2.3 deletes this in favour of [`Self::refresh_coalesced`].
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        self.l1.try_refresh(credential_id).into()
    }

    /// **Legacy.** Mark the L1 in-flight slot complete. Stage 2.3
    /// deletes this in favour of [`Self::refresh_coalesced`].
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn complete(&self, credential_id: &str) {
        self.l1.complete(credential_id);
    }

    /// **Legacy.** Number of credentials currently being refreshed in
    /// the L1 layer.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn in_flight_count(&self) -> usize {
        self.l1.in_flight_count()
    }

    /// **Legacy.** Acquire a permit from the L1 concurrency limiter.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub async fn acquire_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.l1.acquire_permit().await
    }

    /// **Legacy.** Available permits in the L1 concurrency limiter.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn available_permits(&self) -> usize {
        self.l1.available_permits()
    }

    /// **Legacy.** Record a refresh failure for the L1 circuit breaker.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn record_failure(&self, credential_id: &str) {
        self.l1.record_failure(credential_id);
    }

    /// **Legacy.** Record a refresh success for the L1 circuit breaker.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn record_success(&self, credential_id: &str) {
        self.l1.record_success(credential_id);
    }

    /// **Legacy.** Whether the L1 per-credential circuit breaker is
    /// open.
    #[deprecated(
        since = "0.1.0",
        note = "use refresh_coalesced; remove when typed CredentialId migration completes — П3+"
    )]
    pub fn is_circuit_open(&self, credential_id: &str) -> bool {
        self.l1.is_circuit_open(credential_id)
    }
}

// `Default` deliberately not implemented: `Self::new()` calls
// `expect()` on `RefreshCoordConfig::default().validate()`, and
// convention is that `Default` does not panic. Callers construct
// explicitly via `RefreshCoordinator::new()` (which carries the
// validation panic semantics openly) or `new_with(...)` (typed error).

// ──────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────

/// Default replica id string used by the legacy single-replica
/// constructors. Production must thread an explicit replica id via
/// [`RefreshCoordinator::new_with`].
fn default_replica_id_string() -> String {
    // Single string — host name discovery happens at the composition
    // root (Stage 2.3). Until then any constant is fine; this is only
    // observable in diagnostics.
    "nebula-engine-default".to_string()
}

fn jitter_ms(max_ms: u64) -> Duration {
    if max_ms == 0 {
        return Duration::ZERO;
    }
    let amount = rand::random_range(0..max_ms);
    Duration::from_millis(amount)
}

// ──────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        // CI assertion that the shipped defaults satisfy the §3.5
        // interlocking invariants.
        assert!(RefreshCoordConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_rejects_heartbeat_too_slow() {
        // heartbeat × 3 = 33 > claim_ttl 30
        let cfg = RefreshCoordConfig {
            heartbeat_interval: Duration::from_secs(11),
            ..RefreshCoordConfig::default()
        };
        assert!(matches!(cfg.validate(), Err(ConfigError::HeartbeatTooSlow)));
    }

    #[test]
    fn validate_rejects_refresh_timeout_too_long() {
        // refresh_timeout + 2 × heartbeat_interval = 28+20 = 48 > 30
        let cfg = RefreshCoordConfig {
            refresh_timeout: Duration::from_secs(28),
            ..RefreshCoordConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::RefreshTimeoutTooLong)
        ));
    }

    #[test]
    fn validate_rejects_reclaim_too_slow() {
        let default = RefreshCoordConfig::default();
        let cfg = RefreshCoordConfig {
            reclaim_sweep_interval: default.claim_ttl + Duration::from_secs(1),
            ..default
        };
        assert!(matches!(cfg.validate(), Err(ConfigError::ReclaimTooSlow)));
    }

    #[test]
    fn validate_accepts_boundary_case_heartbeat_times_three_eq_ttl() {
        // 10s × 3 == 30s — boundary case; documented as allowed.
        let cfg = RefreshCoordConfig::default();
        assert_eq!(cfg.heartbeat_interval * 3, cfg.claim_ttl);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn new_with_propagates_validate_error() {
        // 11 × 3 = 33 > 30 → invalid
        let cfg = RefreshCoordConfig {
            heartbeat_interval: Duration::from_secs(11),
            ..RefreshCoordConfig::default()
        };
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let result = RefreshCoordinator::new_with(repo, ReplicaId::new("test"), cfg);
        assert!(matches!(result, Err(ConfigError::HeartbeatTooSlow)));
    }

    #[tokio::test]
    async fn refresh_coalesced_acquires_and_releases() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let coord = RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("a"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid");

        let cid = CredentialId::new();
        let result: Result<u32, RefreshError> = coord
            .refresh_coalesced(
                &cid,
                |_id| async { true },
                |claim| async move {
                    // Verify the claim is actually held — sentinel default
                    // is Normal; expires_at should be in the future.
                    assert!(claim.expires_at > chrono::Utc::now());
                    Ok(42)
                },
            )
            .await;
        assert_eq!(result.unwrap(), 42);

        // Row should be released — a fresh acquire wins immediately.
        let attempt = repo
            .try_claim(&cid, &ReplicaId::new("b"), Duration::from_secs(5))
            .await
            .unwrap();
        assert!(matches!(attempt, ClaimAttempt::Acquired(_)));
    }

    #[tokio::test]
    async fn legacy_default_constructor_is_valid() {
        // Existing callers relying on `RefreshCoordinator::new()` (no
        // args) keep compiling and produce a coordinator whose default
        // config validates.
        let coord = RefreshCoordinator::new();
        assert!(coord.config().validate().is_ok());
        assert_eq!(coord.replica_id().as_str(), "nebula-engine-default");
    }

    // ──────────────────────────────────────────────────────────────────
    // Panic-safety + release-error masking regression tests
    // (review feedback C1 + I1, sub-spec §3.4)
    // ──────────────────────────────────────────────────────────────────

    /// `FlakyReleaseRepo` delegates everything to an inner repo except
    /// `release`, which always returns `RepoError::InvalidState`. Used to
    /// prove the coordinator does not mask a successful refresh result
    /// when release fails after the user closure already returned `Ok`.
    struct FlakyReleaseRepo {
        inner: Arc<dyn RefreshClaimRepo>,
    }

    #[async_trait::async_trait]
    impl RefreshClaimRepo for FlakyReleaseRepo {
        async fn try_claim(
            &self,
            credential_id: &CredentialId,
            holder: &ReplicaId,
            ttl: Duration,
        ) -> Result<ClaimAttempt, RepoError> {
            self.inner.try_claim(credential_id, holder, ttl).await
        }

        async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
            self.inner.heartbeat(token, ttl).await
        }

        async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
            Err(RepoError::InvalidState("simulated release failure".into()))
        }

        async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
            self.inner.mark_sentinel(token).await
        }

        async fn reclaim_stuck(
            &self,
        ) -> Result<Vec<nebula_storage::credential::ReclaimedClaim>, RepoError> {
            self.inner.reclaim_stuck().await
        }

        async fn record_sentinel_event(
            &self,
            credential_id: &CredentialId,
            crashed_holder: &ReplicaId,
            generation: u64,
        ) -> Result<(), RepoError> {
            self.inner
                .record_sentinel_event(credential_id, crashed_holder, generation)
                .await
        }

        async fn count_sentinel_events_in_window(
            &self,
            credential_id: &CredentialId,
            window_start: chrono::DateTime<chrono::Utc>,
        ) -> Result<u32, RepoError> {
            self.inner
                .count_sentinel_events_in_window(credential_id, window_start)
                .await
        }
    }

    /// I1 regression — sub-spec §3.4. After Stage 2 review C1+I1: a
    /// transient `release()` failure must NOT mask a successful refresh.
    /// The previous `release().await?` propagated `Repo(...)` and would
    /// trigger `record_failure` → another IdP POST → invalidates the
    /// just-issued refresh token (n8n #13088 lineage). With the
    /// scopeguard-based release, errors are logged not propagated.
    #[tokio::test]
    async fn release_failure_after_successful_refresh_returns_ok() {
        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(FlakyReleaseRepo { inner });
        let coord = RefreshCoordinator::new_with(
            repo,
            ReplicaId::new("test"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid");

        let cid = CredentialId::new();
        let result: Result<u32, RefreshError> = coord
            .refresh_coalesced(&cid, |_id| async { true }, |_claim| async move { Ok(42) })
            .await;

        assert_eq!(
            result.unwrap(),
            42,
            "release failure must NOT mask successful refresh result"
        );
    }

    /// C1 regression — sub-spec §3.4. If the user closure panics, the
    /// coordinator MUST still abort the heartbeat task and release the
    /// L2 claim row. Without the scopeguard, the heartbeat ticks
    /// forever, the row stays held, and Stage 3.3 reclaim cannot
    /// recover it.
    ///
    /// Test strategy: run the panicking closure under
    /// `AssertUnwindSafe::catch_unwind`, give the detached release task
    /// a moment to flush, then verify a fresh `try_claim` succeeds —
    /// i.e. the row is releasable, not held by a phantom heartbeat.
    #[tokio::test]
    async fn user_closure_panic_releases_l2_claim() {
        use std::panic::AssertUnwindSafe;

        use futures::FutureExt;

        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let coord = Arc::new(
            RefreshCoordinator::new_with(
                Arc::clone(&repo),
                ReplicaId::new("panic-test"),
                RefreshCoordConfig::default(),
            )
            .expect("default config valid"),
        );
        let cid = CredentialId::new();

        let coord_for_panic = Arc::clone(&coord);
        let cid_for_panic = cid;
        let panic_result = AssertUnwindSafe(async move {
            let _: Result<i32, RefreshError> = coord_for_panic
                .refresh_coalesced(
                    &cid_for_panic,
                    |_id| async { true },
                    |_claim| async move {
                        panic!("test panic");
                        #[allow(unreachable_code)]
                        Ok::<i32, RefreshError>(0)
                    },
                )
                .await;
        })
        .catch_unwind()
        .await;
        assert!(
            panic_result.is_err(),
            "user closure panic must propagate out of refresh_coalesced"
        );

        // Give the detached release-on-drop spawn a moment to land.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // The L2 row must be releasable: a fresh `try_claim` from a
        // different replica should succeed immediately. If the
        // scopeguard had not fired, the row would still be heartbeated
        // and we'd see `Contended` here.
        let attempt = repo
            .try_claim(&cid, &ReplicaId::new("recoverer"), Duration::from_secs(5))
            .await
            .expect("try_claim must not error");
        assert!(
            matches!(attempt, ClaimAttempt::Acquired(_)),
            "panic must not leave the L2 row held — got {attempt:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Sub-spec §6 — observability emission
    // ──────────────────────────────────────────────────────────────────

    /// `refresh_coalesced` must increment `claims_acquired` exactly once
    /// per successful run and observe a hold-duration sample. Pre-bound
    /// metric handles are shared between coordinator and tests via
    /// `with_metrics`, so we can read them post-run without poking
    /// internals.
    #[tokio::test]
    async fn refresh_coalesced_increments_acquired_and_observes_hold_duration() {
        use nebula_metrics::MetricsRegistry;

        let registry = MetricsRegistry::new();
        let metrics_handle = RefreshCoordMetrics::with_registry(&registry);
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let coord = RefreshCoordinator::new_with(
            repo,
            ReplicaId::new("metrics-test"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid")
        .with_metrics(metrics_handle.clone());

        let cid = CredentialId::new();
        let _: u32 = coord
            .refresh_coalesced(&cid, |_| async { true }, |_claim| async move { Ok(7) })
            .await
            .expect("ok");

        assert_eq!(
            metrics_handle.claims_acquired.get(),
            1,
            "claims_acquired must tick once per successful run"
        );
        assert_eq!(
            metrics_handle.claims_contended.get(),
            0,
            "no contention in this single-caller test"
        );
        assert_eq!(
            metrics_handle.hold_duration.count(),
            1,
            "hold_duration must observe exactly one sample"
        );
    }

    /// L2 backoff resolved by the post-backoff state recheck: the
    /// caller must surface as `coalesced_l2` exactly once.
    #[tokio::test]
    async fn refresh_coalesced_increments_coalesced_l2_when_recheck_short_circuits() {
        use nebula_metrics::MetricsRegistry;

        let registry = MetricsRegistry::new();
        let metrics_handle = RefreshCoordMetrics::with_registry(&registry);
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());

        // Park a contender claim so try_claim returns Contended for the
        // unit-under-test caller. TTL is short so the contender
        // expires before we exhaust attempts.
        let parked_holder = ReplicaId::new("contender");
        let _parked = match repo
            .try_claim(
                &CredentialId::nil(),
                &parked_holder,
                Duration::from_millis(150),
            )
            .await
            .expect("park ok")
        {
            ClaimAttempt::Acquired(c) => c,
            ClaimAttempt::Contended { .. } => panic!("setup must always acquire"),
        };

        let coord = RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("coalesce-l2-test"),
            RefreshCoordConfig {
                claim_ttl: Duration::from_secs(2),
                heartbeat_interval: Duration::from_millis(500),
                refresh_timeout: Duration::from_millis(500),
                reclaim_sweep_interval: Duration::from_millis(500),
                sentinel_threshold: 3,
                sentinel_window: Duration::from_hours(1),
            },
        )
        .expect("custom config valid")
        .with_metrics(metrics_handle.clone());

        // Use the parked credential so the second caller hits the
        // contended path. After backoff, the recheck predicate returns
        // `false` so the coordinator surfaces CoalescedByOtherReplica.
        let cid = CredentialId::nil();
        let outcome: Result<i32, RefreshError> = coord
            .refresh_coalesced(&cid, |_| async { false }, |_| async { Ok(0) })
            .await;
        assert!(
            matches!(outcome, Err(RefreshError::CoalescedByOtherReplica)),
            "expected CoalescedByOtherReplica; got {outcome:?}"
        );
        assert_eq!(
            metrics_handle.coalesced_l2.get(),
            1,
            "post-backoff recheck false must increment coalesced_l2"
        );
        assert!(
            metrics_handle.claims_contended.get() >= 1,
            "at least one Contended attempt must be counted"
        );
    }
}
