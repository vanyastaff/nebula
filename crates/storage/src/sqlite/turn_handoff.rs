//! SQLite dispatch-claim → execution-turn handoff.
//!
//! The claim re-check, the lease acquisition, and the queue acknowledgement are
//! three statements in **one** `BEGIN IMMEDIATE` transaction, so a crash can
//! only land on one side of it: either the row is still `Processing` and no
//! turn was accepted, or the row is acknowledged and the execution holds the
//! lease that governs the work.
//!
//! The database stamps the lease deadline, exactly as `acquire_lease` does.
//! There is one rule for lease time across all three backends, so a lease
//! cannot mean something different depending on which adapter is wired.

use nebula_storage_port::store::{ExecutionTurnHandoff, TurnAcceptance, TurnHandoff};
use nebula_storage_port::{FencingToken, StorageError};
use sqlx::SqlitePool;

use crate::inmem::acceptance_label;
use crate::sqlite::execution::conn_err;

/// SQLite-backed owner of the dispatch-claim → execution-turn handoff.
#[derive(Clone, Debug)]
pub struct SqliteTurnHandoff {
    pool: SqlitePool,
}

impl SqliteTurnHandoff {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Clamp the lease TTL exactly as the execution store does, so a handoff cannot
/// mint a lease the store itself would have refused to issue.
fn normalized_ttl_ms(ttl: std::time::Duration) -> i64 {
    let clamped = std::time::Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0));
    i64::try_from(clamped.as_millis()).unwrap_or(i64::MAX)
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for SqliteTurnHandoff {
    #[tracing::instrument(
        level = "debug",
        name = "turn_handoff.accept_turn",
        skip(self, handoff),
        fields(
            backend = "sqlite",
            execution_id = handoff.execution_id,
            claim_generation = handoff.claim.generation().get(),
            outcome = tracing::field::Empty,
        )
    )]
    async fn accept_turn(&self, handoff: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        let ttl_ms = normalized_ttl_ms(handoff.lease_ttl);
        let result = async {
            // The write lock is taken up front: the claim re-check decides
            // whether the lease may be acquired, and a deferred transaction
            // would let a reclaim sweep move the row in between.
            let mut tx = self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(conn_err)?;

            let still_claimed: Option<i64> = sqlx::query_scalar(
                // The row must belong to the execution and tenant this handoff
                // names. A valid token from one job paired with another
                // execution id would otherwise lease the wrong aggregate and
                // acknowledge — dropping — the job that was actually claimed.
                "SELECT 1 FROM port_job_dispatch_queue \
                 WHERE id = ? AND status = 'Processing' AND claim_generation = ? \
                   AND execution_id = ? AND workspace_id = ? AND org_id = ?",
            )
            .bind(handoff.claim.row_id().as_slice())
            .bind(i64::try_from(handoff.claim.generation().get()).unwrap_or(i64::MAX))
            .bind(handoff.execution_id)
            .bind(&handoff.scope.workspace_id)
            .bind(&handoff.scope.org_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;
            if still_claimed.is_none() {
                drop(tx.rollback().await);
                return Ok(TurnAcceptance::ClaimSuperseded);
            }

            // Same statement shape as `acquire_lease`: the database decides
            // live-vs-expired and stamps the deadline, so a caller's clock
            // cannot fence out a healthy peer or mint an already-dead lease.
            let acquired: Option<i64> = sqlx::query_scalar(
                "UPDATE port_executions \
                 SET lease_holder = ?, \
                     lease_expires_at_ms = \
                         CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) + ?, \
                     fencing_generation = fencing_generation + 1 \
                 WHERE id = ? AND workspace_id = ? AND org_id = ? \
                   AND (lease_expires_at_ms IS NULL \
                        OR lease_expires_at_ms \
                           < CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)) \
                 RETURNING fencing_generation",
            )
            .bind(handoff.holder)
            .bind(ttl_ms)
            .bind(handoff.execution_id)
            .bind(&handoff.scope.workspace_id)
            .bind(&handoff.scope.org_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;

            let Some(generation) = acquired else {
                // Either a live lease blocks the turn, or the execution does
                // not exist in this tenant. Distinguishing them costs one read
                // and is worth it: a caller must not treat a missing execution
                // as contention it can wait out.
                let exists: Option<i64> = sqlx::query_scalar(
                    "SELECT 1 FROM port_executions \
                     WHERE id = ? AND workspace_id = ? AND org_id = ?",
                )
                .bind(handoff.execution_id)
                .bind(&handoff.scope.workspace_id)
                .bind(&handoff.scope.org_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(conn_err)?;
                drop(tx.rollback().await);
                return if exists.is_some() {
                    // The queue row stays claimed on purpose: acknowledging it
                    // would make it terminal while no owner ever ran the turn.
                    Ok(TurnAcceptance::TurnHeldByAnotherOwner)
                } else {
                    Err(StorageError::not_found("execution", handoff.execution_id))
                };
            };

            sqlx::query(
                "UPDATE port_job_dispatch_queue SET status = 'Dispatched' \
                 WHERE id = ? AND status = 'Processing' AND claim_generation = ? \
                   AND execution_id = ? AND workspace_id = ? AND org_id = ?",
            )
            .bind(handoff.claim.row_id().as_slice())
            .bind(i64::try_from(handoff.claim.generation().get()).unwrap_or(i64::MAX))
            .bind(handoff.execution_id)
            .bind(&handoff.scope.workspace_id)
            .bind(&handoff.scope.org_id)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;

            tx.commit().await.map_err(conn_err)?;
            Ok(TurnAcceptance::Accepted {
                fence: FencingToken::from_generation(u64::try_from(generation).unwrap_or_default()),
            })
        }
        .await;

        let outcome = acceptance_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(
            target: "nebula_storage::sqlite",
            outcome,
            "dispatch claim handed off to an execution turn"
        );
        result
    }
}
