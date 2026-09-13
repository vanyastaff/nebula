//! Execution-owned driver for one explicit remote effect occurrence.

mod error;
mod evidence;
mod recovery;

pub use error::EffectExecutionError;

use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures::FutureExt;
use nebula_action::{
    ActionResult, PreparedActionInput,
    effect::{
        EffectInvocationContext, EffectInvocationOutcome, EffectPreparationContext,
        EffectPreparationError, EffectQueryContext, EffectReconciliationOutcome,
        PreparedRemoteEffect, RemoteDestinationGuarantee, RemoteEffectDescriptor,
        RemoteEffectFactory, RemoteEffectPolicy,
    },
};
use nebula_core::{
    ExecutionId, NodeKey, OperationCallId, OperationId, WorkflowId, accessor::Clock,
};
use nebula_storage_port::{
    FencingToken, Scope,
    dto::{
        AttemptGeneration, DestinationCapability, EffectOccurrenceKey, EffectPhase,
        EffectSlotBinding, FrozenOutcomeEvidence, InvocationDisposition, OperationAdvance,
        OperationCommand, OperationLedgerError, OperationRecord, OutcomeEvidenceSource,
        PreparedEffectContract, PreparedEffectPolicy, RequestFingerprint,
    },
    store::OperationLedger,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const MAX_EFFECT_PREPARATION_TIME: Duration = Duration::from_secs(30);

/// Authority borrowed from the active execution-owning turn.
pub(crate) struct EffectTurn<'a> {
    pub ledger: &'a dyn OperationLedger,
    pub scope: &'a Scope,
    pub fencing: FencingToken,
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub node_key: &'a NodeKey,
    pub action_key: &'a str,
    pub action_version: &'a semver::Version,
    pub attempt_generation: u64,
    pub clock: &'a dyn Clock,
    pub cancellation: &'a CancellationToken,
}

