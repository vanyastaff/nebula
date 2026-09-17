//! Cross-replica refresh-claim store (CAS lease, heartbeat, sentinel reclaim).
//!
//! See `docs/INTEGRATION_MODEL.md` (credential refresh) for integration context.
//!
//! The original acquisition CAS shape remains loom-verified. The public result
//! model is deliberately stricter than the historical adapter: an expired
//! `RefreshInFlight` row is durable poison, and reclaim accounting is a single
//! atomic port operation so evidence cannot be overwritten or recorded twice.
//! Errors are a closed, payload-free taxonomy so driver diagnostics and
//! persisted identifiers cannot cross the adapter boundary. The one persisted
//! identifier allowed to cross is the recorded evidence digest, a SHA-256 that
//! is the durable half of the reconciliation retry identity: secret-free, so
//! it is exposed through [`RefreshAdjudication::evidence_digest`] and the
//! recorded pair on
//! [`RefreshClaimAdjudicationError::EvidenceConflict`] so a client that lost an
//! acknowledgement can confirm what is on record.
//!
//! # Writer inventory (sole-management-writer enforcement)
//!
//! This port is the only durable write surface for refresh-claim state, and
//! each write method has exactly one production authority. The parse-only
//! inventory test `crates/credential/tests/writer_inventory.rs` pins this
//! table over its parse sets — `crates/credential/src`, the three adapters,
//! the first-party composition files, and the engine surface; a second call
//! site for any method below inside those sets fails it.
//!
//! | Write method | Sole production authority |
//! |---|---|
//! | [`RefreshClaimStore::try_claim`] | `RefreshCoordinator::try_acquire_l2_with_backoff` |
//! | [`RefreshClaimStore::mark_sentinel`] | `RefreshCoordinator` (immediately pre-provider-egress) |
//! | [`RefreshClaimStore::heartbeat`] | the `RefreshCoordinator` lease heartbeat task (`spawn_heartbeat`) |
//! | [`RefreshClaimStore::release`] | the two `RefreshLease` authorities below (caller discipline; see the method's own docs) |
//! | [`RefreshClaimStore::reclaim_stuck`] | `reclaim::run_one_sweep`, spawned by `ReclaimSweepHandle` |
//! | [`RefreshClaimAdjudicator::adjudicate`] | `CredentialController::reconcile` — the sole management writer |
//!
//! The two `release` authorities share one boundary, stated as caller
//! discipline by design: the store cannot infer side-effect certainty from
//! the token alone. Both sit inside `RefreshLease`: `finish` serves both the
//! pre-provider cleanup sites and the exact post-provider state-disposition
//! sites (it is the disposition match that chooses the release arm), while
//! `Drop` releases only a pre-provider cancellation or task failure without a
//! disposition. See [`RefreshClaimStore::release`] for the retention
//! consequence of refusing the other paths.
//!
//! The seams by which a store or adjudicator handle could leave its authority,
//! each with a single consumer:
//!
//! - `RefreshCoordinator::repo()` is `pub(crate)`; its only consumer is
//!   `ReclaimSweepHandle::spawn`, which clones it into the sweep task.
//! - `CredentialRuntime.adjudicator` (first-party composition root) is
//!   `pub(crate)`; its only production clone-out feeds
//!   `CredentialController::new`, and the server gateway holds the
//!   `CredentialController`, never the raw adjudicator handle.
//! - `SentinelTrigger` holds its repo handle privately and reads only; it
//!   exposes no write path.

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
    /// provider egress. Only an explicit owner-qualified reconciliation
    /// command may clear the retained claim.
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
#[non_exhaustive]
pub enum RefreshClaimError {
    /// Backend operation or decoding failed. Driver diagnostics stay inside
    /// the adapter and are never rendered through this public error.
    #[error("refresh claim storage unavailable")]
    Storage,
    /// The adapter observed an invalid claim state or argument.
    #[error("refresh claim state is invalid")]
    InvalidState,
    /// The claim was not released: an unresolved sentinel incident for this
    /// claim is still on record, so the credential remains fail-closed poison
    /// until a [`RefreshClaimAdjudicator`] records its provider outcome.
    #[error("refresh claim release refused — an unresolved sentinel incident exists")]
    ReleaseRefused,
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

