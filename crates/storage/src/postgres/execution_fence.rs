//! The PostgreSQL execution-lease fence, shared by every PostgreSQL adapter
//! that writes under a turn's lease.
//!
//! The execution row is read `FOR UPDATE`: it stays locked until the caller's
//! transaction ends, so the fence decided here cannot be invalidated (a lease
//! renewal, takeover or release) before the caller's write commits.

use nebula_storage_port::{FencingToken, Scope};
use sqlx::{Postgres, Row, Transaction};

use crate::execution_fence::{FenceRefusal, require_live_lease};

/// Locks the execution row of `execution_id` under `scope` and admits a write:
/// the row must exist in that scope and, when `fencing` is given, hold that
/// live lease by the database clock (`clock_timestamp()`, read after the lock
/// wait).
///
/// A driver failure is [`FenceRefusal::Unavailable`]: nothing was decided or
/// written.
pub(crate) async fn lock_execution(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
) -> Result<(), FenceRefusal> {
    let row = sqlx::query(
        "SELECT fencing_generation, lease_holder, lease_expires_at_ms FROM port_executions \
         WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE",
    )
    .bind(execution_id)
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?
    .ok_or(FenceRefusal::LeaseRejected)?;
    if let Some(fencing) = fencing {
        let now: i64 =
            sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
                .fetch_one(&mut **tx)
                .await
                .map_err(unavailable)?;
        let generation: i64 = row.try_get("fencing_generation").map_err(unavailable)?;
        let holder: Option<String> = row.try_get("lease_holder").map_err(unavailable)?;
        let expires: Option<i64> = row.try_get("lease_expires_at_ms").map_err(unavailable)?;
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