impl EffectTurn<'_> {
    #[tracing::instrument(skip_all, fields(
        execution_id = %self.execution_id, node_key = %self.node_key,
        operation_id = tracing::field::Empty,
        phase = tracing::field::Empty,
    ))]
    pub(crate) async fn execute(
        &self,
        factory: &dyn RemoteEffectFactory,
        descriptor: &RemoteEffectDescriptor,
        input: PreparedActionInput,
    ) -> Result<ActionResult<Value>, EffectExecutionError> {
        descriptor
            .validate()
            .map_err(|_| EffectExecutionError::InvalidContract)?;
        let preparation = EffectPreparationContext::new(
            self.execution_id,
            self.workflow_id,
            self.node_key.clone(),
            self.scope
                .org_id
                .parse()
                .map_err(|_| EffectExecutionError::InvalidContract)?,
            self.scope
                .workspace_id
                .parse()
                .map_err(|_| EffectExecutionError::InvalidContract)?,
        );
        // Preparation has no invocation authority. Bound even a faulty trusted
        // adapter so it cannot keep an execution turn alive indefinitely.
        let preparation_limit = Duration::from_millis(descriptor.policy().recovery_window_ms())
            .min(MAX_EFFECT_PREPARATION_TIME);
        let prepared = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => return Err(EffectExecutionError::Cancelled),
            result = tokio::time::timeout(preparation_limit, AssertUnwindSafe(factory.prepare(input, &preparation)).catch_unwind()) => {
                match result {
                    Ok(Ok(prepared)) => prepared?,
                    _ => return Err(EffectPreparationError::Unavailable.into()),
                }
            },
        };
        if (descriptor.policy().max_queries() > 0) != prepared.adapter().read_only_query().is_some()
        {
            return Err(EffectExecutionError::InvalidContract);
        }
        let (contract, fingerprint) = self.binding(descriptor, &prepared)?;
        let execution = self.execution_id.to_string();
        // This version admits exactly one stateless business effect per node.
        // Node identity is already part of the natural address; retries never
        // alter the logical occurrence or mint a second slot.
        let occurrence = "node-effect/v1";
        let binding = EffectSlotBinding {
            scope: self.scope,
            execution_id: &execution,
            node_key: self.node_key.as_str(),
            occurrence,
            attempt_generation: AttemptGeneration::new(self.attempt_generation),
            fingerprint,
            destination: contract.policy().capability(),
            contract: &contract,
        };
        let record = match self.ledger.prepare(&binding, self.fencing).await {
            Ok(outcome) => {
                let record = self
                    .ledger
                    .read_exact(self.scope, outcome.operation().slot_id())
                    .await?;
                if record.operation() != outcome.operation() {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                record
            },
            Err(OperationLedgerError::AcknowledgementUnknown) => self
                .ledger
                .read_occurrence(&EffectOccurrenceKey::new(
                    self.scope,
                    &execution,
                    self.node_key.as_str(),
                    occurrence,
                ))
                .await
                .map_err(|_| OperationLedgerError::AcknowledgementUnknown)?
                .ok_or(OperationLedgerError::AcknowledgementUnknown)?,
            Err(error) => return Err(error.into()),
        };
        let mut driver = Driver {
            turn: self,
            prepared,
            contract,
            fingerprint,
            record,
        };
        driver.validate_record(&driver.record)?;
        tracing::Span::current().record(
            "operation_id",
            tracing::field::display(driver.operation_id()),
        );
        driver.run().await
    }

    fn binding(
        &self,
        descriptor: &RemoteEffectDescriptor,
        request: &PreparedRemoteEffect,
    ) -> Result<(PreparedEffectContract, RequestFingerprint), EffectExecutionError> {
        let mut identity = Sha256::new();
        frame(&mut identity, b"nebula.remote-effect.destination.v1")?;
        frame(&mut identity, self.action_key.as_bytes())?;
        frame(&mut identity, self.action_version.to_string().as_bytes())?;
        frame(&mut identity, descriptor.contract_id().as_bytes())?;
        identity.update(descriptor.canonicalization_version().to_be_bytes());
        let policy = PreparedEffectPolicyProjection::try_from(descriptor.policy())?.0;
        identity.update([capability_discriminant(policy.capability())?]);
        identity.update(policy.max_invocations().to_be_bytes());
        identity.update(policy.max_queries().to_be_bytes());
        identity.update(policy.recovery_window_ms().to_be_bytes());
        match policy.stable_window_ms() {
            Some(window) => {
                identity.update([1]);
                identity.update(window.to_be_bytes());
            },
            None => identity.update([0]),
        }
        frame(&mut identity, request.destination_binding())?;
        let identity: [u8; 32] = identity.finalize().into();
        let contract = PreparedEffectContract::new(RequestFingerprint::new(1, identity), policy)?;
        let mut canonical = Sha256::new();
        frame(&mut canonical, b"nebula.remote-effect.request.v1")?;
        canonical.update(identity);
        frame(&mut canonical, request.canonical_request())?;
        Ok((
            contract,
            RequestFingerprint::new(1, canonical.finalize().into()),
        ))
    }
}

struct PreparedEffectPolicyProjection(PreparedEffectPolicy);

impl TryFrom<&RemoteEffectPolicy> for PreparedEffectPolicyProjection {
    type Error = EffectExecutionError;

    fn try_from(declared: &RemoteEffectPolicy) -> Result<Self, Self::Error> {
        let (capability, stable_window_ms) = match declared.destination_guarantee() {
            RemoteDestinationGuarantee::StableKey(guarantee) => (
                DestinationCapability::StableKey,
                Some(guarantee.validity_window_ms()),
            ),
            RemoteDestinationGuarantee::Reconcilable => (DestinationCapability::Reconcilable, None),
            RemoteDestinationGuarantee::Opaque => (DestinationCapability::Opaque, None),
            _ => return Err(EffectExecutionError::InvalidContract),
        };
        let mut builder = PreparedEffectPolicy::builder(capability)
            .maximum_invocations(declared.max_invocations())
            .maximum_queries(declared.max_queries())
            .recovery_window(Duration::from_millis(declared.recovery_window_ms()));
        if let Some(stable_window_ms) = stable_window_ms {
            builder = builder.stable_key_window(Duration::from_millis(stable_window_ms));
        }
        builder
            .build()
            .map(Self)
            .map_err(|_| EffectExecutionError::InvalidContract)
    }
}