    /// Release the exact token's claim.
    ///
    /// There are exactly two release authorities: pre-provider cleanup, when
    /// the provider closure has not started, and exact `Confirmed`
    /// finalization. `RetryUnsafe`, `OutcomeUnknown`, and cancelled provider
    /// paths must retain the token; the store cannot infer side-effect
    /// certainty from the token alone.
    ///
    /// Those two authorities share one boundary, and the sweep moves it.
    /// Before [`RefreshClaimStore::reclaim_stuck`] has accounted the claim's
    /// incident, release with the exact token deletes the row, so a `Confirmed`
    /// finalization that lands after lease expiry still cleans up. Once the
    /// sweep has accounted it, the row is retained as poison and
    /// [`RefreshClaimError::ReleaseRefused`] is returned until an adjudication
    /// records the provider outcome. The delete's own predicate draws that
    /// boundary: it refuses any claim whose incident has `adjudicated_at IS
    /// NULL`, and only the sweep writes such an incident.
    ///
    /// Retention here is forced by this interface, not chosen for caution.
    /// `release(&self, token: ClaimToken)` carries no disposition, so the store
    /// cannot tell an exact `Confirmed` from an unknown outcome and must refuse
    /// both alike. Post-sweep the claim row is also the only anchor by which
    /// [`RefreshClaimAdjudicator::adjudicate`] reaches an unresolved incident,
    /// so deleting it would orphan that incident and no adjudication could ever
    /// close its accounting. The cost falls on the denied holder, which may
    /// hold an exact `Confirmed`, a positive proof of completion, and still
    /// lose the credential: adjudication is the only thing that lifts the
    /// denial, so until an operator records one the refusal is what the
    /// shipped product does with this state.
    ///
    /// Release is idempotent for a claim that carries no incident, but it is
    /// **not** unconditional: when an unresolved sentinel incident is on record
    /// for this claim id, the claim row is retained as poison and
    /// [`RefreshClaimError::ReleaseRefused`] is returned. Only
    /// [`RefreshClaimAdjudicator::adjudicate`] may clear that state.
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

/// What the provider side did for a refresh whose claim expired in flight.
///
/// The two variants are the whole decision space: the provider applied the
/// refresh or it did not. There is deliberately no "unknown" variant — an
/// unknown outcome is exactly the state being resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshOutcomeDecision {
    /// The provider applied the refresh; the credential's refreshed material
    /// is durable and the credential may be used again.
    ProviderApplied,
    /// The provider never applied the refresh; the previously stored material
    /// stands and the credential may be used again.
    ProviderNotApplied,
}

impl RefreshOutcomeDecision {
    /// The durable persisted spelling of this decision.
    ///
    /// This value is written to the `adjudication_decision` column, so it is a
    /// storage format and not a display name: renaming one strands every
    /// previously recorded row as undecodable by [`Self::from_wire`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderApplied => "provider_applied",
            Self::ProviderNotApplied => "provider_not_applied",
        }
    }

    /// Parse the persisted spelling back into a decision.
    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "provider_applied" => Some(Self::ProviderApplied),
            "provider_not_applied" => Some(Self::ProviderNotApplied),
            _ => None,
        }
    }
}

/// Recorded result of one reconciliation.
///
/// Read the fields by destructuring; the struct is [`non_exhaustive`] so a
/// future field addition cannot break downstream record literals, and the
/// error taxonomy's `Display` stays payload-free.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshAdjudication {
    /// The decision now on record for the credential.
    pub decision: RefreshOutcomeDecision,
    /// Whether this call recorded the decision.
    ///
    /// `false` means the identical `(evidence, decision)` pair was already on
    /// record for the same incident and this call was a no-op — the
    /// idempotent-recommit case. A second incident row is never written.
    pub changed: bool,
    /// SHA-256 of the evidence whose resolution is on record.
    ///
    /// The digest on record, returned in every success arm: on a recording
    /// call it is the digest of the evidence just stored; on the idempotent
    /// recommit the equality guard proved the recorded digest is byte-equal to
    /// the request's own. The durable half of the reconciliation retry
    /// identity, so a client holding its original evidence can confirm what is
    /// on record. The wire spelling is lowercase hex.
    pub evidence_digest: [u8; 32],
}

impl RefreshAdjudication {
    /// Construct a recorded reconciliation result.
    ///
    /// The only way a crate outside `nebula-storage-port` can build one, since
    /// the struct is [`non_exhaustive`].
    #[must_use]
    pub const fn new(
        decision: RefreshOutcomeDecision,
        changed: bool,
        evidence_digest: [u8; 32],
    ) -> Self {
        Self {
            decision,
            changed,
            evidence_digest,
        }
    }
}

