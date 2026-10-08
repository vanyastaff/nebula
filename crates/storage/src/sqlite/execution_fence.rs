//! The SQLite execution-lease fence, shared by every SQLite adapter that writes
//! under a turn's lease.
//!
//! Callers run inside a `BEGIN IMMEDIATE` transaction: SQLite's single writer
//! is the lock, so the fence read here cannot be invalidated before the
//! caller's write in the same transaction.

use nebula_storage_port::{FencingToken, Scope};
use sqlx::{Row, Sqlite, Transaction};

use crate::execution_fence::{FenceRefusal, require_live_lease};

/// Admits a write for `execution_id` under `scope`: the execution row must
/// exist in that scope and, when `fencing` is given, hold that live lease by
/// the database clock.
///
/// A driver failure is [`FenceRefusal::Unavailable`]: nothing was decided or
/// written.
pub(crate) async fn lock_execution(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
) -> Result<(), FenceRefusal> {
    let row = sqlx::query(
        "SELECT fencing_generation, lease_holder, lease_expires_at FROM executions \
         WHERE org_id = ? AND workspace_id = ? AND id = ?",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(execution_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?
    .ok_or(FenceRefusal::LeaseRejected)?;
    if let Some(fencing) = fencing {
        // Microseconds since the Unix epoch, as `lease_expires_at` is stored.
        let now: i64 = sqlx::query_scalar(
            "SELECT CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(unavailable)?;
        let generation: i64 = row.try_get("fencing_generation").map_err(unavailable)?;
        let holder: Option<String> = row.try_get("lease_holder").map_err(unavailable)?;
        let expires: Option<i64> = row.try_get("lease_expires_at").map_err(unavailable)?;
        require_live_lease(
            fencing,
            u64::try_from(generation).map_err(|_| FenceRefusal::LeaseRejected)?,
            holder.is_some() && expires.is_some_and(|deadline| deadline > now),
        )?;
    }
    Ok(())
}

/// A driver failure before the fence decided.
fn unavailable(_error: sqlx::Error) -> FenceRefusal {
    FenceRefusal::Unavailable
}
