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

use std::{fmt, future::Future, sync::Arc, time::Duration};

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, RefreshClaim,
    RefreshClaimRepo, ReplicaId, RepoError,
};

use super::l1::{
    L1RefreshCoalescer, RefreshAttempt as L1Attempt, RefreshConfigError as L1ConfigError,
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
}

impl fmt::Debug for RefreshCoordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshCoordinator")
            .field("replica_id", &self.replica_id)
            .field("config", &self.config)
            .field("l1", &self.l1)
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
        })
    }

    /// Construct a default coordinator backed by an in-memory claim repo
    /// (suitable for tests and single-replica desktop mode).
    ///
    /// Production deployments must thread a real `RefreshClaimRepo`
    /// (Postgres or SQLite) through [`Self::new_with`].
    #[must_use]
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
        })
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

    /// Borrow the underlying claim repo (engine-internal — used by the
    /// reclaim sweep handle in Stage 3.3).
    #[allow(
        dead_code,
        reason = "consumed by the reclaim sweep landing in Stage 3.3"
    )]
    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
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
    /// # Errors
    ///
    /// See [`RefreshError`]. `CoalescedByOtherReplica` is success-with-side-effect:
    /// another replica refreshed while we were waiting. Caller should
    /// re-read the credential state and proceed.
    pub async fn refresh_coalesced<F, Fut, T>(
        &self,
        credential_id: &CredentialId,
        do_refresh: F,
    ) -> Result<T, RefreshError>
    where
        F: FnOnce(RefreshClaim) -> Fut,
        Fut: Future<Output = Result<T, RefreshError>>,
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
                // Fall through to acquire L2 + run the user closure.
            },
            super::l1::RefreshAttempt::Waiter(rx) => {
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
        let claim = self.try_acquire_l2_with_backoff(credential_id).await?;

        // Heartbeat task in background.
        let hb_task = self.spawn_heartbeat(claim.token.clone());

        // Run user's refresh closure under `refresh_timeout` per §3.5.
        // The timeout is shorter than the claim TTL by construction, so
        // the heartbeat keeps the L2 row alive while the closure runs.
        let timeout = self.config.refresh_timeout;
        let result = tokio::time::timeout(timeout, do_refresh(claim.clone()))
            .await
            .map_err(|_elapsed| RefreshError::Timeout(timeout))
            .and_then(std::convert::identity);

        // Stop heartbeat + release claim. Release is idempotent so even
        // if the heartbeat task already noticed `ClaimLost`, this is
        // safe to call.
        hb_task.abort();
        self.repo.release(claim.token).await?;

        result
    }

    /// L2 acquisition retry loop per sub-spec §3.6.
    async fn try_acquire_l2_with_backoff(
        &self,
        credential_id: &CredentialId,
    ) -> Result<RefreshClaim, RefreshError> {
        const MAX_ATTEMPTS: usize = 5;
        for _attempt in 0..MAX_ATTEMPTS {
            match self
                .repo
                .try_claim(credential_id, &self.replica_id, self.config.claim_ttl)
                .await?
            {
                ClaimAttempt::Acquired(claim) => return Ok(claim),
                ClaimAttempt::Contended {
                    existing_expires_at,
                } => {
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
                    // CRITICAL: Stage 3.1 will re-check credential state
                    // here; if another replica already refreshed,
                    // return Err(CoalescedByOtherReplica). Until then
                    // the caller's L1 path is the only fast-coalesce
                    // surface, so we just retry try_claim.
                },
            }
        }
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
    pub fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        self.l1.try_refresh(credential_id).into()
    }

    /// **Legacy.** Mark the L1 in-flight slot complete. Stage 2.3
    /// deletes this in favour of [`Self::refresh_coalesced`].
    pub fn complete(&self, credential_id: &str) {
        self.l1.complete(credential_id);
    }

    /// **Legacy.** Number of credentials currently being refreshed in
    /// the L1 layer.
    pub fn in_flight_count(&self) -> usize {
        self.l1.in_flight_count()
    }

    /// **Legacy.** Acquire a permit from the L1 concurrency limiter.
    pub async fn acquire_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.l1.acquire_permit().await
    }

    /// **Legacy.** Available permits in the L1 concurrency limiter.
    pub fn available_permits(&self) -> usize {
        self.l1.available_permits()
    }

    /// **Legacy.** Record a refresh failure for the L1 circuit breaker.
    pub fn record_failure(&self, credential_id: &str) {
        self.l1.record_failure(credential_id);
    }

    /// **Legacy.** Record a refresh success for the L1 circuit breaker.
    pub fn record_success(&self, credential_id: &str) {
        self.l1.record_success(credential_id);
    }

    /// **Legacy.** Whether the L1 per-credential circuit breaker is
    /// open.
    pub fn is_circuit_open(&self, credential_id: &str) -> bool {
        self.l1.is_circuit_open(credential_id)
    }
}

impl Default for RefreshCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

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
            .refresh_coalesced(&cid, |claim| async move {
                // Verify the claim is actually held — sentinel default
                // is Normal; expires_at should be in the future.
                assert!(claim.expires_at > chrono::Utc::now());
                Ok(42)
            })
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
}
