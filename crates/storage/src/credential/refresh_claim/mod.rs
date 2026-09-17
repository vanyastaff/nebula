//! Durable cross-replica claim repository for credential refresh
//! coordination.
//!
//! See `docs/INTEGRATION_MODEL.md` (credential refresh) for integration context.
//!
//! The canonical trait + supporting types live in the spec-16 port crate
//! (`nebula_storage_port::store`). The acquisition CAS shape remains
//! loom-verified; the result surface intentionally breaks from the historical
//! adapter so an expired in-flight provider call is represented as durable
//! `OutcomeUnknown` poison rather than ordinary contention. This module binds
//! the three concrete backends to that port.
//!
//! The port exposes a closed, payload-free error taxonomy. SQL adapters
//! discard driver diagnostics at the edge via `SqlxClaimResultExt`. All
//! adapters preserve expired
//! `RefreshInFlight` rows for the atomic reclaim sweep and reject sentinel
//! marks once a claim expires.

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

// The canonical trait + value types live in the port crate. These aliases keep the historical
// `nebula_storage::credential` paths valid — `RefreshClaimRepo` is the port
// trait, `RepoError` the port's `RefreshClaimError`. Not a shim: there is
// exactly one definition (in the port); this is a rename-on-import.
pub use nebula_storage_port::store::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim,
    RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo, ReplicaId,
    SentinelState,
};

// The adjudication role is new with the reconciliation mechanism, so it has no
// historical path to keep: it is re-exported under its canonical port names.
pub use nebula_storage_port::store::{
    MAX_ADJUDICATION_EVIDENCE_BYTES, RefreshAdjudication, RefreshClaimAdjudicationError,
    RefreshClaimAdjudicator, RefreshOutcomeDecision,
};

/// SHA-256 of an adjudication evidence note.
///
/// One definition for every backend: the digest is what makes a recommitted
/// decision comparable across replicas, so the adapters must not each pick
/// their own hash.
pub(crate) fn adjudication_evidence_digest(evidence: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};

    Sha256::digest(evidence.as_bytes()).into()
}

/// Reject an evidence note the port refuses to digest.
///
/// Empty evidence would make every anonymous recommit look like a match, and
/// an unbounded note would let a caller grow durable incident rows without
/// limit.
pub(crate) fn validate_adjudication_evidence(
    evidence: &str,
) -> Result<(), RefreshClaimAdjudicationError> {
    if evidence.is_empty() || evidence.len() > MAX_ADJUDICATION_EVIDENCE_BYTES {
        return Err(RefreshClaimAdjudicationError::InvalidEvidence);
    }
    Ok(())
}

/// Decide what a recommit means against the resolution already on record.
///
/// Shared by all three adapters so the idempotency rule is stated once: the
/// same `(digest, decision)` pair is a no-op that reports
/// [`RefreshAdjudication::changed`] `false`, and any mismatch is refused
/// rather than silently overwriting an operator's earlier decision.
///
/// # Errors
///
/// [`RefreshClaimAdjudicationError::EvidenceConflict`] when the recorded pair
/// differs, and [`RefreshClaimAdjudicationError::Storage`] when an incident
/// carries a resolution timestamp but no decodable decision — that is a
/// corrupted row, not a caller mistake.
pub(crate) fn adjudicate_against_recorded_resolution(
    recorded_digest: &[u8],
    recorded_decision: Option<&str>,
    requested_digest: &[u8; 32],
    requested_decision: RefreshOutcomeDecision,
) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError> {
    let Some(recorded_decision) = recorded_decision.and_then(RefreshOutcomeDecision::from_wire)
    else {
        return Err(RefreshClaimAdjudicationError::Storage);
    };
    if recorded_digest != requested_digest || recorded_decision != requested_decision {
        return Err(RefreshClaimAdjudicationError::EvidenceConflict);
    }
    Ok(RefreshAdjudication {
        decision: recorded_decision,
        changed: false,
    })
}

/// Maps a SQL driver error into the closed backend-error variant at the
/// adapter edge.
///
/// Each SQL adapter (`sqlite`, `postgres`) calls `.store_err()?` on driver
/// results. The concrete diagnostic is deliberately discarded so database
/// text and persisted identifiers cannot cross the public port.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) trait SqlxClaimResultExt<T> {
    /// Convert a `Result<T, sqlx::Error>` into the closed port error.
    fn store_err(self) -> Result<T, RepoError>;
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
impl<T> SqlxClaimResultExt<T> for Result<T, sqlx::Error> {
    fn store_err(self) -> Result<T, RepoError> {
        self.map_err(|_| RepoError::Storage)
    }
}

#[cfg(test)]
mod replica_id_tests {
    use super::ReplicaId;

    #[test]
    fn short_id_is_stored_verbatim() {
        let r = ReplicaId::new("pod-a-1");
        assert_eq!(r.as_str(), "pod-a-1");
    }

    #[test]
    fn id_at_max_bytes_is_kept_intact() {
        let s: String = "a".repeat(ReplicaId::MAX_BYTES);
        let r = ReplicaId::new(s.clone());
        assert_eq!(r.as_str().len(), ReplicaId::MAX_BYTES);
        assert_eq!(r.as_str(), s);
    }

    #[test]
    fn oversized_ascii_id_is_truncated_to_max_bytes() {
        let s: String = "x".repeat(ReplicaId::MAX_BYTES + 100);
        let r = ReplicaId::new(s);
        assert_eq!(r.as_str().len(), ReplicaId::MAX_BYTES);
        assert!(r.as_str().chars().all(|c| c == 'x'));
    }

    #[test]
    fn truncation_respects_utf8_char_boundary() {
        // 4-byte char "🦀" (U+1F980 CRAB) placed near the cap so a
        // naïve byte-truncate would split it.
        let mut s = "a".repeat(ReplicaId::MAX_BYTES - 2);
        s.push('🦀');
        s.push_str("trailing");
        // s.len() now > MAX_BYTES because crab is 4 bytes and we added
        // MAX_BYTES - 2 + 4 + 8 bytes total.
        assert!(s.len() > ReplicaId::MAX_BYTES);
        let r = ReplicaId::new(s);
        // The crab byte sequence starts at byte index MAX_BYTES - 2
        // and would extend to MAX_BYTES + 2; truncation must back off
        // to MAX_BYTES - 2 to avoid splitting the codepoint.
        assert_eq!(r.as_str().len(), ReplicaId::MAX_BYTES - 2);
        // Round-trip: still valid UTF-8 and no panic on display.
        let _ = r.to_string();
    }
}
