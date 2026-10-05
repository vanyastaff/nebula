//! Atomic command acceptance under the execution owner's current fence.

use super::*;
use nebula_storage_port::store::{
    ControlClaimToken, ControlObservationAcknowledgement, ControlTurnCommand, ControlTurnCommit,
    ControlTurnCommitOutcome, ControlTurnTransition,
};

/// Current Resume or Restart claim and its atomic execution-owner capability.
#[derive(Clone, Debug)]
pub struct ClaimedControlTurnRequest {
    /// Actual queue claim, checked again by its owner.
    pub claim: ControlClaimToken,
    /// Owner of the queue, marker and execution aggregate.
    pub handoff: Arc<dyn nebula_storage_port::ExecutionTurnHandoff>,
    /// Exact persisted command, including the Resume target.
    pub command: ControlTurnCommand,
}

/// Whether a Resume or Restart delivery was durably accepted.
#[must_use]
pub enum ClaimedControlTurnOutcome {
    /// The command has not been accepted.
    NotAccepted(EngineError),
    /// Another delivery owns the command now.
    ClaimSuperseded,
    /// Commit or live-owner acknowledgement was lost; never mutate the claim.
    AcceptanceUnknown(EngineError),
    /// Command acceptance is durable, independently of subsequent execution.
    Accepted(Result<(), EngineError>),
}
impl std::fmt::Debug for ClaimedControlTurnOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotAccepted(_) => "NotAccepted(<redacted>)",
            Self::ClaimSuperseded => "ClaimSuperseded",
            Self::AcceptanceUnknown(_) => "AcceptanceUnknown(<redacted>)",
            Self::Accepted(Ok(())) => "Accepted(Ok(()))",
            Self::Accepted(Err(_)) => "Accepted(Err(<redacted>))",
        })
    }
}

pub(super) enum ControlCommitFailure {
    NotAccepted(EngineError),
    ClaimSuperseded,
    AcceptanceUnknown(EngineError),
}
impl From<EngineError> for ControlCommitFailure {
    fn from(error: EngineError) -> Self {
        Self::NotAccepted(error)
    }
}
impl ControlCommitFailure {
    pub(super) fn into_outcome(self) -> ClaimedControlTurnOutcome {
        match self {
            Self::NotAccepted(error) => ClaimedControlTurnOutcome::NotAccepted(error),
            Self::ClaimSuperseded => ClaimedControlTurnOutcome::ClaimSuperseded,
            Self::AcceptanceUnknown(error) => ClaimedControlTurnOutcome::AcceptanceUnknown(error),
        }
    }
}

impl WorkflowEngine {
    /// `None` means the request definitely never entered a live-owner mailbox.
    pub(super) async fn deliver_claimed_control(
        &self,
        execution_id: ExecutionId,
        request: &ClaimedControlTurnRequest,
    ) -> Option<ClaimedControlTurnOutcome> {
        let receiver = {
            let (ack, receiver) = oneshot::channel();
            let entry = self.running.get(&execution_id)?;
            let resume_target = match &request.command {
                ControlTurnCommand::Resume { target } => target.clone(),
                ControlTurnCommand::Restart => None,
                _ => return None,
            };
            entry
                .resume_tx
                .try_send(ResumeRequest {
                    ack,
                    resume_target,
                    control: Some(request.clone()),
                })
                .ok()?;
            receiver
        };
        match tokio::time::timeout(RESUME_ACK_TIMEOUT, receiver).await {
            Ok(Ok(ResumeOutcome::Claimed(outcome))) => Some(outcome),
            _ => Some(ClaimedControlTurnOutcome::AcceptanceUnknown(
                EngineError::ControlTurnInterrupted,
            )),
        }
    }

