//! The in-memory execution-lease fence, shared by every in-memory adapter that
//! writes under a turn's lease.
//!
//! Callers hold the shared execution mutex for the whole operation, the
//! in-memory equivalent of the SQL backends' row lock: the fence decided here
//! cannot be invalidated before the caller's write.

use nebula_storage_port::{FencingToken, Scope};

use crate::execution_fence::{FenceRefusal, require_live_lease};

/// Admits a write for `execution_id` under `scope`: the execution row must
/// exist in that scope and, when `fencing` is given, hold that live lease at
/// `now` (this adapter's clock).
pub(super) fn require_live_execution(
    state: &super::execution::State,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), FenceRefusal> {
    let row = state
        .rows
        .get(execution_id)
        .filter(|row| row.scope == *scope)
        .ok_or(FenceRefusal::LeaseRejected)?;
    if let Some(fencing) = fencing {
        require_live_lease(
            fencing,
            row.fencing_generation,
            row.lease_holder.is_some()
                && row.lease_expires_at.is_some_and(|deadline| deadline > now),
        )?;
    }
    Ok(())
}
