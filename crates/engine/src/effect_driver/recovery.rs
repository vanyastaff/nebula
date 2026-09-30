//! Separate read-only queries of an unknown remote outcome.
//!
//! Outcome acknowledgement recovery is the shared
//! [`LedgerSlot::commit_evidence`].

use super::*;

impl Driver<'_, '_> {
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
            Ok(Some(GrantedCall::Reconciliation {
                call,
                authorized_at_ms,
                request_started,
            })) => (call, authorized_at_ms, request_started),
            Err(EffectExecutionError::Ledger(OperationLedgerError::RecoveryExhausted)) => {
                return Err(EffectExecutionError::OutcomeUnknown {
                    operation_id: self.operation_id(),
                });
            },
            Err(error) => return Err(error),
            _ => return Err(EffectExecutionError::InvalidEvidence),
        };
        let (call, authorized_at_ms, request_started) = call;
        let (deadline, timeout) = self.call_timing(
            CallPurpose::Reconciliation,
            authorized_at_ms,
            request_started,
        )?;
        if timeout.is_zero() {
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