    /// Publish an arm only with acknowledgement of checkpoint, marker and queue completion together.
    #[tracing::instrument(skip_all, fields(%execution_id, org_id = %scope.org_id, workspace_id = %scope.workspace_id, backend = tracing::field::Empty, outcome = tracing::field::Empty, observation_acknowledgement = tracing::field::Empty, expected_flavor = tracing::field::Empty, actual_flavor = tracing::field::Empty))]
    pub(super) async fn commit_claimed_control(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        request: &ClaimedControlTurnRequest,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
        fence: nebula_storage_port::FencingToken,
    ) -> Result<Vec<NodeKey>, ControlCommitFailure> {
        let flavor = exec_state
            .worker_flavor_revision_id
            .ok_or(EngineError::MissingExactRuntime)?;
        let now = self.clock.now();
        let mut candidate = match &request.command {
            ControlTurnCommand::Resume { .. } => Some(exec_state.clone()),
            ControlTurnCommand::Restart => None,
            _ => return Err(EngineError::InvalidRecordedExecution.into()),
        };
        let armed = match (&mut candidate, &request.command) {
            (Some(candidate), ControlTurnCommand::Resume { target }) => {
                arm_signal_waits_under_lease(candidate, target.as_ref(), now)
            },
            _ => Vec::new(),
        };
        if armed.is_empty() {
            candidate = None;
        }
        if let Some(candidate) = &mut candidate {
            candidate.version = candidate
                .version
                .checked_add(1)
                .ok_or(EngineError::InvalidRecordedExecution)?;
            candidate.updated_at = now;
            checkpoint::validate_checkpoint_size(candidate)?;
        }
        let id = execution_id.to_string();
        let batch = candidate
            .as_ref()
            .map(|candidate| {
                let state = serde_json::to_value(candidate)
                    .map_err(|_| EngineError::InvalidRecordedExecution)?;
                nebula_storage_port::TransitionBatch::builder()
                    .scope(scope.clone())
                    .execution_id(&id)
                    .expected_version(*repo_version)
                    .fencing(fence)
                    .new_state(state)
                    .build()
                    .map_err(|_| EngineError::InvalidRecordedExecution)
            })
            .transpose()?;
        let transition = match &batch {
            Some(batch) => ControlTurnTransition::Checkpoint(batch),
            None => ControlTurnTransition::Unchanged {
                scope,
                execution_id: &id,
                expected_version: *repo_version,
                fence,
            },
        };
        let decision = request
            .handoff
            .commit_control_turn(&ControlTurnCommit::new(
                request.claim.clone(),
                flavor,
                request.command.clone(),
                transition,
            ))
            .await;
        let observation = match &decision {
            Ok(ControlTurnCommitOutcome::Accepted {
                fence: accepted, ..
            }) if *accepted == fence => Some((
                nebula_execution::ExecutionControlOutcome::Accepted,
                ControlObservationAcknowledgement::Recorded,
            )),
            Ok(
                ControlTurnCommitOutcome::FencedOut {
                    observation_acknowledgement,
                }
                | ControlTurnCommitOutcome::ClaimFenced {
                    observation_acknowledgement,
                    ..
                },
            ) => Some((
                nebula_execution::ExecutionControlOutcome::Fenced,
                *observation_acknowledgement,
            )),
            Ok(ControlTurnCommitOutcome::FlavorMismatch {
                observation_acknowledgement,
                ..
            }) => Some((
                nebula_execution::ExecutionControlOutcome::FlavorMismatch,
                *observation_acknowledgement,
            )),
            Ok(ControlTurnCommitOutcome::VersionConflict {
                observation_acknowledgement,
                ..
            }) => Some((
                nebula_execution::ExecutionControlOutcome::Deferred,
                *observation_acknowledgement,
            )),
            _ => None,
        };
        if let Some((outcome, acknowledgement)) = observation {
            let backend = request.handoff.backend_kind();
            let span = tracing::Span::current();
            span.record("backend", backend.as_str());
            span.record("outcome", outcome.as_str());
            span.record("observation_acknowledgement", acknowledgement.as_str());
            if acknowledgement == ControlObservationAcknowledgement::Recorded
                && let Err(error) = crate::control_metrics::record_execution_control_outcome(
                    &self.metrics,
                    backend,
                    outcome,
                )
            {
                tracing::warn!(%error, "execution-control outcome metric could not be recorded");
            }
        }
        match decision {
            Ok(ControlTurnCommitOutcome::Accepted {
                fence: accepted_fence,
                new_version,
            }) => {
                if accepted_fence != fence {
                    return Err(ControlCommitFailure::AcceptanceUnknown(
                        EngineError::ControlTurnInterrupted,
                    ));
                }
                *repo_version = new_version;
                if let Some(candidate) = candidate {
                    *exec_state = candidate;
                }
                Ok(armed)
            },
            Ok(
                ControlTurnCommitOutcome::ClaimSuperseded
                | ControlTurnCommitOutcome::ClaimFenced { .. },
            ) => Err(ControlCommitFailure::ClaimSuperseded),
            Ok(ControlTurnCommitOutcome::FencedOut { .. }) => Err(EngineError::Leased {
                execution_id,
                holder: "another runtime owner or expired lease".to_owned(),
            }
            .into()),
            Ok(ControlTurnCommitOutcome::FlavorMismatch {
                expected, actual, ..
            }) => {
                tracing::Span::current()
                    .record("expected_flavor", tracing::field::display(expected));
                tracing::Span::current().record("actual_flavor", tracing::field::display(actual));
                Err(EngineError::InvalidRecordedContract.into())
            },
            Ok(ControlTurnCommitOutcome::VersionConflict { actual, .. }) => {
                Err(EngineError::ControlTurnVersionConflict {
                    expected: *repo_version,
                    actual,
                }
                .into())
            },
            Ok(_) => Err(ControlCommitFailure::AcceptanceUnknown(
                EngineError::ControlTurnInterrupted,
            )),
            Err(source @ nebula_storage_port::StorageError::AcknowledgementUnknown { .. }) => Err(
                ControlCommitFailure::AcceptanceUnknown(EngineError::ControlTurnHandoff { source }),
            ),
            Err(source) => Err(EngineError::ControlTurnHandoff { source }.into()),
        }
    }
}
