//! Durable ownership handoffs that precede execution recovery.

use super::*;
use nebula_storage_port::store::ControlClaimToken;

/// A discovered accepted turn and the owner capability that may grant recovery.
#[derive(Clone, Debug)]
pub struct RecoveryTurnRequest<'a> {
    /// Owner of the durable acceptance marker and execution lease.
    pub handoff: &'a dyn nebula_storage_port::TurnRecovery,
    /// New worker identity, used only if recovery is granted.
    pub holder: &'a str,
    /// Initial duration of a freshly granted lease.
    pub lease_ttl: Duration,
    /// Accepted generation observed during discovery; never itself authority.
    pub accepted_fencing_generation: u64,
}

/// Recovery decision, preserving whether the owner committed a new turn.
#[must_use]
pub enum RecoveryTurnOutcome {
    /// Terminal, cancelled, or waiting for a condition that is not ready.
    NotReady,
    /// A newer accepted generation superseded this discovery result.
    CandidateSuperseded,
    /// No recovery lease was granted.
    NotAccepted(EngineError),
    /// The owner may have committed, but no fence was acknowledged.
    AcceptanceUnknown(EngineError),
    /// Acknowledged ownership followed by the execution result.
    Accepted(Result<ExecutionResult, EngineError>),
}

impl std::fmt::Debug for RecoveryTurnOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotReady => "NotReady",
            Self::CandidateSuperseded => "CandidateSuperseded",
            Self::NotAccepted(_) => "NotAccepted(<redacted>)",
            Self::AcceptanceUnknown(_) => "AcceptanceUnknown(<redacted>)",
            Self::Accepted(Ok(_)) => "Accepted(Ok(<redacted>))",
            Self::Accepted(Err(_)) => "Accepted(Err(<redacted>))",
        })
    }
}

/// Current Start delivery and the execution owner's atomic handoff capability.
#[derive(Clone, Debug)]
pub struct ClaimedStartRequest<'a> {
    /// Proof of the current queue claim.
    pub claim: ControlClaimToken,
    /// Backend owning both the control queue and execution aggregate.
    pub handoff: &'a dyn nebula_storage_port::ExecutionTurnHandoff,
    /// Worker identity recorded on the accepted lease.
    pub holder: &'a str,
    /// Initial lease duration; the engine renews the accepted fence.
    pub lease_ttl: Duration,
}

/// Whether delivery ended before driving a claimed Start.
#[must_use]
pub enum ClaimedStartOutcome {
    /// Preflight or ownership rejection; delivery remains the caller's concern.
    NotAccepted(EngineError),
    /// Another claim superseded this delivery; the caller must leave it untouched.
    ClaimSuperseded,
    /// Commit acknowledgement was lost; no drive or queue mutation is safe.
    AcceptanceUnknown(EngineError),
    /// Delivery is complete, including when the accepted execution turn fails.
    Accepted(Result<ExecutionResult, EngineError>),
}

impl std::fmt::Debug for ClaimedStartOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotAccepted(_) => "NotAccepted(<redacted>)",
            Self::ClaimSuperseded => "ClaimSuperseded",
            Self::AcceptanceUnknown(_) => "AcceptanceUnknown(<redacted>)",
            Self::Accepted(Ok(_)) => "Accepted(Ok(<redacted>))",
            Self::Accepted(Err(_)) => "Accepted(Err(<redacted>))",
        })
    }
}