const fn capability_discriminant(
    capability: DestinationCapability,
) -> Result<u8, EffectExecutionError> {
    match capability {
        DestinationCapability::Opaque => Ok(0),
        DestinationCapability::StableKey => Ok(1),
        DestinationCapability::Reconcilable => Ok(2),
        _ => Err(EffectExecutionError::InvalidContract),
    }
}

fn frame(digest: &mut Sha256, bytes: &[u8]) -> Result<(), EffectExecutionError> {
    let length = u64::try_from(bytes.len()).map_err(|_| EffectExecutionError::InvalidContract)?;
    digest.update(length.to_be_bytes());
    digest.update(bytes);
    Ok(())
}

struct Driver<'a, 'turn> {
    turn: &'a EffectTurn<'turn>,
    prepared: PreparedRemoteEffect,
    contract: PreparedEffectContract,
    fingerprint: RequestFingerprint,
    record: OperationRecord,
}

enum GrantedCall {
    Invocation {
        call: OperationCallId,
        authorized_at_ms: i64,
        request_started: Instant,
    },
    Reconciliation {
        call: OperationCallId,
        authorized_at_ms: i64,
        request_started: Instant,
    },
}

enum CallPurpose {
    Invocation,
    Reconciliation,
}

impl Driver<'_, '_> {
    fn operation_id(&self) -> OperationId {
        self.record.operation().operation_id()
    }

    fn validate_record(&self, record: &OperationRecord) -> Result<(), EffectExecutionError> {
        let protocol = record
            .protocol()
            .ok_or(EffectExecutionError::InvalidEvidence)?;
        if record.operation() != self.record.operation()
            || record.fingerprint() != self.fingerprint
            || protocol.contract() != &self.contract
            || record.operation().destination() != self.contract.policy().capability()
        {
            return Err(EffectExecutionError::InvalidEvidence);
        }
        protocol.validate()?;
        if protocol.invocations() > self.contract.policy().max_invocations()
            || protocol.queries() > self.contract.policy().max_queries()
            || (protocol.phase() == EffectPhase::Resolved) != protocol.evidence().is_some()
        {
            return Err(EffectExecutionError::InvalidEvidence);
        }
        if let Some(evidence) = protocol.evidence() {
            evidence
                .validate()
                .map_err(|_| EffectExecutionError::InvalidEvidence)?;
        }
        Ok(())
    }

    fn accept(&mut self, record: OperationRecord) -> Result<(), EffectExecutionError> {
        self.validate_record(&record)?;
        self.record = record;
        Ok(())
    }

