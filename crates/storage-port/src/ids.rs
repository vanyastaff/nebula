//! Id seam + lease fencing token.
//!
//! The port reuses `nebula-core`'s typed ULID identifiers verbatim; they are
//! re-exported here so consumers and adapters have one import path for the
//! storage contract and the port does not re-define IDs.

pub use nebula_core::id::{
    ExecutionId, OrgId, ResourceId, TriggerId, UserId, WorkflowId, WorkflowVersionId, WorkspaceId,
};

/// Monotone lease fencing token.
///
/// A reclaim/takeover bumps the generation; `commit`/`renew_lease` reject a
/// non-current token even on a version match — this closes the zombie-runner
/// hole where a paused-then-superseded holder could still write state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FencingToken(u64);

impl FencingToken {
    /// Construct from a monotone generation counter.
    pub fn from_generation(g: u64) -> Self {
        Self(g)
    }

    /// Underlying generation.
    pub fn generation(self) -> u64 {
        self.0
    }
}