/// Maximum byte length of an adjudication evidence note.
///
/// Evidence is an operator note, not a payload: it is digested and persisted
/// on the incident row, so an unbounded note would let a caller grow durable
/// rows without limit. Longer input is refused with
/// [`RefreshClaimAdjudicationError::InvalidEvidence`].
pub const MAX_ADJUDICATION_EVIDENCE_BYTES: usize = 4096;

/// Errors from [`RefreshClaimAdjudicator::adjudicate`].
///
/// Deliberately a separate taxonomy from [`RefreshClaimError`]: reconciliation
/// is a privileged operator decision, and its failure modes (nothing to
/// adjudicate, evidence conflict, lost acknowledgement) mean nothing to an
/// ordinary claim caller. Payload-free so driver diagnostics and persisted
/// identifiers cannot cross the adapter boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshClaimAdjudicationError {
    /// Backend operation, decoding, or commit failed. Driver diagnostics stay
    /// inside the adapter and are never rendered through this public error.
    #[error("refresh claim adjudication storage unavailable")]
    Storage,
    /// The decision write was dispatched but its acknowledgement was lost, so
    /// whether it committed is unknown. A recommit of the same evidence and
    /// decision is idempotent and is the only authorized follow-up.
    #[error("refresh claim adjudication outcome unknown")]
    AcknowledgementUnknown,
    /// `evidence` was empty or longer than
    /// [`MAX_ADJUDICATION_EVIDENCE_BYTES`].
    #[error("refresh claim adjudication evidence is invalid")]
    InvalidEvidence,
    /// The credential holds no poisoned claim and no recorded resolution, so
    /// there is nothing to adjudicate.
    #[error("refresh claim is not poisoned")]
    NotPoisoned,
    /// A resolution is already on record for the incident this claim resolves,
    /// and the recommitted evidence or decision differs from it.
    ///
    /// The payload stays out of `Display` — the taxonomy's payload-free
    /// discipline is unchanged — but the pair the comparison refused against
    /// is the one persisted identifier this port exposes: a client that lost
    /// the recorded identity cannot otherwise name what is on record. In the
    /// claim-keyed comparison it is the incident's own recorded resolution; in
    /// the resolved-set comparison it is the newest resolution on record for
    /// the credential.
    #[error("refresh claim adjudication conflicts with the recorded resolution")]
    EvidenceConflict {
        /// SHA-256 of the recorded evidence this request conflicted with.
        recorded_digest: [u8; 32],
        /// The recorded decision this request conflicted with.
        recorded_decision: RefreshOutcomeDecision,
    },
}