    async fn advance(
        &mut self,
        command: &OperationCommand,
    ) -> Result<Option<GrantedCall>, EffectExecutionError> {
        let previous = self.protocol()?;
        let revision = previous.revision();
        let invocations = previous.invocations();
        let queries = previous.queries();
        let request_started = self.turn.clock.monotonic();
        let advance = self
            .turn
            .ledger
            .advance(
                self.turn.scope,
                self.record.operation().slot_id(),
                self.turn.fencing,
                command,
            )
            .await?;
        match advance {
            OperationAdvance::Granted {
                call,
                authorized_at_ms,
                record,
            } => {
                let protocol = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?;
                if Some(protocol.revision()) != revision.checked_add(1)
                    || Some(protocol.invocations()) != invocations.checked_add(1)
                    || protocol.queries() != queries
                    || protocol.phase() != EffectPhase::InvocationOutstanding
                    || protocol.invocation() != Some(call)
                    || protocol.query().is_some()
                {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(Some(GrantedCall::Invocation {
                    call,
                    authorized_at_ms,
                    request_started,
                }))
            },
            OperationAdvance::ReconciliationGranted {
                call,
                authorized_at_ms,
                record,
            } => {
                let protocol = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?;
                if Some(protocol.revision()) != revision.checked_add(1)
                    || Some(protocol.queries()) != queries.checked_add(1)
                    || protocol.invocations() != invocations
                    || protocol.query() != Some(call)
                    || protocol.phase() != EffectPhase::OutcomeUnknown
                {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(Some(GrantedCall::Reconciliation {
                    call,
                    authorized_at_ms,
                    request_started,
                }))
            },
            OperationAdvance::Recorded(record) => {
                let recorded_revision = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?
                    .revision();
                if !recorded_revision_is_valid(command, revision, recorded_revision) {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(None)
            },
            _ => Err(EffectExecutionError::InvalidEvidence),
        }
    }

    fn protocol(
        &self,
    ) -> Result<&nebula_storage_port::dto::OperationProtocolRecord, EffectExecutionError> {
        self.record
            .protocol()
            .ok_or(EffectExecutionError::InvalidEvidence)
    }

    fn deadline(&self, purpose: CallPurpose) -> Result<i64, EffectExecutionError> {
        let protocol = self.protocol()?;
        let policy = self.contract.policy();
        let window = match purpose {
            CallPurpose::Reconciliation => policy.recovery_window_ms(),
            CallPurpose::Invocation => policy.stable_window_ms().map_or_else(
                || policy.recovery_window_ms(),
                |stable| stable.min(policy.recovery_window_ms()),
            ),
        };
        protocol
            .prepared_at_ms()
            .checked_add(i64::try_from(window).map_err(|_| EffectExecutionError::InvalidContract)?)
            .ok_or(EffectExecutionError::InvalidEvidence)
    }

    fn call_timing(
        &self,
        purpose: CallPurpose,
        authorized_at_ms: i64,
        request_started: Instant,
    ) -> Result<(i64, Duration), EffectExecutionError> {
        let deadline = self.deadline(purpose)?;
        let remaining = remaining_call_budget(
            deadline,
            authorized_at_ms,
            request_started,
            self.turn.clock.monotonic(),
        )
        .ok_or(EffectExecutionError::InvalidEvidence)?;
        Ok((deadline, remaining))
    }

    async fn mark_unknown(&mut self) -> Result<(), EffectExecutionError> {
        let revision = self.protocol()?.revision();
        self.advance(&OperationCommand::MarkUnknown {
            expected_revision: revision,
        })
        .await?;
        Ok(())
    }

    async fn run(&mut self) -> Result<ActionResult<Value>, EffectExecutionError> {
        loop {
            let protocol = self.protocol()?;
            tracing::Span::current().record("phase", tracing::field::debug(protocol.phase()));
            match protocol.phase() {
                EffectPhase::Resolved => {
                    return evidence::replay(
                        self.operation_id(),
                        protocol
                            .evidence()
                            .ok_or(EffectExecutionError::InvalidEvidence)?,
                    );
                },
                EffectPhase::InvocationOutstanding => {
                    // Crash residue is indistinguishable from a lost outcome ACK.
                    // Even a stable destination receives no second effect call.
                    self.mark_unknown().await?;
                },
                EffectPhase::OutcomeUnknown => return self.reconcile().await,
                EffectPhase::Prepared | EffectPhase::BeforeBoundary | EffectPhase::Ambiguous => {
                    if self.turn.cancellation.is_cancelled() {
                        return Err(EffectExecutionError::Cancelled);
                    }
                    let revision = protocol.revision();
                    let grant = self
                        .advance(&OperationCommand::GrantInvocation {
                            expected_revision: revision,
                        })
                        .await?;
                    let Some(GrantedCall::Invocation {
                        call,
                        authorized_at_ms,
                        request_started,
                    }) = grant
                    else {
                        if grant.is_some() {
                            return Err(EffectExecutionError::InvalidEvidence);
                        }
                        continue;
                    };
                    let (deadline, timeout) = self.call_timing(
                        CallPurpose::Invocation,
                        authorized_at_ms,
                        request_started,
                    )?;
                    if timeout.is_zero() {
                        self.mark_unknown().await?;
                        continue;
                    }
                    let context = InvocationGrant {
                        operation_id: self.operation_id(),
                        call,
                        deadline,
                        cancellation: self.turn.cancellation.child_token(),
                    };
                    let outcome = tokio::select! {
                        biased;
                        () = self.turn.cancellation.cancelled() => EffectInvocationOutcome::Ambiguous,
                        result = tokio::time::timeout(timeout, AssertUnwindSafe(self.prepared.adapter().invoke(&context)).catch_unwind()) => {
                            match result { Ok(Ok(outcome)) => outcome, _ => EffectInvocationOutcome::Ambiguous }
                        },
                    };
                    let source = OutcomeEvidenceSource::Invocation(call);
                    match outcome {
                        EffectInvocationOutcome::Applied(output) => {
                            return self
                                .commit_evidence(evidence::applied(
                                    self.operation_id(),
                                    source,
                                    Some(output),
                                )?)
                                .await;
                        },
                        EffectInvocationOutcome::AppliedWithoutOutput => {
                            return self
                                .commit_evidence(evidence::applied(
                                    self.operation_id(),
                                    source,
                                    None,
                                )?)
                                .await;
                        },
                        EffectInvocationOutcome::Rejected(code) => {
                            return self
                                .commit_evidence(evidence::rejected(
                                    self.operation_id(),
                                    source,
                                    code,
                                )?)
                                .await;
                        },
                        EffectInvocationOutcome::BeforeBoundary(_) => {
                            self.advance(&OperationCommand::RecordDisposition {
                                invocation: call,
                                disposition: InvocationDisposition::BeforeBoundary,
                            })
                            .await?;
                        },
                        EffectInvocationOutcome::Ambiguous => {
                            self.advance(&OperationCommand::RecordDisposition {
                                invocation: call,
                                disposition: InvocationDisposition::Ambiguous,
                            })
                            .await?;
                        },
                        _ => {
                            self.mark_unknown().await?;
                        },
                    }
                },
                _ => return Err(EffectExecutionError::InvalidEvidence),
            }
        }
    }
}

fn remaining_call_budget(
    deadline_ms: i64,
    authorized_at_ms: i64,
    request_started: Instant,
    grant_received: Instant,
) -> Option<Duration> {
    let backend_budget_ms = deadline_ms.checked_sub(authorized_at_ms)?;
    let backend_budget = Duration::from_millis(u64::try_from(backend_budget_ms).ok()?);
    let storage_round_trip = grant_received.checked_duration_since(request_started)?;
    Some(backend_budget.saturating_sub(storage_round_trip))
}

fn recorded_revision_is_valid(
    command: &OperationCommand,
    previous_revision: u64,
    recorded_revision: u64,
) -> bool {
    let Some(next_revision) = previous_revision.checked_add(1) else {
        return false;
    };
    let command_requires_progress = matches!(
        command,
        OperationCommand::GrantInvocation { .. } | OperationCommand::GrantReconciliation { .. }
    );
    recorded_revision == next_revision
        || (!command_requires_progress && recorded_revision == previous_revision)
}

struct InvocationGrant {
    operation_id: OperationId,
    call: OperationCallId,
    deadline: i64,
    cancellation: CancellationToken,
}

impl EffectInvocationContext for InvocationGrant {
    fn operation_id(&self) -> OperationId {
        self.operation_id
    }
    fn call_id(&self) -> OperationCallId {
        self.call
    }
    fn deadline_unix_ms(&self) -> i64 {
        self.deadline
    }
    fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_timeout_uses_backend_budget_and_monotonic_elapsed() {
        let request_started = Instant::now();
        let grant_received = request_started + Duration::from_millis(100);

        assert_eq!(
            remaining_call_budget(2_000, 1_000, request_started, grant_received),
            Some(Duration::from_millis(900)),
        );
    }

    #[test]
    fn expired_backend_grant_has_no_provider_call_budget() {
        let request_started = Instant::now();

        assert_eq!(
            remaining_call_budget(1_000, 1_000, request_started, request_started),
            Some(Duration::ZERO),
        );
    }

    #[test]
    fn recorded_grant_must_advance_the_protocol_revision() {
        assert!(!recorded_revision_is_valid(
            &OperationCommand::GrantInvocation {
                expected_revision: 7,
            },
            7,
            7,
        ));
    }

    #[test]
    fn recorded_idempotent_transition_may_retain_the_protocol_revision() {
        assert!(recorded_revision_is_valid(
            &OperationCommand::RecordDisposition {
                invocation: OperationCallId::from_bytes([3; 16]),
                disposition: InvocationDisposition::BeforeBoundary,
            },
            7,
            7,
        ));
    }
}
