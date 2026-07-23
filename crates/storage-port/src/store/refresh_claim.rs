//! Cross-replica refresh-claim store (CAS lease, heartbeat, sentinel reclaim).
//!
//! See `docs/INTEGRATION_MODEL.md` (credential refresh) for integration context.
//!
//! The original acquisition CAS shape remains loom-verified. The public result
//! model is deliberately stricter than the historical adapter: an expired
//! `RefreshInFlight` row is durable poison, and reclaim accounting is a single
//! atomic port operation so evidence cannot be overwritten or recorded twice.
//! Errors are a closed, payload-free taxonomy so driver diagnostics and
//! persisted identifiers cannot cross the adapter boundary.

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
    /// A previous holder crossed the provider side-effect boundary and its
    /// claim expired before the outcome was durably resolved.
    ///
    /// This is a persistent, fail-closed poison state. Callers must not retry
    /// provider egress. Only an explicit future reconciliation command may
    /// clear the retained claim.
    OutcomeUnknown {
        /// When the poisoned claim expired.
        expired_at: DateTime<Utc>,
    },
}

/// Errors from `RefreshClaimStore::heartbeat`.
///
/// `Repo` (not `Store`) is retained for source compatibility with existing
/// consumer match arms.
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// Our claim expired and another replica took it.
    #[error("claim lost — another holder took ownership")]
    ClaimLost,
    /// Underlying store error.
    #[error("store error: {0}")]
    Repo(#[from] RefreshClaimError),
}

/// Errors from claim acquisition, release, reclaim accounting, and sentinel
/// transitions.
#[derive(Debug, thiserror::Error)]
pub enum RefreshClaimError {
    /// Backend operation or decoding failed. Driver diagnostics stay inside
    /// the adapter and are never rendered through this public error.
    #[error("refresh claim storage unavailable")]
    Storage,
    /// The adapter observed an invalid claim state or argument.
    #[error("refresh claim state is invalid")]
    InvalidState,
}

/// Sentinel mark applied to an in-flight refresh row (sub-spec §3.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentinelState {
    /// Normal claim — no IdP call yet OR already complete.
    Normal,
    /// Holder has started the IdP POST but not yet released.
    RefreshInFlight,
}

/// One newly-accounted result returned by [`RefreshClaimStore::reclaim_stuck`].
///
/// The variants are structural: a normal expiry is released, while an
/// in-flight expiry is retained as durable poison after its sentinel evidence
/// is recorded exactly once.
#[derive(Debug, Clone)]
pub enum ExpiredClaim {
    /// A claim that expired before provider egress and was deleted.
    ReclaimedNormal {
        /// Credential whose stale claim was released.
        credential_id: CredentialId,
        /// Replica that previously held the claim.
        previous_holder: ReplicaId,
        /// Generation of the previous holder's claim.
        previous_generation: u64,
    },
    /// A claim that expired after provider egress began. Its evidence was
    /// durably recorded while the claim row remained as fail-closed poison.
    OutcomeUnknownAccounted {
        /// Credential retained in the poisoned claim row.
        credential_id: CredentialId,
        /// Replica whose provider outcome is unknown.
        previous_holder: ReplicaId,
        /// Generation whose provider outcome is unknown.
        previous_generation: u64,
    },
}

/// Cross-replica claim store.
///
/// Acquisition keeps the loom-verified single-winner CAS semantics. Reclaim
/// is deliberately stronger: poison accounting and claim retention form one
/// atomic operation exposed through this port.
#[async_trait::async_trait]
pub trait RefreshClaimStore: Send + Sync + 'static {
    /// Try to acquire a refresh claim for `credential_id` on behalf of
    /// `holder`. A missing row or expired [`SentinelState::Normal`] row can
    /// be acquired. An expired [`SentinelState::RefreshInFlight`] row returns
    /// [`ClaimAttempt::OutcomeUnknown`] and remains durable fail-closed poison.
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RefreshClaimError>;

    /// Extend the TTL of an existing claim, replacing `expires_at` with
    /// `now + ttl`. Fails with `ClaimLost` if the token was superseded.
    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError>;

    /// Release the exact token's claim (idempotent).
    ///
    /// There are exactly two release authorities: pre-provider cleanup, when
    /// the provider closure has not started, and exact `Confirmed`
    /// finalization, including after lease expiry. `RetryUnsafe`,
    /// `OutcomeUnknown`, and cancelled provider paths must retain the token;
    /// the store cannot infer side-effect certainty from the token alone.
    async fn release(&self, token: ClaimToken) -> Result<(), RefreshClaimError>;

    /// Mark the claim `RefreshInFlight` immediately before the IdP POST.
    /// The token must still identify an unexpired claim; an expired token
    /// cannot authorize provider egress.
    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RefreshClaimError>;

    /// Account claims past TTL in one atomic storage boundary.
    ///
    /// Expired Normal rows are deleted and returned as
    /// [`ExpiredClaim::ReclaimedNormal`]. Expired `RefreshInFlight` rows are
    /// never deleted: their sentinel event is inserted idempotently under the
    /// claim-row lock, keyed by the globally unique claim UUID, and only a
    /// newly recorded event is returned as
    /// [`ExpiredClaim::OutcomeUnknownAccounted`]. If accounting fails, neither
    /// the event nor claim state is partially advanced.
    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RefreshClaimError>;

    /// Count sentinel events for `credential_id` strictly inside `window`.
    ///
    /// SQL adapters derive the cutoff from the same database clock that
    /// authors `detected_at`; callers provide a duration, never a
    /// replica-clock timestamp.
    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window: Duration,
    ) -> Result<u32, RefreshClaimError>;
}

#[cfg(test)]
mod tests {
    use super::RefreshClaimError;

    #[test]
    fn refresh_claim_errors_are_closed_and_payload_free() {
        for (error, expected) in [
            (
                RefreshClaimError::Storage,
                "refresh claim storage unavailable",
            ),
            (
                RefreshClaimError::InvalidState,
                "refresh claim state is invalid",
            ),
        ] {
            let category = match &error {
                RefreshClaimError::Storage => "storage",
                RefreshClaimError::InvalidState => "invalid_state",
            };
            assert!(!category.is_empty());
            assert_eq!(error.to_string(), expected);
        }
    }
}