impl WorkflowEngine {
    /// Ask the control owner to observe an existing execution's actual runtime mismatch.
    /// This runs before the exact loader rejects the incompatible frozen registry.
    #[tracing::instrument(skip_all, fields(%execution_id, org_id = %scope.org_id, workspace_id = %scope.workspace_id, backend = tracing::field::Empty, outcome = tracing::field::Empty, observation_acknowledgement = tracing::field::Empty, expected_flavor = tracing::field::Empty, actual_flavor = tracing::field::Empty))]
    pub(super) async fn record_claimed_flavor_refusal(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        expected: WorkerFlavorRevisionId,
        source: &ResumeLeaseSource<'_>,
    ) -> Result<(), ExactTurnFailure> {
        use nebula_storage_port::store::{
            ControlFlavorRefusal, ControlFlavorRefusalOutcome, ControlObservationAcknowledgement,
        };
        let Some(context) = self.worker_flavor_context() else {
            return Ok(());
        };
        let actual = context.revision_id();
        if actual == expected {
            return Ok(());
        }
        let (claim, handoff) = match source {
            ResumeLeaseSource::ControlStart(request) => (&request.claim, request.handoff),
            ResumeLeaseSource::Control(request) => (&request.claim, request.handoff.as_ref()),
            _ => return Ok(()),
        };
        if claim.scope() != scope {
            return Err(EngineError::InvalidRecordedExecution.into());
        }
        let execution_key = execution_id.to_string();
        let refusal = ControlFlavorRefusal::new(claim, &execution_key, actual);
        let decision = handoff.record_control_flavor_refusal(&refusal).await;
        match decision {
            Ok(ControlFlavorRefusalOutcome::Recorded {
                expected,
                actual,
                backend,
            }) => {
                tracing::Span::current().record("observation_acknowledgement", "recorded");
                tracing::Span::current()
                    .record("expected_flavor", tracing::field::display(expected));
                tracing::Span::current().record("actual_flavor", tracing::field::display(actual));
                if let Err(error) = crate::control_metrics::record_execution_control_outcome(
                    &self.metrics,
                    backend,
                    nebula_execution::ExecutionControlOutcome::FlavorMismatch,
                ) {
                    tracing::warn!(%error, "execution flavor refusal metric could not be recorded");
                }
                // Keep exact-load validation authoritative for the typed rejection.
                // Recording the owner's refusal grants no lease or permission to drive.
                Ok(())
            },
            Ok(ControlFlavorRefusalOutcome::AlreadyRecorded {
                expected,
                actual,
                backend,
            }) => {
                let span = tracing::Span::current();
                span.record("observation_acknowledgement", "already_recorded");
                span.record("expected_flavor", tracing::field::display(expected));
                span.record("actual_flavor", tracing::field::display(actual));
                span.record("backend", backend.as_str());
                span.record("outcome", "flavor-mismatch");
                Ok(())
            },
            Ok(ControlFlavorRefusalOutcome::ClaimFenced {
                observation_acknowledgement,
                ..
            }) => {
                let backend = handoff.backend_kind();
                let span = tracing::Span::current();
                span.record(
                    "observation_acknowledgement",
                    observation_acknowledgement.as_str(),
                );
                span.record("backend", backend.as_str());
                span.record("outcome", "fenced");
                if observation_acknowledgement == ControlObservationAcknowledgement::Recorded
                    && let Err(error) = crate::control_metrics::record_execution_control_outcome(
                        &self.metrics,
                        backend,
                        nebula_execution::ExecutionControlOutcome::Fenced,
                    )
                {
                    tracing::warn!(%error, "execution claim refusal metric could not be recorded");
                }
                Err(ExactTurnFailure::ClaimSuperseded)
            },
            Ok(ControlFlavorRefusalOutcome::ClaimSuperseded) => {
                Err(ExactTurnFailure::ClaimSuperseded)
            },
            Ok(ControlFlavorRefusalOutcome::NoMismatch) => Ok(()),
            Ok(_) => Err(ExactTurnFailure::AcceptanceUnknown(
                EngineError::ControlTurnInterrupted,
            )),
            Err(source @ nebula_storage_port::StorageError::AcknowledgementUnknown { .. }) => Err(
                ExactTurnFailure::AcceptanceUnknown(EngineError::ControlTurnHandoff { source }),
            ),
            Err(source) => Err(EngineError::ControlTurnHandoff { source }.into()),
        }
    }

