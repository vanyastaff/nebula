//! Listing projection of an execution snapshot.
//!
//! Every `TransitionBatch` carries the snapshot together with this
//! projection, so storage can filter and order history without parsing the
//! opaque state. The projection is computed from the very `ExecutionState`
//! being serialized, so the two cannot disagree.

use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_storage_port::{ExecutionListing, ExecutionListingStatus};

/// Project `state` onto its queryable listing — what every snapshot write
/// (engine commits and test fixtures alike) hands storage alongside the state.
pub fn execution_listing(state: &ExecutionState) -> ExecutionListing {
    ExecutionListing::new(
        listing_status(state.status),
        state.started_at,
        state.completed_at,
    )
}

/// Exhaustive by design: a new execution status must be mapped here before
/// it can reach storage.
const fn listing_status(status: ExecutionStatus) -> ExecutionListingStatus {
    match status {
        ExecutionStatus::Created => ExecutionListingStatus::Created,
        ExecutionStatus::Running => ExecutionListingStatus::Running,
        ExecutionStatus::Paused => ExecutionListingStatus::Paused,
        ExecutionStatus::Cancelling => ExecutionListingStatus::Cancelling,
        ExecutionStatus::Completed => ExecutionListingStatus::Completed,
        ExecutionStatus::Failed => ExecutionListingStatus::Failed,
        ExecutionStatus::Cancelled => ExecutionListingStatus::Cancelled,
        ExecutionStatus::TimedOut => ExecutionListingStatus::TimedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ExecutionStatus; 8] = [
        ExecutionStatus::Created,
        ExecutionStatus::Running,
        ExecutionStatus::Paused,
        ExecutionStatus::Cancelling,
        ExecutionStatus::Completed,
        ExecutionStatus::Failed,
        ExecutionStatus::Cancelled,
        ExecutionStatus::TimedOut,
    ];

    /// The stored listing name is the execution status's own serde name, so
    /// the migration backfill (which reads `state.status`) and live commits
    /// write the same value.
    #[test]
    fn listing_status_names_match_execution_serde_names() {
        for status in ALL {
            let serde_name = serde_json::to_value(status).expect("status serializes");
            assert_eq!(
                serde_name,
                serde_json::json!(listing_status(status).as_str())
            );
            assert_eq!(status.is_terminal(), listing_status(status).is_terminal());
        }
    }
}
