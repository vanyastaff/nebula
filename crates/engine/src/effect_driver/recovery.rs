//! Database-only outcome acknowledgement recovery and separate read-only queries.

use super::*;

impl Driver<'_, '_> {
    pub(super) async fn commit_evidence(
        &mut self,
        evidence: FrozenOutcomeEvidence,
    ) -> Result<ActionResult<Value>, EffectExecutionError> {
        let command = OperationCommand::RecordOutcome(evidence.clone());
        let mut last_error = OperationLedgerError::Unavailable;
        let mut acknowledgement_unknown = false;
        for _ in 0..2 {
            match self.advance(&command).await {
                Ok(None) => {
                    if self.protocol()?.evidence() != Some(&evidence) {
                        return Err(EffectExecutionError::InvalidEvidence);
                    }
                    return evidence::replay(self.operation_id(), &evidence);
                },
                Ok(Some(_)) => return Err(EffectExecutionError::InvalidEvidence),
                Err(EffectExecutionError::Ledger(
                    error @ (OperationLedgerError::Unavailable
                    | OperationLedgerError::AcknowledgementUnknown),
                )) => {
                    acknowledgement_unknown |=
                        error == OperationLedgerError::AcknowledgementUnknown;
                    last_error = error;
                    let record = self
                        .turn
                        .ledger
                        .read_exact(self.turn.scope, self.record.operation().slot_id())
                        .await
                        .map_err(|error| {
                            if acknowledgement_unknown {
                                OperationLedgerError::AcknowledgementUnknown
                            } else {
                                error
                            }
                        })?;
                    self.accept(record)?;
                    if let Some(recorded) = self.protocol()?.evidence() {
                        if recorded != &evidence {
                            return Err(EffectExecutionError::InvalidEvidence);
                        }
                        return evidence::replay(self.operation_id(), recorded);
                    }
                    // Only this exact immutable command can be retried. No path
                    // from outcome acknowledgement recovery invokes a provider.
                },
                Err(error) => return Err(error),
            }
        }
        Err(if acknowledgement_unknown {
            OperationLedgerError::AcknowledgementUnknown
        } else {
            last_error
        }
        .into())
    }

    pub(super) async fn reconcile(&mut self) -> Result<ActionResult<Value>, EffectExecutionError> {
        if self.turn.cancellation.is_cancelled() {
            return Err(EffectExecutionError::Cancelled);
        }
        if self.prepared.adapter().read_only_query().is_none() {
            return Err(EffectExecutionError::OutcomeUnknown {
                operation_id: self.operation_id(),
            });
        }
        let revision = self.protocol()?.revision();
        let call = match self
            .advance(&OperationCommand::GrantReconciliation {
                expected_revision: revision,
            })
            .await
        {
            Ok(Some(GrantedCall::Reconciliation(call))) => call,
            Err(EffectExecutionError::Ledger(OperationLedgerError::RecoveryExhausted)) => {
                return Err(EffectExecutionError::OutcomeUnknown {
                    operation_id: self.operation_id(),
                });
            },
            Err(error) => return Err(error),
            _ => return Err(EffectExecutionError::InvalidEvidence),
        };
        let deadline = self.deadline(CallPurpose::Reconciliation)?;
        let remaining = deadline.saturating_sub(self.turn.clock.now().timestamp_millis());
        if remaining <= 0 {
            self.advance(&OperationCommand::RecordReconciliationInconclusive { query: call })
                .await?;
            return Err(EffectExecutionError::OutcomeUnknown {
                operation_id: self.operation_id(),
            });
        }
        let context = QueryGrant {
            operation_id: self.operation_id(),
            call,
            deadline,
            cancellation: self.turn.cancellation.child_token(),
        };
        let query = self
            .prepared
            .adapter()
            .read_only_query()
            .ok_or(EffectExecutionError::InvalidContract)?;
        let timeout = Duration::from_millis(
            u64::try_from(remaining).map_err(|_| EffectExecutionError::InvalidEvidence)?,
        );
        let outcome = tokio::select! {
            biased;
            () = self.turn.cancellation.cancelled() => EffectReconciliationOutcome::Inconclusive,
            result = tokio::time::timeout(timeout, AssertUnwindSafe(query.reconcile(&context)).catch_unwind()) => {
                match result { Ok(Ok(outcome)) => outcome, _ => EffectReconciliationOutcome::Inconclusive }
            },
        };
        let source = OutcomeEvidenceSource::Reconciliation(call);
        match outcome {
            EffectReconciliationOutcome::Applied(output) => {
                self.commit_evidence(evidence::applied(
                    self.operation_id(),
                    source,
                    Some(output),
                )?)
                .await
            },
            EffectReconciliationOutcome::AppliedWithoutOutput => {
                self.commit_evidence(evidence::applied(self.operation_id(), source, None)?)
                    .await
            },
            EffectReconciliationOutcome::Rejected(code) => {
                self.commit_evidence(evidence::rejected(self.operation_id(), source, code)?)
                    .await
            },
            EffectReconciliationOutcome::Inconclusive => {
                self.advance(&OperationCommand::RecordReconciliationInconclusive { query: call })
                    .await?;
                Err(EffectExecutionError::OutcomeUnknown {
                    operation_id: self.operation_id(),
                })
            },
            _ => Err(EffectExecutionError::InvalidEvidence),
        }
    }
}

struct QueryGrant {
    operation_id: OperationId,
    call: OperationCallId,
    deadline: i64,
    cancellation: CancellationToken,
}

impl EffectQueryContext for QueryGrant {
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