/// Privileged reconciliation of a poisoned refresh claim.
///
/// Deliberately a separate role from [`RefreshClaimStore`]: a refresh
/// coordinator holds no authority over a claim it no longer holds, and the
/// store cannot infer side-effect certainty. Deciding that an ambiguous
/// provider outcome is known is a human or operator decision, so it is
/// capability-gated, carries its own evidence note, and is recorded on the
/// sentinel incident it resolves.
#[async_trait::async_trait]
pub trait RefreshClaimAdjudicator: Send + Sync + 'static {
    /// Record `decision` as the resolution of `credential_id`'s poisoned
    /// claim, or of the resolution already on record for it.
    ///
    /// `evidence` is an operator-supplied, secret-free note recording *why*
    /// the outcome is now known — a provider support ticket, a reconciliation
    /// query result. It is digested and persisted so the decision is
    /// reviewable rather than anonymous.
    ///
    /// Clearing the poison and writing the resolution are one atomic
    /// operation: a call either leaves the claim acquirable *and* the
    /// resolution on record, or changes nothing.
    ///
    /// Repeating the same evidence and decision is idempotent and returns the
    /// recorded decision with [`RefreshAdjudication::changed`] `false` and the
    /// digest on record in [`RefreshAdjudication::evidence_digest`]. The same
    /// evidence with a different decision, or different evidence for an
    /// already-resolved incident, returns
    /// [`RefreshClaimAdjudicationError::EvidenceConflict`] — reconciliation
    /// resolves an unknown outcome, it does not overrule a recorded one.
    ///
    /// # Errors
    ///
    /// [`RefreshClaimAdjudicationError::NotPoisoned`] when the credential is
    /// neither poisoned nor already resolved;
    /// [`RefreshClaimAdjudicationError::InvalidEvidence`] when `evidence` is
    /// empty or over [`MAX_ADJUDICATION_EVIDENCE_BYTES`];
    /// [`RefreshClaimAdjudicationError::EvidenceConflict`] when the recommitted
    /// evidence or decision differs from the resolution already on record for
    /// the incident this claim resolves;
    /// [`RefreshClaimAdjudicationError::AcknowledgementUnknown`] when the
    /// commit was dispatched but its acknowledgement was lost.
    async fn adjudicate(
        &self,
        credential_id: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError>;
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_ADJUDICATION_EVIDENCE_BYTES, RefreshClaimAdjudicationError, RefreshClaimError,
        RefreshOutcomeDecision,
    };

    /// Every variant renders its own message.
    ///
    /// The `match` is the tripwire: it is exhaustive only from inside the
    /// crate, which is where a new variant is added, so a variant that arrives
    /// without an arm fails to compile here.
    #[test]
    fn every_refresh_claim_error_variant_renders_its_message() {
        for (error, expected) in [
            (
                RefreshClaimError::Storage,
                "refresh claim storage unavailable",
            ),
            (
                RefreshClaimError::InvalidState,
                "refresh claim state is invalid",
            ),
            (
                RefreshClaimError::ReleaseRefused,
                "refresh claim release refused — an unresolved sentinel incident exists",
            ),
        ] {
            match &error {
                RefreshClaimError::Storage => {},
                RefreshClaimError::InvalidState => {},
                RefreshClaimError::ReleaseRefused => {},
            }
            assert_eq!(error.to_string(), expected);
        }
    }

    /// Every variant renders its own message.
    #[test]
    fn every_adjudication_error_variant_renders_its_message() {
        // `#[non_exhaustive]` means the compiler cannot prove this list
        // complete from outside the crate; this in-crate match does, so a new
        // variant fails here until it is given a message.
        //
        // What that buys is a forced visit to the crate that defines the
        // variant, and no more. This module is `#[cfg(test)]`, so a lib-only
        // build compiles a sixth variant unmapped, and a consumer crate's
        // wildcard — or a mapping with no compile check at all — is outside
        // this tripwire's reach.
        for (error, expected) in [
            (
                RefreshClaimAdjudicationError::Storage,
                "refresh claim adjudication storage unavailable",
            ),
            (
                RefreshClaimAdjudicationError::AcknowledgementUnknown,
                "refresh claim adjudication outcome unknown",
            ),
            (
                RefreshClaimAdjudicationError::InvalidEvidence,
                "refresh claim adjudication evidence is invalid",
            ),
            (
                RefreshClaimAdjudicationError::NotPoisoned,
                "refresh claim is not poisoned",
            ),
            (
                RefreshClaimAdjudicationError::EvidenceConflict {
                    recorded_digest: [0u8; 32],
                    recorded_decision: RefreshOutcomeDecision::ProviderApplied,
                },
                "refresh claim adjudication conflicts with the recorded resolution",
            ),
        ] {
            match &error {
                RefreshClaimAdjudicationError::Storage => {},
                RefreshClaimAdjudicationError::AcknowledgementUnknown => {},
                RefreshClaimAdjudicationError::InvalidEvidence => {},
                RefreshClaimAdjudicationError::NotPoisoned => {},
                RefreshClaimAdjudicationError::EvidenceConflict { .. } => {},
            }
            assert_eq!(error.to_string(), expected);
        }
    }

    /// The two spellings are a persisted format.
    ///
    /// `as_str` writes the `adjudication_decision` column, so a rename strands
    /// every previously recorded row: `from_wire` answers `None` and the
    /// recommit path fails closed as `Storage`. The literals are therefore
    /// asserted here, not merely round-tripped.
    #[test]
    fn decision_wire_spellings_are_pinned_and_round_trip() {
        for (decision, spelling) in [
            (RefreshOutcomeDecision::ProviderApplied, "provider_applied"),
            (
                RefreshOutcomeDecision::ProviderNotApplied,
                "provider_not_applied",
            ),
        ] {
            assert_eq!(
                decision.as_str(),
                spelling,
                "the persisted spelling of {decision:?} is a durable format"
            );
            assert_eq!(RefreshOutcomeDecision::from_wire(spelling), Some(decision));
        }
        assert_eq!(RefreshOutcomeDecision::from_wire("unknown"), None);
    }

    #[test]
    fn adjudication_evidence_bound_is_four_kib() {
        // The bound is a durable-row size limit, not a message-size preference:
        // the note is digested and persisted verbatim on the incident row.
        assert_eq!(MAX_ADJUDICATION_EVIDENCE_BYTES, 4096);
    }
}
