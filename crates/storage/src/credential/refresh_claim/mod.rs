//! Durable cross-replica claim repository for credential refresh
//! coordination.
//!
//! Per ADR-0041 + sub-spec
//! `docs/INTEGRATION_MODEL.md (credential refresh; ADR-0030/0041)`.
//!
//! The canonical trait + supporting types now live in the spec-16 port
//! crate (`nebula_storage_port::store`) — this component is loom-verified,
//! so its surface was re-homed **shape-unchanged**. This module re-exports
//! the port types under their historical names so existing consumers
//! (engine refresh-coordinator, audit chain, tests) compile across the
//! move without churn, and binds the three concrete backends to the port
//! trait via those aliases.
//!
//! The only behavioral delta from the pre-move definition is the
//! backend-error variant: the port's `RepoError::Storage` carries a
//! `String` (the port has no sqlx dependency). The SQL adapters map their
//! driver error into that variant at the edge via `SqlxClaimResultExt`.

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

// Re-home (spec §4.2, shape-unchanged): the canonical trait + value types
// moved to the port crate. These aliases keep the historical
// `nebula_storage::credential` paths valid — `RefreshClaimRepo` is the port
// trait, `RepoError` the port's `RefreshClaimError`. Not a shim: there is
// exactly one definition (in the port); this is a rename-on-import.
pub use nebula_storage_port::store::{
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim,
    RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo, ReplicaId,
    SentinelState,
};

/// Maps a SQL driver error into the port's string-typed backend-error
/// variant at the adapter edge.
///
/// The port crate has no sqlx dependency, so `RepoError::Storage`
/// carries a `String` rather than `#[from] sqlx::Error`. Each SQL adapter
/// (`sqlite`, `postgres`) calls `.store_err()?` on its driver results so the
/// CAS / heartbeat / sentinel / reclaim invariants stay byte-for-byte
/// identical to the loom-verified pre-move logic — only the error
/// conversion point changed.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) trait SqlxClaimResultExt<T> {
    /// Convert a `Result<T, sqlx::Error>` into the port's
    /// `Result<T, RefreshClaimError>` by stringifying the driver error.
    fn store_err(self) -> Result<T, RepoError>;
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
impl<T> SqlxClaimResultExt<T> for Result<T, sqlx::Error> {
    fn store_err(self) -> Result<T, RepoError> {
        self.map_err(|e| RepoError::Storage(e.to_string()))
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
