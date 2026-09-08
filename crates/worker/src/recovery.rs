//! Bounded discovery of accepted turns whose runtime owner disappeared.

use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use nebula_core::WorkerFlavorRevisionId;
use nebula_engine::{RecoveryTurnOutcome, RecoveryTurnRequest, WorkflowEngine};
use nebula_storage_port::store::TurnRecovery;
use tokio::{task::JoinSet, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::WorkerRuntimeError;

const PAGE_SIZE: u32 = 32;
const MAX_ACTIVE: usize = 4;
const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const MAX_DISCOVERY_FAILURES: u32 = 5;

pub(crate) async fn run(
    engine: Arc<WorkflowEngine>,
    owner: Arc<dyn TurnRecovery>,
    flavor: WorkerFlavorRevisionId,
    holder: String,
    lease_ttl: Duration,
    shutdown: CancellationToken,
) -> Result<(), WorkerRuntimeError> {
    let holder: Arc<str> = holder.into();
    let mut active = JoinSet::new();
    let mut active_ids = HashSet::new();
    let mut pending = VecDeque::new();
    let mut cursor: Option<String> = None;
    let mut next_scan = Instant::now();
    let mut failures = 0u32;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        // Discovery can advance through many empty pages. Reap children before
        // fetching another page so cursor progress cannot hide a failed task
        // or keep a completed execution occupying a recovery slot.
        while let Some(joined) = active.try_join_next() {
            active_ids.remove(&joined.map_err(recovery_task_failure)?);
        }
        while active.len() < MAX_ACTIVE {
            let Some(turn): Option<nebula_storage_port::store::RecoverableTurn> =
                pending.pop_front()
            else {
                break;
            };
            let Ok(execution_id) = turn.execution_id().parse::<nebula_core::ExecutionId>() else {
                tracing::error!("recovery discovery returned an invalid execution identity");
                continue;
            };
            if !active_ids.insert(turn.execution_id().to_owned()) {
                continue;
            }
            let engine = Arc::clone(&engine);
            let owner = Arc::clone(&owner);
            let holder = Arc::clone(&holder);
            active.spawn(async move {
                let outcome = engine.resume_recoverable_turn(
                    turn.scope(),
                    execution_id,
                    RecoveryTurnRequest {
                        handoff: owner.as_ref(),
                        holder: holder.as_ref(),
                        lease_ttl,
                        accepted_fencing_generation: turn.accepted_fencing_generation(),
                    },
                ).await;
                match outcome {
                    RecoveryTurnOutcome::NotReady | RecoveryTurnOutcome::CandidateSuperseded => {},
                    RecoveryTurnOutcome::NotAccepted(error) => {
                        tracing::warn!(%execution_id, %error, "recovery preflight did not acquire ownership");
                    },
                    RecoveryTurnOutcome::AcceptanceUnknown(error) => {
                        tracing::warn!(%execution_id, %error, "recovery ownership acknowledgement unknown");
                    },
                    RecoveryTurnOutcome::Accepted(Ok(_)) => {
                        tracing::debug!(%execution_id, "accepted execution recovery completed its turn");
                    },
                    RecoveryTurnOutcome::Accepted(Err(error)) => {
                        tracing::warn!(%execution_id, %error, "accepted execution recovery stopped before completion");
                    },
                }
                turn.execution_id().to_owned()
            });
        }
        if pending.is_empty() && Instant::now() >= next_scan {
            let page = tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                page = owner.list_recoverable_turns(flavor, cursor.as_deref(), PAGE_SIZE) => page,
            };
            match page {
                Ok(page) => {
                    let advances = page.next_cursor().is_none_or(|next| {
                        !next.is_empty()
                            && next.len() <= 128
                            && cursor
                                .as_ref()
                                .is_none_or(|previous| next > previous.as_str())
                    });
                    if page.turns().len() > PAGE_SIZE as usize || !advances {
                        tracing::error!("recovery discovery returned an invalid bounded page");
                        cursor = None;
                        next_scan = Instant::now() + SCAN_INTERVAL;
                        continue;
                    }
                    failures = 0;
                    let (turns, next_cursor) = page.into_parts();
                    cursor = next_cursor;
                    retain_inactive_candidates(&mut pending, &active_ids, turns);
                    next_scan = if cursor.is_none() {
                        Instant::now() + SCAN_INTERVAL
                    } else {
                        Instant::now()
                    };
                    // Even empty pages of live owners must advance. Yield so a
                    // large reference population cannot monopolize an in-memory runner.
                    tokio::task::yield_now().await;
                    continue;
                },
                Err(error) => {
                    failures = failures.saturating_add(1);
                    if failures >= MAX_DISCOVERY_FAILURES {
                        tracing::error!(
                            attempts = failures,
                            %error,
                            "accepted-turn discovery exhausted its retry budget"
                        );
                        return Err(WorkerRuntimeError::AcceptedTurnRecovery {
                            attempts: failures,
                            source: error,
                        });
                    }
                    next_scan = Instant::now() + Duration::from_secs(1u64 << failures);
                    tracing::warn!(
                        attempt = failures,
                        maximum_attempts = MAX_DISCOVERY_FAILURES,
                        %error,
                        "accepted-turn discovery failed; retrying with bounded backoff"
                    );
                },
            }
        }
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            joined = active.join_next(), if !active.is_empty() => {
                if let Some(joined) = joined {
                    active_ids.remove(&joined.map_err(recovery_task_failure)?);
                }
            },
            () = tokio::time::sleep_until(next_scan), if pending.is_empty() => {},
        }
    }
    if let Ok(result) = tokio::time::timeout(SHUTDOWN_GRACE, async {
        while let Some(joined) = active.join_next().await {
            joined.map_err(recovery_task_failure)?;
        }
        Ok::<(), WorkerRuntimeError>(())
    })
    .await
    {
        return result;
    }
    active.abort_all();
    while let Some(joined) = active.join_next().await {
        if let Err(error) = joined
            && !error.is_cancelled()
        {
            return Err(recovery_task_failure(error));
        }
    }
    Ok(())
}

fn recovery_task_failure(source: tokio::task::JoinError) -> WorkerRuntimeError {
    WorkerRuntimeError::ComponentJoin {
        component: "accepted-turn-recovery-task",
        source,
    }
}

fn retain_inactive_candidates(
    pending: &mut VecDeque<nebula_storage_port::store::RecoverableTurn>,
    active_ids: &HashSet<String>,
    discovered: Vec<nebula_storage_port::store::RecoverableTurn>,
) {
    let mut retained_ids = HashSet::with_capacity(discovered.len());
    pending.extend(discovered.into_iter().filter(|turn| {
        !active_ids.contains(turn.execution_id())
            && retained_ids.insert(turn.execution_id().to_owned())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::{Scope, store::RecoverableTurn};

    fn candidate(execution_id: &str) -> RecoverableTurn {
        RecoverableTurn::new(
            Scope::new("workspace", "organization"),
            execution_id.to_owned(),
            1,
        )
    }

    #[test]
    fn active_page_prefix_cannot_starve_later_recovery_candidates() {
        let active_ids = ["active-a", "active-b", "active-c"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut pending = VecDeque::new();

        retain_inactive_candidates(
            &mut pending,
            &active_ids,
            vec![
                candidate("active-a"),
                candidate("active-b"),
                candidate("active-c"),
                candidate("waiting-a"),
                candidate("waiting-b"),
            ],
        );

        assert_eq!(
            pending
                .into_iter()
                .map(|turn| turn.execution_id().to_owned())
                .collect::<Vec<_>>(),
            ["waiting-a", "waiting-b"]
        );
    }
}
