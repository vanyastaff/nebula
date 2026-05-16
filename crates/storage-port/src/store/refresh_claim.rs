//! Cross-replica refresh-claim store (ADR-0041 + refresh-coordination
//! sub-spec).
//!
//! Re-homed from the adapter **shape-unchanged**: this component is
//! loom-verified, so its trait surface and supporting types are preserved
//! exactly. The only difference from the adapter's former definition is that
//! the backend-error variant carries a `String` instead of `sqlx::Error`
//! (the port has no sqlx; the adapter maps `sqlx::Error` → this at the
//! edge). The CAS / heartbeat / sentinel / reclaim invariants are identical.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use uuid::Uuid;

/// Stable identifier for a Nebula replica process. Bounded length so it
/// cannot bloat audit-event payloads or span attributes.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ReplicaId(String);

impl ReplicaId {
    /// Maximum byte length stored on a `ReplicaId`. Longer inputs are
    /// truncated at the nearest UTF-8 boundary by [`ReplicaId::new`].
    pub const MAX_BYTES: usize = 256;

    /// Construct a replica id, truncating oversized input at a UTF-8
    /// boundary (truncation is preferred over panic so a misbehaving
    /// caller cannot crash the engine).
    pub fn new(id: impl Into<String>) -> Self {
        let mut s: String = id.into();
        if s.len() > Self::MAX_BYTES {
            let mut cap = Self::MAX_BYTES;
            while !s.is_char_boundary(cap) {
                cap -= 1;
            }
            s.truncate(cap);
        }
        Self(s)
    }

    /// Borrow the replica id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque proof of claim ownership. Carries a generation so a stale
/// holder's heartbeat cannot extend a reclaimed claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimToken {
    /// Per-claim UUID stamped on acquisition.
    pub claim_id: Uuid,
    /// Bumped each time the row is overwritten on reclaim.
    pub generation: u64,
}

/// A successfully acquired refresh claim.
#[derive(Clone, Debug)]
pub struct RefreshClaim {
    /// Credential the claim is held against.
    pub credential_id: CredentialId,
    /// Holder-side proof of ownership.
    pub token: ClaimToken,
    /// When the claim was acquired.
    pub acquired_at: DateTime<Utc>,
    /// When the claim TTL expires unless heartbeat-extended.
    pub expires_at: DateTime<Utc>,
}

/// Result of `RefreshClaimStore::try_claim`.
#[derive(Debug)]
pub enum ClaimAttempt {
    /// Caller acquired the claim.
    Acquired(RefreshClaim),
    /// Another holder owns a valid claim.
    Contended {
        /// When the existing claim is expected to expire (backoff hint).
        existing_expires_at: DateTime<Utc>,
    },
}

/// Errors from `RefreshClaimStore::heartbeat`.
///
/// Variant names are preserved verbatim from the loom-verified
/// `RefreshClaimRepo` this trait re-homes (spec §4.2: shape unchanged) —
/// `Repo` (not `Store`) so existing `match HeartbeatError::Repo(_)` arms
/// in consumers remain valid across the move.
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// Our claim expired and another replica took it.
    #[error("claim lost — another holder took ownership")]
    ClaimLost,
    /// Underlying store error.
    #[error("store error: {0}")]
    Repo(#[from] RefreshClaimError),
}

/// Errors from `try_claim` / `release` / `reclaim_stuck` / sentinel ops.
#[derive(Debug, thiserror::Error)]
pub enum RefreshClaimError {
    /// Backend failure (the adapter maps its driver error into this).
    #[error("storage error: {0}")]
    Storage(String),
    /// Invariant violation observed in the store (bad TTL, missing row
    /// after CAS lost, decode failure, etc.).
    #[error("invalid state: {0}")]
    InvalidState(String),
}

/// Sentinel mark applied to an in-flight refresh row (sub-spec §3.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentinelState {
    /// Normal claim — no IdP call yet OR already complete.
    Normal,
    /// Holder has started the IdP POST but not yet released.
    RefreshInFlight,
}

/// One row returned by `reclaim_stuck`.
#[derive(Debug, Clone)]
pub struct ReclaimedClaim {
    /// Credential whose stale claim was released by the sweep.
    pub credential_id: CredentialId,
    /// Replica that previously held the claim (now presumed gone).
    pub previous_holder: ReplicaId,
    /// Generation of the previous holder's claim.
    pub previous_generation: u64,
    /// Sentinel state observed at sweep time.
    pub sentinel: SentinelState,
}

/// Cross-replica claim store (ADR-0041 §3.2). Shape preserved verbatim from
/// the loom-verified adapter trait.
#[async_trait::async_trait]
pub trait RefreshClaimStore: Send + Sync + 'static {
    /// Try to acquire a refresh claim for `credential_id` on behalf of
    /// `holder`.
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RefreshClaimError>;

    /// Extend the TTL of an existing claim, replacing `expires_at` with
    /// `now + ttl`. Fails with `ClaimLost` if the token was superseded.
    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError>;

    /// Release a claim (idempotent).
    async fn release(&self, token: ClaimToken) -> Result<(), RefreshClaimError>;

    /// Mark the claim `RefreshInFlight` immediately before the IdP POST.
    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RefreshClaimError>;

    /// Sweep claims past TTL; returns reclaimed credentials + sentinel
    /// state observed.
    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RefreshClaimError>;

    /// Record a sentinel event into `credential_sentinel_events`.
    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RefreshClaimError>;

    /// Count sentinel events for `credential_id` whose `detected_at` is
    /// strictly after `window_start`.
    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RefreshClaimError>;
}
