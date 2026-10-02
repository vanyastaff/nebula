//! The execution-lease fence every adapter checks before it writes under a
//! turn's lease.
//!
//! A write authorized by an execution lease — an operation-ledger prepare or
//! advance, an iteration checkpoint — is admitted only while the presented
//! [`FencingToken`](nebula_storage_port::FencingToken) is the execution row's current fencing generation **and**
//! that lease is live by the backend's own clock. The comparison is answered
//! once here; each backend's helper (`inmem::execution_fence`,
//! `sqlite::execution_fence`, `postgres::execution_fence`) reads the row under
//! its own serialization point — the in-memory mutex, SQLite's single writer,
//! PostgreSQL's `SELECT … FOR UPDATE` — and asks this module.
//!
//! The refusal is backend-neutral: each port maps it onto its own error type,
//! so the fence cannot drift between the ports that share it.

use nebula_storage_port::FencingToken;

/// Why an execution fence refused a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceRefusal {
    /// The execution is absent, outside the caller's scope, superseded by a
    /// newer fencing generation, or its lease is not live.
    LeaseRejected,
    /// The backend failed before it could decide; nothing was written.
    #[cfg_attr(
        not(any(test, feature = "sqlite", feature = "postgres")),
        expect(
            dead_code,
            reason = "only the SQL fence helpers can fail before deciding"
        )
    )]
    Unavailable,
}

/// Admits `fencing` against the execution row's `current` generation and
/// whether its lease is `live` by the backend's authoritative clock.
///
/// Called under the execution owner's serialization point.
pub(crate) fn require_live_lease(
    fencing: FencingToken,
    current: u64,
    live: bool,
) -> Result<(), FenceRefusal> {
    if fencing.generation() != current || !live {
        return Err(FenceRefusal::LeaseRejected);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_superseded_attempt_cannot_decide_the_current_one() {
        assert_eq!(
            require_live_lease(FencingToken::from_generation(4), 5, true),
            Err(FenceRefusal::LeaseRejected)
        );
    }

    #[test]
    fn an_expired_lease_of_the_current_generation_is_refused() {
        assert_eq!(
            require_live_lease(FencingToken::from_generation(5), 5, false),
            Err(FenceRefusal::LeaseRejected)
        );
        assert_eq!(
            require_live_lease(FencingToken::from_generation(5), 5, true),
            Ok(())
        );
    }
}
