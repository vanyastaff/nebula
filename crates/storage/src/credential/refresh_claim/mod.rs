//! Durable cross-replica claim repository for credential refresh
//! coordination.
//!
//! Per ADR-0041 + sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`.
//!
//! Implementations of `RefreshClaimRepo` provide CAS-based claim
//! acquisition with TTL + heartbeat semantics. Mirrors the control-queue
//! claim pattern (ADR-0008 + ADR-0017).

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use uuid::Uuid;

mod in_memory;
pub use in_memory::InMemoryRefreshClaimRepo;

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteRefreshClaimRepo;

#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PgRefreshClaimRepo;

/// Stable identifier for a Nebula replica process. Used to distinguish
/// claim holders for diagnostics + sweep ownership.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ReplicaId(String);

impl ReplicaId {
    /// Construct a new replica id from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the replica id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque token returned to the holder after `RefreshClaimRepo::try_claim`
/// succeeds. Carries an internal generation counter so heartbeats from a
/// stale holder cannot extend a reclaimed claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimToken {
    /// Per-claim UUID stamped on acquisition.
    pub claim_id: Uuid,
    /// Bumped each time the row is overwritten on reclaim. Heartbeats from
    /// an older generation are rejected with `HeartbeatError::ClaimLost`.
    pub generation: u64,
}

/// Successful claim record returned by `RefreshClaimRepo::try_claim`.
#[derive(Clone, Debug)]
pub struct RefreshClaim {
    /// The credential the claim is held against.
    pub credential_id: CredentialId,
    /// Holder-side proof of ownership (passed to heartbeat / release).
    pub token: ClaimToken,
    /// Wall-clock time the claim was acquired.
    pub acquired_at: DateTime<Utc>,
    /// Wall-clock time the claim TTL expires unless heartbeat-extended.
    pub expires_at: DateTime<Utc>,
}

/// Result of a `RefreshClaimRepo::try_claim` call.
#[derive(Debug)]
pub enum ClaimAttempt {
    /// Caller acquired the claim.
    Acquired(RefreshClaim),
    /// Another holder has a valid claim. The `existing_expires_at` lets
    /// the caller back off until that moment.
    Contended {
        /// When the existing claim is expected to expire (caller backoff hint).
        existing_expires_at: DateTime<Utc>,
    },
}

/// Errors from `RefreshClaimRepo::heartbeat`.
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// Our claim expired and another replica took it. Discard and re-check
    /// state.
    #[error("claim lost — another holder took ownership")]
    ClaimLost,
    /// Underlying repo error (DB connectivity etc.).
    #[error("repo error: {0}")]
    Repo(#[from] RepoError),
}

/// Errors from `RefreshClaimRepo::try_claim`, `release`, or
/// `reclaim_stuck`.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// Storage backend error (sqlx). Only present when a SQL backend is
    /// compiled in.
    #[cfg(any(feature = "postgres", feature = "sqlite"))]
    #[error("storage error: {0}")]
    Storage(#[from] sqlx::Error),
    /// Invariant violation observed in the repo (bad TTL, missing row
    /// after CAS lost, decode failure, etc.).
    #[error("invalid state: {0}")]
    InvalidState(String),
}

/// Sentinel mark applied to an in-flight refresh row, per
/// sub-spec §3.4. Cleared on successful release; if found set
/// during reclaim sweep, the holder is presumed crashed mid-refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentinelState {
    /// Normal claim — no IdP call yet OR already complete.
    Normal,
    /// Holder has started the IdP POST but not yet released.
    RefreshInFlight,
}

/// One row returned by `RefreshClaimRepo::reclaim_stuck`.
#[derive(Debug, Clone)]
pub struct ReclaimedClaim {
    /// Credential whose stale claim was just released by the sweep.
    pub credential_id: CredentialId,
    /// Replica that previously held the claim (now presumed gone).
    pub previous_holder: ReplicaId,
    /// Generation of the previous holder's claim (monotonically increasing).
    pub previous_generation: u64,
    /// Sentinel state observed at sweep time (`RefreshInFlight` indicates
    /// a presumed mid-refresh crash; engine emits a sentinel event).
    pub sentinel: SentinelState,
}

/// Cross-replica claim repository.
///
/// Per ADR-0041 + sub-spec §3.2. Implementations MUST:
///
/// - Provide atomic CAS semantics on `try_claim` (one acquirer wins when multiple replicas attempt
///   simultaneously).
/// - Validate holder/generation on `heartbeat` (a stale token cannot extend a reclaimed claim).
/// - Idempotent `release`.
/// - Reclaim sweep returns the credentials whose stale claims were re-acquired (parallel to
///   control-queue reclaim cadence).
#[async_trait::async_trait]
pub trait RefreshClaimRepo: Send + Sync + 'static {
    /// Try to acquire a refresh claim for `credential_id` on behalf of
    /// `holder`. Returns `Acquired` on success, `Contended` if another
    /// holder owns a non-expired claim.
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError>;

    /// Extend the TTL of an existing claim by `ttl`, replacing the previous
    /// `expires_at` with `now + ttl`. The caller (typically
    /// `RefreshCoordinator::spawn_heartbeat`) MUST pass the same TTL it used
    /// for `try_claim` so the heartbeat / sweep invariants from sub-spec §3.5
    /// hold (`heartbeat_interval × 3 < claim_ttl`,
    /// `reclaim_sweep_interval ≤ claim_ttl`).
    ///
    /// Fails with `ClaimLost` if the holder's token has been superseded
    /// (generation bumped) or the claim has been released.
    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError>;

    /// Release a claim. Idempotent — a token that no longer matches a row
    /// is treated as already-released.
    async fn release(&self, token: ClaimToken) -> Result<(), RepoError>;

    /// Marks the claim as `RefreshInFlight` — called immediately before
    /// the IdP POST. Idempotent.
    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError>;

    /// Sweeps claims past TTL, returns reclaimed credential ids paired
    /// with the sentinel state observed (so caller can record events for
    /// `RefreshInFlight` cases).
    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError>;

    /// Records a sentinel event into `credential_sentinel_events`. Called
    /// by the reclaim sweep when an expired claim's sentinel column is
    /// `RefreshInFlight` — meaning the holder crashed mid-refresh.
    ///
    /// One event per detected crash; the threshold logic
    /// (`count_sentinel_events_in_window`) lives in `nebula-engine`.
    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError>;

    /// Counts sentinel events for `credential_id` whose `detected_at` is
    /// strictly after `window_start`. Used by `SentinelTrigger` to apply
    /// the rolling-window N-events-in-1h threshold per sub-spec §3.4.
    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError>;
}
