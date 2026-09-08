//! Lease acquisition and durable turn handoff decisions.

use super::*;
use nebula_core::WorkerFlavorRevisionId;
use nebula_storage_port::store::{
    ControlStartAcceptance, ControlStartHandoff, RecoveryTurnAcceptance, RecoveryTurnHandoff,
};

pub(super) enum ExactTurnFailure {
    BeforeLease(EngineError),
    AfterLease(EngineError),
    AcceptanceUnknown(EngineError),
    ClaimSuperseded,
    NotReady,
    CandidateSuperseded,
}

impl From<EngineError> for ExactTurnFailure {
    fn from(error: EngineError) -> Self {
        Self::BeforeLease(error)
    }
}

#[derive(Clone, Copy)]
pub(super) enum ResumeLeaseSource<'a> {
    Acquire,
    Adopt {
        fence: nebula_storage_port::FencingToken,
    },
    ControlStart(ClaimedStartRequest<'a>),
    Recovery(RecoveryTurnRequest<'a>),
    Control(&'a ClaimedControlTurnRequest),
}

pub(super) struct ResumeLeaseRequest<'a> {
    pub(super) scope: &'a Scope,
    pub(super) execution_id: ExecutionId,
    pub(super) source: ResumeLeaseSource<'a>,
    pub(super) cancel_token: CancellationToken,
    pub(super) execution_state: &'a mut ExecutionState,
    pub(super) repository_version: &'a mut u64,
    pub(super) loaded_repository_version: u64,
    pub(super) worker_flavor_revision_id: WorkerFlavorRevisionId,
    pub(super) original_status: ExecutionStatus,
    pub(super) outputs: &'a DashMap<NodeKey, serde_json::Value>,
    pub(super) started: Instant,
}

pub(super) enum LeasePreparation {
    Drive(Option<LeaseGuard>),
    Completed(ExecutionResult),
}

impl WorkflowEngine {
    pub(super) async fn prepare_resume_lease(
        &self,
        request: ResumeLeaseRequest<'_>,
    ) -> Result<LeasePreparation, ExactTurnFailure> {
        let ResumeLeaseRequest {
            scope,
            execution_id,
            source,
            cancel_token,
            execution_state,
            repository_version,
            loaded_repository_version,
            worker_flavor_revision_id,
            original_status,
            outputs,
            started,
        } = request;

        let lease = match source {
            ResumeLeaseSource::Control(request) => {
                let guard = self
                    .acquire_and_heartbeat_lease(scope, execution_id, cancel_token.clone())
                    .await?
                    .ok_or(EngineError::MissingExactRuntime)?;
                let fence = guard
                    .fencing_token()
                    .ok_or(EngineError::MissingExactRuntime)?;
                let armed = match self
                    .commit_claimed_control(
                        scope,
                        execution_id,
                        request,
                        execution_state,
                        repository_version,
                        fence,
                    )
                    .await
                {
                    Ok(armed) => armed,
                    Err(failure) => {
                        guard.shutdown().await;
                        return Err(match failure {
                            control_turn::ControlCommitFailure::NotAccepted(error) => {
                                ExactTurnFailure::BeforeLease(error)
                            },
                            control_turn::ControlCommitFailure::ClaimSuperseded => {
                                ExactTurnFailure::ClaimSuperseded
                            },
                            control_turn::ControlCommitFailure::AcceptanceUnknown(error) => {
                                ExactTurnFailure::AcceptanceUnknown(error)
                            },
                        });
                    },
                };
                let no_work = match request.command {
                    nebula_storage_port::store::ControlTurnCommand::Resume { .. } => {
                        armed.is_empty() && original_status != ExecutionStatus::Created
                    },
                    nebula_storage_port::store::ControlTurnCommand::Restart => {
                        original_status == ExecutionStatus::Running
                    },
                    _ => {
                        guard.shutdown().await;
                        return Err(EngineError::InvalidRecordedExecution.into());
                    },
                };
                if no_work {
                    guard.shutdown().await;
                    return Ok(LeasePreparation::Completed(ExecutionResult {
                        execution_id,
                        status: original_status,
                        node_outputs: outputs
                            .iter()
                            .map(|output| (output.key().clone(), output.value().clone()))
                            .collect(),
                        node_errors: HashMap::new(),
                        duration: started.elapsed(),
                        termination_reason: None,
                    }));
                }
                Some(guard)
            },
            ResumeLeaseSource::Recovery(request) => {
                let execution_key = execution_id.to_string();
                let recovery_handoff = RecoveryTurnHandoff::for_candidate(
                    scope,
                    &execution_key,
                    worker_flavor_revision_id,
                    request.accepted_fencing_generation,
                )
                .at_version(loaded_repository_version)
                .lease_to(request.holder, request.lease_ttl);
                let decision = request
                    .handoff
                    .accept_recovery_turn(&recovery_handoff)
                    .await;
                let fence = match decision {
                    Ok(RecoveryTurnAcceptance::Accepted { fence }) => fence,
                    Ok(RecoveryTurnAcceptance::CandidateSuperseded) => {
                        return Err(ExactTurnFailure::CandidateSuperseded);
                    },
                    Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner) => {
                        return Err(EngineError::Leased {
                            execution_id,
                            holder: "another runtime owner".to_owned(),
                        }
                        .into());
                    },
                    Ok(RecoveryTurnAcceptance::VersionConflict { actual }) => {
                        return Err(EngineError::RecoveryVersionConflict {
                            expected: loaded_repository_version,
                            actual,
                        }
                        .into());
                    },
                    Ok(_) => {
                        return Err(ExactTurnFailure::AcceptanceUnknown(
                            EngineError::ControlTurnInterrupted,
                        ));
                    },
                    Err(
                        source @ nebula_storage_port::StorageError::AcknowledgementUnknown {
                            ..
                        },
                    ) => {
                        return Err(ExactTurnFailure::AcceptanceUnknown(
                            EngineError::RecoveryHandoff { source },
                        ));
                    },
                    Err(source) => return Err(EngineError::RecoveryHandoff { source }.into()),
                };
                tracing::info!(
                    %execution_id,
                    fence_generation = fence.generation(),
                    "accepted execution turn recovered under a fresh fence"
                );
                Some(
                    self.adopt_handoff_lease(scope, execution_id, fence, cancel_token.clone())
                        .await
                        .map_err(ExactTurnFailure::AfterLease)?,
                )
            },
            ResumeLeaseSource::Acquire => {
                self.acquire_and_heartbeat_lease(scope, execution_id, cancel_token.clone())
                    .await?
            },
            ResumeLeaseSource::Adopt { fence } => Some(
                self.adopt_handoff_lease(scope, execution_id, fence, cancel_token.clone())
                    .await?,
            ),
            ResumeLeaseSource::ControlStart(request) => {
                let execution_key = execution_id.to_string();
                let start_handoff = ControlStartHandoff::for_claim(
                    scope,
                    &execution_key,
                    request.claim,
                    worker_flavor_revision_id,
                )
                .at_version(loaded_repository_version)
                .lease_to(request.holder, request.lease_ttl);
                let decision = request.handoff.accept_control_start(&start_handoff).await;
                let fence = match decision {
                    Ok(ControlStartAcceptance::Accepted { fence }) => fence,
                    Ok(ControlStartAcceptance::ClaimSuperseded) => {
                        return Err(ExactTurnFailure::ClaimSuperseded);
                    },
                    Ok(ControlStartAcceptance::TurnHeldByAnotherOwner) => {
                        return Err(EngineError::Leased {
                            execution_id,
                            holder: "another runtime owner".to_owned(),
                        }
                        .into());
                    },
                    Ok(ControlStartAcceptance::VersionConflict { actual }) => {
                        return Err(EngineError::ControlStartVersionConflict {
                            expected: loaded_repository_version,
                            actual,
                        }
                        .into());
                    },
                    Ok(_) => {
                        return Err(ExactTurnFailure::AcceptanceUnknown(
                            EngineError::ControlTurnInterrupted,
                        ));
                    },
                    Err(
                        source @ nebula_storage_port::StorageError::AcknowledgementUnknown {
                            ..
                        },
                    ) => {
                        return Err(ExactTurnFailure::AcceptanceUnknown(
                            EngineError::ControlStartHandoff { source },
                        ));
                    },
                    Err(source) => return Err(EngineError::ControlStartHandoff { source }.into()),
                };
                tracing::info!(
                    %execution_id,
                    fence_generation = fence.generation(),
                    "control Start delivery accepted before execution"
                );
                Some(
                    self.adopt_handoff_lease(scope, execution_id, fence, cancel_token.clone())
                        .await
                        .map_err(ExactTurnFailure::AfterLease)?,
                )
            },
        };

        Ok(LeasePreparation::Drive(lease))
    }
}
