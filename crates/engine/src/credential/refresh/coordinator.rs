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
use tokio_util::sync::CancellationToken;
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
}

impl RefreshCoordConfig {
    /// Verify the per-§3.5 interlocking invariants.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::*` whose variant names which invariant the
    /// configuration violates. Returns `ConfigError::Overflow` if any of
    /// the intermediate `Duration` arithmetic (e.g. `heartbeat_interval × 3`,
    /// `refresh_timeout + 2 × heartbeat_interval`) overflows `Duration::MAX`
    /// — the canonical fix is to lower the offending knob.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // `Duration::checked_mul` and `checked_add` return `None` on
        // overflow rather than panicking — surface that as a typed
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
    /// Background heartbeat task failed mid-refresh and could not extend
    /// the L2 claim. The user closure was aborted before issuing the IdP
    /// POST so a stale `refresh_token_v1` cannot be sent against an
    /// already-rotated row. Caller routes through `record_failure` and
    /// retries with a fresh L2 acquire.
    ///
    /// Distinct from `Heartbeat(...)`: that variant is the surface for
    /// the heartbeat error at construction; this variant signals the
    /// claim was lost while a refresh closure was already running.
    #[error(
        "L2 claim lost during refresh — heartbeat task failed before the IdP POST could complete"
    )]
    ClaimLostMidRefresh,
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
    /// Maximum number of consecutive non-`ClaimLost` heartbeat
    /// failures tolerated before the heartbeat task gives up and
    /// cancels the in-flight refresh (sub-spec §3.4 wave-4 fix).
    ///
    /// At three failures the worst-case latency before cancellation
    /// is `3 × heartbeat_interval`, which is bounded by the §3.5
    /// invariant `heartbeat_interval × 3 ≤ claim_ttl` — i.e. we
    /// never burn more than one TTL window absorbing transient
    /// noise. Not configurable: production tuning belongs in
    /// `RefreshCoordConfig` if a need emerges.
    const MAX_TRANSIENT_HEARTBEAT_FAILURES: u32 = 3;

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
        // Bootstrap: a fresh private registry so the coordinator is fully
        // functional without composition. Production callers MUST follow
        // up with `with_metrics(engine_registry)` so a scraper actually
        // observes the §6 series — see `with_metrics` rustdoc.
        Ok(Self {
            l1: L1RefreshCoalescer::new(),
            repo,
            replica_id,
            config,
            metrics: RefreshCoordMetrics::with_registry(&nebula_metrics::MetricsRegistry::new()),
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
        // Bootstrap: a fresh private registry so the coordinator is fully
        // functional without composition. Production callers MUST follow
        // up with `with_metrics(engine_registry)` so a scraper actually
        // observes the §6 series — see `with_metrics` rustdoc.
        Self {
            l1: L1RefreshCoalescer::new(),
            repo,
            replica_id: ReplicaId::new(default_replica_id_string()),
            config,
            metrics: RefreshCoordMetrics::with_registry(&nebula_metrics::MetricsRegistry::new()),
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
        // See `new_with` — the same bootstrap rule applies; production
        // composition MUST follow up with `with_metrics`.
        Ok(Self {
            l1: L1RefreshCoalescer::with_max_concurrent(max)?,
            repo,
            replica_id: ReplicaId::new(default_replica_id_string()),
            config,
            metrics: RefreshCoordMetrics::with_registry(&nebula_metrics::MetricsRegistry::new()),
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
    ///
    /// # Cancel-safety
    ///
    /// The `do_refresh` future MUST be cancel-safe: dropping it mid-`await`
    /// must not leak resources or leave external state in an inconsistent
    /// shape. The coordinator drops the future without notice if a heartbeat
    /// failure mid-refresh requires aborting before the IdP POST issues
    /// (returning [`RefreshError::ClaimLostMidRefresh`]).
    ///
    /// Standard async HTTP clients (`reqwest`, `hyper`) satisfy this. Closures
    /// that await a `tokio::task::JoinHandle` from `spawn_blocking` without a
    /// cleanup path do not — the blocking task continues running after the
    /// `JoinHandle` is dropped, and any state it produces is silently
    /// discarded.
    ///
    /// The coordinator itself is cancel-safe: dropping the
    /// `refresh_coalesced` future at any await point (`tokio::time::timeout`,
    /// `tokio::select!`, `JoinHandle::abort`) cancels the heartbeat
    /// task and best-effort releases the L2 claim row (spawned, fire-and-
    /// forget; release errors are logged at WARN). The synchronous-release
    /// contract on the success path is preserved — when `refresh_coalesced`
    /// returns `Ok(...)`, the L2 row has been awaited-released before the
    /// function returns.
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

        // Global rate-limit gate (audit B6 / wave-2 regression).
        //
        // Wave-2 introduced this typed entry point but silently bypassed
        // the L1 global concurrency semaphore (`refresh_semaphore`,
        // default 32 permits). Per-credential L1 coalescing alone does
        // not bound the case where many *distinct* credentials expire
        // near-simultaneously — e.g. on a daily TTL boundary or after
        // a replica restart with stale tokens — and a 200-credential
        // expiry burst would issue 200 concurrent IdP POSTs, recreating
        // the cascading-429 / refresh-storm pattern the cap is meant to
        // prevent. Only the legacy `String`-id path
        // (`resolver.rs::refresh_via_l1_only`) consumed permits, so
        // typed callers were unprotected.
        //
        // Acquired AFTER `try_refresh` (Winner-only — Waiters already
        // park on the oneshot above and do not need a permit) and BEFORE
        // L2 backoff so the bound covers the entire IdP POST window.
        // The order also keeps `_l1_complete` declared first so its
        // guard fires on every cancel/Drop path even if `acquire_permit`
        // itself is cancelled (its `await` is cancel-safe per
        // `L1RefreshCoalescer::acquire_permit` rustdoc — dropping the
        // future does not consume a permit).
        //
        // RAII: `_permit` holds an `OwnedSemaphorePermit` until end of
        // function (after explicit synchronous release on the success
        // path; after `l2_teardown` fires on every other path), so the
        // permit is released on every exit including Drop and panic.
        // Regression test
        // `refresh_coalesced_respects_global_concurrency_cap` proves
        // the cap gates concurrent typed refreshes (mutation: removing
        // this line makes the test observe `counter == 4` instead of
        // `2`).
        let _permit = self.l1.acquire_permit().await;

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

        // Cancellation token shared between heartbeat task and the user
        // closure. The heartbeat task fires `cancel()` on Err so
        // `do_refresh` can abort before the IdP POST runs against an
        // already-rotated row (n8n #13088 lineage).
        let cancel = CancellationToken::new();
        let hb_cancel = cancel.clone();

        // Heartbeat task in background.
        let hb_task = self.spawn_heartbeat(claim.token.clone(), hb_cancel, *credential_id);

        // Cancel-safety guard (review C1 + wave-5, sub-spec §3.4). Fires
        // on EVERY exit path that does NOT explicitly defuse it: panic
        // unwind, error early-return, AND Drop (caller cancels the
        // outer future via `tokio::time::timeout`, `tokio::select!`, or
        // `JoinHandle::abort`). Without Drop coverage, a caller that
        // wraps `refresh_coalesced` in its own timeout / select! /
        // spawn-and-drop would leak the heartbeat task (extending L2
        // expiry forever) and skip `release()` — the same "stuck claim"
        // failure mode the panic-only guard was designed to prevent,
        // just triggered by `Drop` instead of unwind.
        //
        // The success path defuses this guard with
        // `scopeguard::ScopeGuard::into_inner(...)` BEFORE running its
        // synchronous release — that preserves the synchronous-release
        // contract on `Ok(...)` while making every other exit do
        // best-effort spawned cleanup. `release()` is idempotent and
        // the spawned task is detached because Drop is synchronous; we
        // cannot `.await` here.
        //
        // Heartbeat shutdown order mirrors the success path (below):
        // `cancel.cancel()` first so the heartbeat exits through its
        // `cancelled()` arm cleanly, then `abort()` as a belt-and-suspenders
        // guarantee. `CancellationToken` is reference-counted so dropping
        // it does NOT auto-cancel — both paths must call `.cancel()`
        // explicitly to keep release semantics symmetric.
        let token_teardown = claim.token.clone();
        let repo_teardown = Arc::clone(&self.repo);
        let hb_cancel_teardown = cancel.clone();
        let hb_task_teardown = hb_task.abort_handle();
        let hold_duration_teardown = self.metrics.hold_duration.clone();
        let l2_teardown = scopeguard::guard((), move |()| {
            hb_cancel_teardown.cancel();
            hb_task_teardown.abort();
            // Hold-time histogram is recorded on every exit path —
            // observe it before the spawn so the histogram never drops
            // a sample regardless of which path triggered teardown.
            hold_duration_teardown.observe(hold_start.elapsed().as_secs_f64());
            tokio::spawn(async move {
                if let Err(e) = repo_teardown.release(token_teardown).await {
                    tracing::warn!(?e, "L2 claim release on drop/unwind failed");
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
        // Wrap in `select!` over `cancel.cancelled()` so a heartbeat
        // failure mid-refresh aborts the closure BEFORE it issues the
        // IdP POST — sub-spec §3.4 invariant.
        //
        // Bias order MUST poll `do_refresh_fut` first: if the future
        // resolves `Ok(...)` and the heartbeat task fires
        // `cancel.cancel()` in the same wake-cycle, biased-cancel-first
        // would silently drop the successful result and route through
        // `record_failure`, which would reissue an IdP POST against a
        // stale `refresh_token_v1` — re-introducing the n8n #13088
        // refresh-storm pattern P2 was designed to prevent. With
        // `do_refresh_fut` polled first, a ready future deterministically
        // wins; cancel only fires while the refresh future is still
        // suspended. The `select!` is wrapped INSIDE `timeout(...)` so
        // the timeout still bounds the overall wait.
        let timeout = self.config.refresh_timeout;
        let do_refresh_fut = do_refresh(claim);
        let result = tokio::time::timeout(timeout, async {
            tokio::select! {
                biased;
                r = do_refresh_fut => r,
                () = cancel.cancelled() => Err(RefreshError::ClaimLostMidRefresh),
            }
        })
        .await
        .map_err(|_elapsed| RefreshError::Timeout(timeout))
        .and_then(std::convert::identity);

        // Normal-exit release (review I1 + wave-5, sub-spec §3.4). We
        // DO NOT propagate release errors — propagating them with `?`
        // would mask a successful refresh: caller would observe
        // `RefreshError::Repo(...)`, route to `record_failure`, then
        // retry → ANOTHER IdP POST → invalidates the just-issued
        // refresh token (n8n #13088 spec lineage). Log at warn level
        // instead.
        //
        // `into_inner` defuses the teardown guard BEFORE the explicit
        // cleanup runs. Without this, the guard would also fire when
        // `l2_teardown` goes out of scope at end of function, racing
        // the synchronous release below (double-release; second one
        // sees a missing row → spurious WARN log). Defusing first
        // preserves the synchronous-release contract: when
        // `refresh_coalesced` returns `Ok(...)`, the L2 row has been
        // awaited-released before the function returns.
        scopeguard::ScopeGuard::into_inner(l2_teardown);
        // Cancel the heartbeat token before aborting so the task exits
        // through its `cancelled()` arm rather than racing the abort.
        cancel.cancel();
        hb_task.abort();
        // Sub-spec §6 — observe the hold duration on the normal-exit
        // release path. Symmetric with the teardown guard above.
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

    /// Spawn the background heartbeat task that refreshes the L2 claim
    /// TTL on a fixed interval. Per Stage 1 fix C2 the trait's
    /// `heartbeat(token, ttl)` takes the same TTL passed to
    /// `try_claim`, so the §3.5 invariants
    /// (`heartbeat_interval × 3 < claim_ttl`,
    /// `reclaim_sweep_interval ≤ claim_ttl`) hold across heartbeats.
    ///
    /// Exits and signals cancellation via the supplied
    /// [`CancellationToken`] in two cases:
    ///
    /// 1. **Claim lost** (`HeartbeatError::ClaimLost`): a different replica reclaimed the row
    ///    (generation bumped or row deleted). Cancellation fires immediately so the in-flight
    ///    refresh aborts BEFORE the IdP POST. Without this signal the closure would press on with a
    ///    stale `refresh_token_v1` and invalidate the just-rotated row (n8n #13088 lineage).
    ///
    /// 2. **Transient errors past budget**: any non-`ClaimLost` heartbeat error (e.g. transient
    ///    backend hiccup wrapped in `HeartbeatError::Repo`) retries up to
    ///    [`Self::MAX_TRANSIENT_HEARTBEAT_FAILURES`] times. Single transient hiccups are absorbed
    ///    silently so storage backpressure does not amplify into refresh storms. After the budget
    ///    is exhausted, cancellation fires.
    fn spawn_heartbeat(
        &self,
        token: ClaimToken,
        cancel: CancellationToken,
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
            // Burn the initial immediate tick — the claim was just
            // acquired and already has a fresh `expires_at`.
            ticker.tick().await;
            // Transient-failure budget per sub-spec §3.4 (wave-4 fix).
            // Resets on every successful heartbeat so a long-running
            // refresh can absorb intermittent backend noise without
            // cancelling. Only `HeartbeatError::ClaimLost` is treated
            // as immediate-cancel — that is the unambiguous "your
            // claim is gone" signal.
            let mut transient_failures: u32 = 0;
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        // Caller wound up its refresh — heartbeat exits
                        // cleanly, no signal to fire.
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
                                    "credential refresh heartbeat lost claim; cancelling concurrent \
                                     do_refresh before it issues the IdP POST"
                                );
                                // Fire cancel BEFORE breaking so the user
                                // closure sees `cancelled()` and aborts
                                // the IdP call.
                                cancel.cancel();
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
                                         budget; cancelling concurrent do_refresh before it issues \
                                         the IdP POST"
                                    );
                                    cancel.cancel();
                                    break;
                                }
                                // Log at WARN — single hiccups are
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
                                // Continue — next ticker tick will retry.
                            }
                        }
                    }
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
    ///
    /// Only the standalone permit-grab API is deprecated; the underlying
    /// global concurrency semaphore is **not** going away. The typed-path
    /// `Self::refresh_coalesced` acquires from the same semaphore
    /// internally (Winner-only, RAII-scoped), so callers migrating off
    /// the legacy `String`-id surface inherit the rate-limit defense
    /// without any explicit wiring. See audit B6 / wave-2 regression.
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

    use crate::credential::refresh::test_fixtures::{
        AlwaysContendedRepo, AlwaysFailHeartbeatRepo, CountingHeartbeatRepo, FlakyReleaseRepo,
        SignallingFailHeartbeatRepo, TransientFailHeartbeatRepo,
    };

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

    // ──────────────────────────────────────────────────────────────────
    // Wave-3 review fixes
    // ──────────────────────────────────────────────────────────────────

    /// `validate()` MUST surface a typed `Overflow` error rather than
    /// panicking when intermediate `Duration` arithmetic overflows.
    /// Previously `heartbeat_interval * 3` panicked inside `validate()`
    /// for pathologically-large user inputs — defeating the very
    /// purpose of the validation step.
    #[test]
    fn validate_returns_overflow_instead_of_panicking() {
        // `Duration::MAX / 2` overflows `* 3` and `* 2` cleanly.
        let pathological = Duration::MAX / 2;
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(30),
            heartbeat_interval: pathological,
            refresh_timeout: Duration::from_secs(8),
            reclaim_sweep_interval: Duration::from_secs(30),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let err = cfg
            .validate()
            .expect_err("validate must reject pathologically-large heartbeat_interval");
        assert!(
            matches!(err, ConfigError::Overflow { .. }),
            "expected Overflow, got {err:?}"
        );
    }

    /// `validate()` overflow path also covers the
    /// `refresh_timeout + heartbeat_interval × 2` operand. Pick a small
    /// `heartbeat_interval` that fits `× 3` and `× 2` cleanly, then a
    /// huge `refresh_timeout` whose addition overflows.
    #[test]
    fn validate_returns_overflow_for_hold_budget_addition() {
        let cfg = RefreshCoordConfig {
            // Make claim_ttl Duration::MAX so heartbeat × 3 ≤ ttl
            // (otherwise we'd hit HeartbeatTooSlow before the addition
            // is even attempted).
            claim_ttl: Duration::MAX,
            heartbeat_interval: Duration::from_secs(1),
            // refresh_timeout near MAX so refresh_timeout +
            // (heartbeat_interval × 2) overflows.
            refresh_timeout: Duration::MAX
                .checked_sub(Duration::from_secs(1))
                .expect("MAX - 1s does not underflow"),
            reclaim_sweep_interval: Duration::from_secs(30),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let err = cfg
            .validate()
            .expect_err("validate must reject overflowing hold-budget addition");
        assert!(
            matches!(err, ConfigError::Overflow { .. }),
            "expected Overflow, got {err:?}"
        );
    }

    /// m2 — wave-3: contention exhaustion path. Use a repo wrapper
    /// that always returns `Contended` from `try_claim`. The
    /// `existing_expires_at` is set just past `now` so the per-attempt
    /// backoff is short (the helper sleeps until that timestamp,
    /// capped at 5s). After `MAX_ATTEMPTS = 5` the coordinator must
    /// surface `ContentionExhausted` and increment `claims_exhausted`.
    #[tokio::test]
    async fn refresh_coalesced_returns_contention_exhausted_after_max_attempts() {
        use nebula_metrics::MetricsRegistry;

        let registry = MetricsRegistry::new();
        let metrics_handle = RefreshCoordMetrics::with_registry(&registry);
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(AlwaysContendedRepo);

        let coord = RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("exhausted-test"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid")
        .with_metrics(metrics_handle.clone());

        // Predicate always says "still needs refresh" so we never
        // short-circuit through the L2 coalesce path.
        let cid = CredentialId::new();
        let outcome: Result<i32, RefreshError> = coord
            .refresh_coalesced(&cid, |_| async { true }, |_| async { Ok(0) })
            .await;
        assert!(
            matches!(outcome, Err(RefreshError::ContentionExhausted)),
            "expected ContentionExhausted; got {outcome:?}"
        );
        assert_eq!(
            metrics_handle.claims_exhausted.get(),
            1,
            "claims_exhausted must tick exactly once on retry exhaustion"
        );
        // Each retry's `try_claim` returned Contended, so we expect
        // MAX_ATTEMPTS contended ticks.
        assert_eq!(
            metrics_handle.claims_contended.get(),
            5,
            "every retry must contribute to claims_contended"
        );
    }

    /// M1 — wave-3: heartbeat task fails mid-refresh, cancellation
    /// token fires, user closure aborts with `ClaimLostMidRefresh`
    /// before its IdP POST equivalent runs.
    ///
    /// Strategy: build a repo that returns `HeartbeatError::ClaimLost`
    /// on every `heartbeat` call. The user closure sleeps long enough
    /// for the heartbeat ticker to fire at least once, then would have
    /// returned `Ok(...)` if not cancelled.
    #[tokio::test]
    async fn heartbeat_failure_cancels_concurrent_do_refresh() {
        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(AlwaysFailHeartbeatRepo { inner });

        // Tight heartbeat / refresh-timeout window so the heartbeat
        // tick fires inside the closure's pause without flapping.
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(10),
            heartbeat_interval: Duration::from_millis(50),
            refresh_timeout: Duration::from_secs(5),
            reclaim_sweep_interval: Duration::from_secs(5),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let coord = RefreshCoordinator::new_with(repo, ReplicaId::new("hb-test"), cfg)
            .expect("config valid");
        let cid = CredentialId::new();

        let result: Result<i32, RefreshError> = coord
            .refresh_coalesced(
                &cid,
                |_| async { true },
                |_claim| async move {
                    // Sleep longer than the heartbeat tick so the
                    // heartbeat fails and cancellation reaches us
                    // before we return Ok.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok::<i32, RefreshError>(123)
                },
            )
            .await;

        assert!(
            matches!(result, Err(RefreshError::ClaimLostMidRefresh)),
            "expected ClaimLostMidRefresh after heartbeat failure; got {result:?}"
        );
    }

    /// Tokio-semantics regression — sub-spec §3.4. Defense-in-depth
    /// for the production end-to-end test
    /// `refresh_coalesced_returns_ok_when_closure_ready_concurrent_with_cancel`
    /// below. This inline `select!` reproduces the exact biased-select
    /// shape used in `refresh_coalesced` and proves tokio's `biased`
    /// primitive picks the first-listed ready arm, not "any ready arm".
    /// If a future tokio release silently changed `biased` semantics,
    /// the production test could appear to pass by accident; this
    /// test would catch the upstream regression.
    ///
    /// Scope: tokio's `select! { biased; ... }` primitive — NOT the
    /// production code path. See the end-to-end test for that.
    #[tokio::test]
    async fn select_with_biased_refresh_first_lets_ready_future_win_pre_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel(); // pre-cancelled — worst case for the new bias
        let user_future = std::future::ready(Ok::<i32, RefreshError>(42));
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                biased;
                r = user_future => r,
                () = cancel.cancelled() => Err(RefreshError::ClaimLostMidRefresh),
            }
        })
        .await
        .expect("inner select must complete before timeout");

        assert!(
            matches!(result, Ok(42)),
            "biased order must let ready future win even when cancel is set; got {result:?}"
        );
    }

    /// M1 wave-4 reviewer-Issue-1 regression — sub-spec §3.4.
    /// End-to-end test through `refresh_coalesced` proving the
    /// production bias order picks `do_refresh_fut` over
    /// `cancel.cancelled()` when both arms are ready in the same
    /// wake-cycle. The sibling
    /// `select_with_biased_refresh_first_lets_ready_future_win_pre_cancelled`
    /// covers tokio's `biased` primitive in isolation; this test
    /// exercises the actual production code path so a future refactor
    /// that flips the bias order (or accidentally introduces a
    /// non-biased `select!`) is caught here.
    ///
    /// Strategy under `start_paused = true`:
    ///   1. `SignallingFailHeartbeatRepo::heartbeat()` calls `notify.notify_one()` AND THEN returns
    ///      `Err(ClaimLost)` in one synchronous body. Production's heartbeat task then calls
    ///      `cancel.cancel()` synchronously in the same task poll. Order: notify → main task
    ///      scheduled → cancel fires → main task already in queue (no-op).
    ///   2. The user closure awaits `notify.notified()` THEN returns `Ok(42)`. By the time it
    ///      resolves, `cancel` is set.
    ///   3. Main task re-polls `select!`. Both arms are ready — `do_refresh_fut` (Ok(42)) and
    ///      `cancel.cancelled()`. With the production bias `r = do_refresh_fut => r` listed first,
    ///      the ready closure wins → `Ok(42)`.
    ///
    /// Pre-fix bias (`cancel.cancelled()` first) would surface
    /// `ClaimLostMidRefresh` here — caller routes through
    /// `record_failure` → reissues IdP POST against stale
    /// `refresh_token_v1` (n8n #13088 lineage). Verified by mutation
    /// during construction: flipping the production bias order to
    /// cancel-first makes this test fail.
    #[tokio::test(start_paused = true)]
    async fn refresh_coalesced_returns_ok_when_closure_ready_concurrent_with_cancel() {
        use tokio::sync::Notify;

        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let heartbeat_called = Arc::new(Notify::new());
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(SignallingFailHeartbeatRepo {
            inner,
            heartbeat_called: Arc::clone(&heartbeat_called),
        });

        // validate()-passing config. `refresh_timeout` is generous so
        // it does NOT race with the heartbeat tick — under virtual
        // time, it never elapses because the closure resolves the
        // moment the heartbeat fixture fires `notify`. Heartbeat tick
        // at 10ms drives the test; the timeout at 5s is a safety net.
        //   heartbeat_interval × 3 = 30ms ≤ claim_ttl 6s ✓
        //   refresh_timeout + heartbeat × 2 = 5s + 20ms ≤ claim_ttl 6s ✓
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(6),
            heartbeat_interval: Duration::from_millis(10),
            refresh_timeout: Duration::from_secs(5),
            reclaim_sweep_interval: Duration::from_secs(5),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let coord = RefreshCoordinator::new_with(repo, ReplicaId::new("bias-e2e-test"), cfg)
            .expect("config valid");
        let cid = CredentialId::new();

        let notify_for_closure = Arc::clone(&heartbeat_called);
        let result: Result<i32, RefreshError> = coord
            .refresh_coalesced(
                &cid,
                |_| async { true },
                |_claim| async move {
                    // Wait for the heartbeat fixture to fire the notify.
                    // Production's heartbeat task then synchronously calls
                    // `cancel.cancel()` in the same task poll, so by the
                    // time the main task re-polls `select!`, BOTH arms
                    // are ready and the bias must pick this future.
                    notify_for_closure.notified().await;
                    Ok::<i32, RefreshError>(42)
                },
            )
            .await;

        assert!(
            matches!(result, Ok(42)),
            "biased select must let ready do_refresh future win over cancel in production code; \
             got {result:?}"
        );
    }

    /// M2 wave-4 regression — sub-spec §3.4. The heartbeat task MUST
    /// absorb up to `MAX_TRANSIENT_HEARTBEAT_FAILURES - 1` consecutive
    /// non-`ClaimLost` heartbeat errors without cancelling the
    /// in-flight refresh. Otherwise a 50ms DB hiccup amplifies into a
    /// refresh storm under storage backpressure.
    ///
    /// Strategy: `start_paused = true` makes the runtime drive time
    /// deterministically — `tokio::time::sleep` and `tokio::time::interval`
    /// auto-advance virtual time without wall-clock dependence. With
    /// `pattern = [true, true, false]` the heartbeat fails on ticks 1
    /// and 2 (counter → 1 → 2; below `MAX = 3`) and succeeds on tick 3
    /// (counter → 0). The closure resolves Ok; cancel never fires.
    /// Pre-fix (no budget), the first failure would cancel and the
    /// closure would surface `ClaimLostMidRefresh`.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_tolerates_transient_errors_within_budget() {
        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(TransientFailHeartbeatRepo::with_pattern(
            Arc::clone(&inner),
            // fail, fail, succeed — counter trajectory 1, 2, 0.
            // 2 < MAX_TRANSIENT_HEARTBEAT_FAILURES (3) so cancel does
            // NOT fire.
            vec![true, true, false],
        ));

        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(10),
            heartbeat_interval: Duration::from_millis(50),
            refresh_timeout: Duration::from_secs(5),
            reclaim_sweep_interval: Duration::from_secs(5),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let coord = RefreshCoordinator::new_with(repo, ReplicaId::new("transient-test"), cfg)
            .expect("config valid");
        let cid = CredentialId::new();

        let result: Result<i32, RefreshError> = coord
            .refresh_coalesced(
                &cid,
                |_| async { true },
                |_claim| async move {
                    // Sleep > 3 × heartbeat_interval (150ms) so ticks 1,
                    // 2, 3 all fire inside the closure. Under
                    // `start_paused`, this is virtual time — no
                    // wall-clock dependence.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    Ok::<i32, RefreshError>(7)
                },
            )
            .await;

        assert_eq!(
            result.expect("transient heartbeat errors within budget must NOT cancel refresh"),
            7,
            "user closure must complete despite 2 transient heartbeat failures"
        );
    }

    /// M2 wave-4 reviewer-Issue-4 — prove **reset-on-success** semantics.
    /// The previous test fails 2 ticks then succeeds, but does not
    /// distinguish "counter reset on success" from "counter is monotonic
    /// up to MAX". This sibling test asserts a second burst of 2
    /// failures AFTER an intervening success does NOT amplify into
    /// cancellation — i.e. the counter genuinely resets to zero on
    /// every successful heartbeat.
    ///
    /// Pattern: fail, fail, succeed, fail, fail, succeed.
    /// Counter trajectory: 1, 2, 0, 1, 2, 0 — never reaches MAX = 3.
    /// Without reset, trajectory would be 1, 2, 2, 3 → cancel on tick
    /// 4 → closure surfaces `ClaimLostMidRefresh`.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_resets_counter_on_success_after_partial_burst() {
        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(TransientFailHeartbeatRepo::with_pattern(
            Arc::clone(&inner),
            // fail, fail, succeed (resets), fail, fail, succeed.
            // Without reset-on-success the second burst would push the
            // counter to MAX and cancel.
            vec![true, true, false, true, true, false],
        ));

        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(10),
            heartbeat_interval: Duration::from_millis(50),
            refresh_timeout: Duration::from_secs(5),
            reclaim_sweep_interval: Duration::from_secs(5),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let coord = RefreshCoordinator::new_with(repo, ReplicaId::new("reset-test"), cfg)
            .expect("config valid");
        let cid = CredentialId::new();

        let result: Result<i32, RefreshError> = coord
            .refresh_coalesced(
                &cid,
                |_| async { true },
                |_claim| async move {
                    // Sleep > 6 × heartbeat_interval (300ms) so all 6
                    // ticks fire inside the closure under virtual time.
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok::<i32, RefreshError>(11)
                },
            )
            .await;

        assert_eq!(
            result.expect("reset-on-success must absorb a second transient burst"),
            11,
            "two bursts of transient failures separated by a success must NOT cancel refresh"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Wave-5 — coordinator cancel-safety on Drop
    // ──────────────────────────────────────────────────────────────────

    /// Wave-5 regression — sub-spec §3.4. Dropping the
    /// `refresh_coalesced` future mid-`await` (e.g. via a caller's
    /// `tokio::time::timeout`, `tokio::select!`, or `JoinHandle::abort`)
    /// MUST cancel the heartbeat task and best-effort release the L2
    /// claim row. Previously the panic-only `guard_on_unwind` did not
    /// fire on Drop, leaving the heartbeat task ticking forever and the
    /// L2 row held until TTL expiry — exactly the "stuck claim" failure
    /// mode the panic guard was designed to prevent, just triggered by
    /// `Drop` instead of unwind.
    ///
    /// Strategy under `start_paused = true`:
    ///   1. Wrap the in-memory repo in `CountingHeartbeatRepo` so we can observe heartbeat ticks
    ///      before and after Drop.
    ///   2. Spawn `refresh_coalesced` as a `JoinHandle` whose user closure parks on
    ///      `std::future::pending()` so the future is suspended.
    ///   3. Advance virtual time past several heartbeat intervals; record `count_before_drop`.
    ///   4. `handle.abort()` — drops the suspended future. With the wave-5 fix, the regular
    ///      `scopeguard::guard` (no `_on_unwind`) fires on Drop: `cancel.cancel()` +
    ///      `hb_task.abort()`
    ///      + spawned `repo.release()`.
    ///   5. Yield repeatedly so the spawned release task runs and the heartbeat task's
    ///      `cancelled()` arm wakes.
    ///   6. Advance virtual time again — far past several more heartbeat intervals — and yield.
    ///      With the fix, the heartbeat task is gone, so `count_after_drop == count_before_drop`.
    ///      Without the fix, the counter keeps climbing.
    ///   7. Assert the L2 row is released: a fresh `try_claim` from a different replica returns
    ///      `Acquired`. Without the fix, it would still be `Contended` (heartbeat keeps extending
    ///      the claim TTL).
    ///
    /// Mutation-test rationale: reverting just the
    /// `scopeguard::guard` → `scopeguard::guard_on_unwind` swap (and
    /// removing the matching `into_inner` defuse on the success path)
    /// makes both assertions fail — the heartbeat counter keeps
    /// climbing AND `try_claim` returns `Contended`. So the test
    /// structurally exercises the bug.
    #[tokio::test(start_paused = true)]
    async fn refresh_coalesced_drop_releases_l2_claim_and_stops_heartbeat() {
        use std::{
            future::pending,
            sync::atomic::{AtomicUsize, Ordering},
        };

        let inner: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let heartbeat_count = Arc::new(AtomicUsize::new(0));
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(CountingHeartbeatRepo {
            inner: Arc::clone(&inner),
            heartbeat_count: Arc::clone(&heartbeat_count),
        });

        // §3.5-validating config tuned for short virtual-time intervals:
        //   heartbeat_interval × 3 = 60ms ≤ claim_ttl 10s ✓
        //   refresh_timeout (5s) + heartbeat × 2 (40ms) ≤ claim_ttl 10s ✓
        // `refresh_timeout` is generous so the OUTER `refresh_coalesced`
        // does NOT race the timeout under virtual time — we want the
        // future to be SUSPENDED when we drop it, not resolved by
        // timeout.
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(10),
            heartbeat_interval: Duration::from_millis(20),
            refresh_timeout: Duration::from_secs(5),
            reclaim_sweep_interval: Duration::from_secs(5),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        let coord = Arc::new(
            RefreshCoordinator::new_with(Arc::clone(&repo), ReplicaId::new("drop-test"), cfg)
                .expect("config valid"),
        );
        let cid = CredentialId::new();

        // Spawn the refresh future. The user closure parks on
        // `pending()` so the future is suspended at the `select!` arm
        // when we abort the JoinHandle.
        let coord_for_task = Arc::clone(&coord);
        let cid_for_task = cid;
        let handle = tokio::spawn(async move {
            coord_for_task
                .refresh_coalesced(
                    &cid_for_task,
                    |_| async { true },
                    |_claim| async move {
                        // Park forever — only Drop on the outer future
                        // will end this. Without the wave-5 fix, the
                        // heartbeat task and L2 row leak when the outer
                        // future is dropped here.
                        pending::<Result<i32, RefreshError>>().await
                    },
                )
                .await
        });

        // Drive virtual time past several heartbeat intervals so the
        // heartbeat task has actually fired ticks before drop. Yielding
        // between advance steps lets the spawned heartbeat task run.
        for _ in 0..10 {
            tokio::time::advance(Duration::from_millis(20)).await;
            tokio::task::yield_now().await;
        }
        let count_before_drop = heartbeat_count.load(Ordering::Relaxed);
        assert!(
            count_before_drop >= 1,
            "heartbeat task must have ticked at least once before drop; got {count_before_drop}"
        );

        // Abort the JoinHandle — this drops the suspended
        // `refresh_coalesced` future. With the wave-5 fix, the
        // teardown guard fires: cancel + abort + spawned release.
        handle.abort();

        // Yield so the dropped future's destructor runs, the heartbeat
        // task's `cancelled()` arm wakes, and the spawned release task
        // gets scheduled. A small budget (50 yields) is plenty under
        // paused time — there are no real timers to wait on.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }

        // Advance virtual time WAY past several heartbeat intervals.
        // With the fix, the heartbeat task has exited, so no ticks
        // fire. Without the fix, the counter would climb by 50+ here.
        for _ in 0..50 {
            tokio::time::advance(Duration::from_millis(20)).await;
            tokio::task::yield_now().await;
        }

        let count_after_drop = heartbeat_count.load(Ordering::Relaxed);
        assert_eq!(
            count_after_drop, count_before_drop,
            "heartbeat task must STOP after refresh_coalesced is dropped; \
             before={count_before_drop} after={count_after_drop} (fix not applied?)"
        );

        // L2 row must be released — a fresh `try_claim` from a
        // different replica should win immediately. Without the fix,
        // the heartbeat would keep extending the row's `expires_at`
        // and `try_claim` would return `Contended`.
        let attempt = repo
            .try_claim(&cid, &ReplicaId::new("recoverer"), Duration::from_secs(5))
            .await
            .expect("try_claim must not error");
        assert!(
            matches!(attempt, ClaimAttempt::Acquired(_)),
            "drop must not leave the L2 row held — got {attempt:?}"
        );
    }

    /// B6 wave-2 regression — global concurrency cap MUST gate the
    /// typed `refresh_coalesced` path. Without the per-Winner permit
    /// acquisition, 200 distinct credentials expiring near-simultaneously
    /// would issue 200 concurrent IdP POSTs with no rate-limit defense
    /// — the cascading-429 / refresh-storm scenario the 32-permit cap
    /// (default) was designed to bound. Wave-2 silently bypassed this
    /// for the typed code path; only the legacy `String`-id path
    /// (`resolver.rs::refresh_via_l1_only`) consumed permits.
    ///
    /// Strategy under `start_paused = true`:
    ///   1. Build coordinator with `max_concurrent=2`.
    ///   2. Spawn 4 concurrent `refresh_coalesced` calls on different credentials. Each closure
    ///      increments an atomic counter, then parks on `Notify::notified()` (no time involvement
    ///      so virtual time does not advance and heartbeat ticks never fire during the assertion
    ///      windows).
    ///   3. Sleep 1ms (virtual) to let the runtime drive all 4 tasks to a parked state — two on
    ///      `notified()` after running the closure, two on the semaphore's `acquire_owned()`.
    ///   4. Assert counter == 2 — only the first two acquired permits.
    ///   5. Wake the parked closures; the first two complete and drop their permits, freeing the
    ///      semaphore for the remaining two.
    ///   6. Sleep 1ms again; assert counter == 4.
    ///
    /// Pre-fix (no `acquire_permit` in `refresh_coalesced`): all 4
    /// closures start immediately; counter reaches 4 before any notify
    /// fires; the first assertion fails with `observed 4 started`.
    /// Verified by mutation during construction — removing the
    /// `let _permit = self.l1.acquire_permit().await;` line causes
    /// this test to fail at the first assertion.
    #[tokio::test(start_paused = true)]
    async fn refresh_coalesced_respects_global_concurrency_cap() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use tokio::sync::Notify;

        let coord = Arc::new(
            RefreshCoordinator::with_max_concurrent(2).expect("max=2 is a valid concurrency limit"),
        );

        let started = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());

        let cids: Vec<CredentialId> = (0..4).map(|_| CredentialId::new()).collect();
        let mut handles = Vec::with_capacity(cids.len());

        for cid in &cids {
            let coord = Arc::clone(&coord);
            let started = Arc::clone(&started);
            let notify = Arc::clone(&notify);
            let cid = *cid;
            handles.push(tokio::spawn(async move {
                coord
                    .refresh_coalesced(
                        &cid,
                        |_| async { true },
                        move |_claim| async move {
                            started.fetch_add(1, Ordering::SeqCst);
                            notify.notified().await;
                            Ok::<i32, RefreshError>(7)
                        },
                    )
                    .await
            }));
        }

        // Sleep 1ms (virtual time) so the runtime drives all 4 spawned
        // tasks to a parked state. 1ms is well below the default
        // `heartbeat_interval` (10s), so no heartbeat tick fires during
        // the assertion windows.
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert_eq!(
            started.load(Ordering::SeqCst),
            2,
            "with max_concurrent=2, only two closures must run concurrently; \
             observed {} started",
            started.load(Ordering::SeqCst)
        );

        // Wake the two parked closures. They return `Ok(7)`, drop their
        // permits via RAII; the semaphore then wakes the two waiters
        // parked on `acquire_owned()`.
        notify.notify_waiters();
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert_eq!(
            started.load(Ordering::SeqCst),
            4,
            "after two permits released, the remaining two closures must run; \
             observed {} started",
            started.load(Ordering::SeqCst)
        );

        // Drain remaining tasks so the test exits cleanly.
        notify.notify_waiters();
        for handle in handles {
            let outcome = handle.await.expect("spawned task did not panic");
            assert_eq!(outcome.expect("refresh closure returned Ok"), 7);
        }
    }
}
