//! In-memory [`JobDispatchQueue`] over the shared execution-store core.

use std::time::Duration;

use nebula_core::{PluginKey, WorkerFlavorRevisionId};
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::JobDispatchMsg;
use nebula_storage_port::store::{
    ClaimGeneration, JobClaim, JobClaimToken, JobDispatchQueue, ReclaimOutcome,
};
use tokio::time::Instant;

use super::execution::{QueuedJob, SharedState};

/// Format a raw 16-byte ULID as lowercase hex for `StorageError` ids. Uses
/// std formatting so the `inmem` module does not need the optional `hex` crate
/// that is only enabled by the `postgres`/`sqlite` features.
fn ulid_hex(id: &[u8; 16]) -> String {
    id.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Terminalise a claimed row, fenced on `(row id, Processing, generation)`.
///
/// Mirrors the SQL backends' single fenced `UPDATE` plus its zero-rows
/// disambiguation: an absent row is [`StorageError::NotFound`], and a present
/// row the token no longer owns is [`StorageError::FencedOut`] with **no state
/// change**. Returning `Ok` for a superseded token would let a worker whose
/// claim was reclaimed terminalise work the current owner is still doing.
fn acknowledge(
    state: &mut super::execution::State,
    claim: &JobClaimToken,
    terminal_status: &str,
    error: Option<&str>,
) -> Result<(), StorageError> {
    let id = claim.row_id();
    let Some(job) = state.jobs.get_mut(id) else {
        return Err(StorageError::NotFound {
            entity: "job_dispatch",
            id: ulid_hex(id),
        });
    };
    if job.msg.scope != *claim.scope() {
        return Err(StorageError::NotFound {
            entity: "job_dispatch",
            id: ulid_hex(id),
        });
    }
    if job.status != "Processing" || job.claim_generation != claim.generation().get() {
        return Err(StorageError::FencedOut {
            entity: "job_dispatch",
            id: ulid_hex(id),
        });
    }
    terminal_status.clone_into(&mut job.status);
    job.processed_at = Some(Instant::now());
    if let Some(error) = error {
        job.error_message = Some(error.to_owned());
    }
    Ok(())
}

// ── JobDispatchQueue ─────────────────────────────────────────────────────────

/// In-memory job-dispatch queue handle.
///
/// Shares the execution store's core with the execution aggregate and control queue.
#[derive(Debug, Clone)]
pub struct InMemoryJobDispatchQueue {
    inner: SharedState,
}

impl InMemoryJobDispatchQueue {
    /// Build a job-dispatch queue over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }
}

#[async_trait::async_trait]
impl JobDispatchQueue for InMemoryJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if st.jobs.contains_key(&msg.id) {
            return Err(StorageError::Duplicate {
                entity: "job_dispatch",
                detail: ulid_hex(&msg.id),
            });
        }
        st.jobs.insert(
            msg.id,
            QueuedJob {
                msg: msg.clone(),
                status: "Pending".to_owned(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );
        tracing::debug!(target: "nebula_storage::inmem", "job_dispatch: enqueued");
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, available_plugins), fields(batch_size, advertised_worker_flavor_id = %worker_flavor_id))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
        worker_flavor_id: WorkerFlavorRevisionId,
    ) -> Result<Vec<JobClaim>, StorageError> {
        // Parity with SQLite + Postgres: an empty advertised set claims nothing.
        if available_plugins.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self.inner.lock();
        let now = Instant::now();

        // Stable order so a bounded batch is deterministic across calls.
        //
        // Superset predicate: the worker may claim a job only when its
        // available plugins cover every plugin in `required_plugins`.  The
        // check is inside the parking_lot Mutex so the predicate + status flip
        // are atomic (no TOCTOU window). The primary plugin remains an
        // independent requirement even if malformed metadata omits it from
        // `required_plugins`.
        let mut ids: Vec<[u8; 16]> = st
            .jobs
            .iter()
            .filter(|(_, q)| {
                q.status == "Pending"
                    && q.msg.required_worker_flavor_id == worker_flavor_id
                    && available_plugins.contains(&q.msg.required_plugin_key)
                    && q.msg
                        .required_plugins
                        .iter()
                        .all(|rp| available_plugins.contains(rp))
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();

        let mut claimed = Vec::new();
        for id in ids.into_iter().take(batch_size as usize) {
            if let Some(q) = st.jobs.get_mut(&id) {
                // Mint the generation in the same critical section that flips
                // Pending -> Processing, so no other claimer can observe the
                // row as claimed under a generation that was not minted yet.
                // The SQL backends do this inside the claiming UPDATE.
                let Some(generation) = q.claim_generation.checked_add(1) else {
                    // Fail closed: a wrapped generation would let a superseded
                    // token match a future claim, which is exactly the fence
                    // this counter exists to provide.
                    return Err(StorageError::Internal(format!(
                        "job_dispatch claim generation overflowed for row {}",
                        ulid_hex(&id)
                    )));
                };
                q.claim_generation = generation;
                "Processing".clone_into(&mut q.status);
                q.processed_by = Some(*processor);
                q.processed_at = Some(now);
                q.msg.reclaim_count = q.reclaim_count;
                claimed.push(JobClaim {
                    msg: q.msg.clone(),
                    token: JobClaimToken::new(
                        id,
                        ClaimGeneration::new(generation),
                        q.msg.scope.clone(),
                    ),
                });
            }
        }
        tracing::debug!(
            target: "nebula_storage::inmem",
            claimed = claimed.len(),
            "job_dispatch: claimed"
        );
        Ok(claimed)
    }

    async fn mark_dispatched(&self, claim: &JobClaimToken) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Dispatched", None)
    }

    async fn mark_failed(&self, claim: &JobClaimToken, error: &str) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Failed", Some(error))
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        let mut st = self.inner.lock();
        let now = Instant::now();
        let mut outcome = ReclaimOutcome::default();
        for q in st.jobs.values_mut() {
            if q.status != "Processing" {
                continue;
            }
            let stale = match q.processed_at {
                Some(at) => now.duration_since(at) >= reclaim_after,
                None => false,
            };
            if !stale {
                continue;
            }
            if q.reclaim_count >= max_reclaim_count {
                "Failed".clone_into(&mut q.status);
                q.processed_at = Some(now);
                q.error_message = Some(format!(
                    "reclaim exhausted: presumed dead after {} reclaims",
                    q.reclaim_count
                ));
                outcome.exhausted += 1;
            } else {
                // Ownership is cleared, but `claim_generation` is deliberately
                // left alone: the next claim increments past it, so the token
                // this reclaim just invalidated can never match again.
                "Pending".clone_into(&mut q.status);
                q.reclaim_count = q.reclaim_count.saturating_add(1);
                q.processed_by = None;
                q.processed_at = None;
                outcome.reclaimed += 1;
            }
        }
        Ok(outcome)
    }

    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError> {
        let mut state = self.inner.lock();
        let now = Instant::now();
        let rows_before_cleanup = state.jobs.len();
        state.jobs.retain(|_, job| {
            let is_terminal = matches!(job.status.as_str(), "Dispatched" | "Failed");
            let has_expired = job
                .processed_at
                .is_some_and(|processed_at| now.duration_since(processed_at) > retention);
            !(is_terminal && has_expired)
        });
        let deleted = rows_before_cleanup.saturating_sub(state.jobs.len());
        u64::try_from(deleted).map_err(|error| {
            StorageError::Internal(format!(
                "job-dispatch cleanup count cannot be represented as u64: {error}"
            ))
        })
    }
}

#[cfg(test)]
#[path = "job_dispatch_job_ownership_tests.rs"]
mod job_ownership_tests;