    /// Atomically accept a claimed Resume or Restart under its execution owner.
    /// A live owner replies after the command commit, independently of action
    /// duration; a recovered owner commits the intent before continuing work.
    pub async fn resume_claimed_control_turn(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        request: ClaimedControlTurnRequest,
    ) -> ClaimedControlTurnOutcome {
        if let Some(outcome) = self.deliver_claimed_control(execution_id, &request).await {
            return outcome;
        }
        match self
            .drive_exact_execution(scope, execution_id, ResumeLeaseSource::Control(&request))
            .await
        {
            Ok(_) => ClaimedControlTurnOutcome::Accepted(Ok(())),
            Err(ExactTurnFailure::BeforeLease(error)) => {
                ClaimedControlTurnOutcome::NotAccepted(error)
            },
            Err(ExactTurnFailure::AfterLease(error)) => {
                ClaimedControlTurnOutcome::Accepted(Err(error))
            },
            Err(ExactTurnFailure::AcceptanceUnknown(error)) => {
                ClaimedControlTurnOutcome::AcceptanceUnknown(error)
            },
            Err(ExactTurnFailure::ClaimSuperseded | ExactTurnFailure::CandidateSuperseded) => {
                ClaimedControlTurnOutcome::ClaimSuperseded
            },
            Err(ExactTurnFailure::NotReady) => {
                ClaimedControlTurnOutcome::NotAccepted(EngineError::InvalidRecordedExecution)
            },
        }
    }

    /// Recover a discovered accepted turn only after exact preflight and a
    /// fresh atomic owner grant. Discovery never authorizes action invocation.
    pub async fn resume_recoverable_turn(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        request: RecoveryTurnRequest<'_>,
    ) -> RecoveryTurnOutcome {
        match self
            .drive_exact_execution(scope, execution_id, ResumeLeaseSource::Recovery(request))
            .await
        {
            Ok(result) => RecoveryTurnOutcome::Accepted(Ok(result)),
            Err(ExactTurnFailure::BeforeLease(error)) => RecoveryTurnOutcome::NotAccepted(error),
            Err(ExactTurnFailure::AfterLease(error)) => RecoveryTurnOutcome::Accepted(Err(error)),
            Err(ExactTurnFailure::AcceptanceUnknown(error)) => {
                RecoveryTurnOutcome::AcceptanceUnknown(error)
            },
            Err(ExactTurnFailure::NotReady) => RecoveryTurnOutcome::NotReady,
            Err(ExactTurnFailure::ClaimSuperseded | ExactTurnFailure::CandidateSuperseded) => {
                RecoveryTurnOutcome::CandidateSuperseded
            },
        }
    }

    /// Validate the exact stored execution, then atomically end Start delivery
    /// and acquire its lease before invoking any action.
    ///
    /// The returned phase is authoritative for queue handling: accepted and
    /// uncertain outcomes must never acknowledge, fail, or release the claim.
    pub async fn resume_control_start(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        request: ClaimedStartRequest<'_>,
    ) -> ClaimedStartOutcome {
        match self
            .drive_exact_execution(
                scope,
                execution_id,
                ResumeLeaseSource::ControlStart(request),
            )
            .await
        {
            Ok(result) => ClaimedStartOutcome::Accepted(Ok(result)),
            Err(ExactTurnFailure::BeforeLease(error)) => ClaimedStartOutcome::NotAccepted(error),
            Err(ExactTurnFailure::AfterLease(error)) => ClaimedStartOutcome::Accepted(Err(error)),
            Err(ExactTurnFailure::AcceptanceUnknown(error)) => {
                ClaimedStartOutcome::AcceptanceUnknown(error)
            },
            Err(ExactTurnFailure::ClaimSuperseded) => ClaimedStartOutcome::ClaimSuperseded,
            Err(ExactTurnFailure::NotReady | ExactTurnFailure::CandidateSuperseded) => {
                ClaimedStartOutcome::NotAccepted(EngineError::InvalidRecordedExecution)
            },
        }
    }
}
