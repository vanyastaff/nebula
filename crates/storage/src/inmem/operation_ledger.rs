//! In-memory operation ledger — the reference/conformance model.
//!
//! Every operation runs inside one `parking_lot` mutex critical section, the
//! in-memory equivalent of the SQL backends' single transaction. The mutex is
//! not a transaction: there is no rollback, so each operation decides
//! everything it needs *before* it writes, and the writes that follow are
//! infallible. A prepare that failed halfway would otherwise leave a slot
//! bound to a request the caller was told was rejected.
//!
//! Identities are minted here, never accepted from a caller: a caller able to
//! choose a slot identity could merge two intended occurrences into one.

use std::collections::HashMap;

use nebula_core::{OperationCallId, OperationId};
use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    EffectOccurrenceKey, EffectSlotBinding, EffectSlotId, FencingToken, OperationLedgerError,
    OperationRecord, OperationState, PrepareOutcome, Scope,
};

use crate::operation_ledger::{
    compose_record, decide_prepare, prepare_label, read_label, write_label,
};

/// The natural key a caller can rebuild without having seen the slot.
///
/// A restarted worker finds the operation it already prepared through this,
/// which is why the slot identity cannot be the only address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlotKey {
    workspace_id: String,
    org_id: String,
    execution_id: String,
    node_key: String,
    occurrence: String,
}

impl SlotKey {
    fn of(binding: &EffectSlotBinding<'_>) -> Self {
        Self {
            workspace_id: binding.scope.workspace_id.clone(),
            org_id: binding.scope.org_id.clone(),
            execution_id: binding.execution_id.to_owned(),
            node_key: binding.node_key.to_owned(),
            occurrence: binding.occurrence.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct LedgerRow {
    scope: Scope,
    execution_id: String,
    record: OperationRecord,
    adjudication: Option<AdjudicationAudit>,
}

#[derive(Clone)]
struct AdjudicationAudit {
    evidence: String,
    recorded_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for AdjudicationAudit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdjudicationAudit")
            .field("evidence_bytes", &self.evidence.len())
            .field("recorded_at", &self.recorded_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub(super) struct LedgerState {
    /// Slots addressed by the key a caller can rebuild.
    by_key: HashMap<SlotKey, EffectSlotId>,
    /// Slots addressed by the identity a caller carries afterwards.
    rows: HashMap<EffectSlotId, LedgerRow>,
}

/// In-memory reference implementation of the durable operation ledger.
#[derive(Clone, Debug)]
pub struct InMemoryOperationLedger {
    execution: super::InMemoryExecutionStore,
}

impl InMemoryOperationLedger {
    /// Share the execution owner's atomic state and clock.
    #[must_use]
    pub fn new(execution: &super::InMemoryExecutionStore) -> Self {
        Self {
            execution: execution.clone(),
        }
    }
}

/// Read a row for `slot_id` that `scope` is allowed to see.
///
/// A slot owned by another tenant is reported exactly as an absent one: saying
/// "exists, but not yours" would turn a guessed identity into a cross-tenant
/// existence oracle.
fn visible_row<'state>(
    state: &'state LedgerState,
    scope: &Scope,
    slot_id: EffectSlotId,
) -> Result<&'state LedgerRow, OperationLedgerError> {
    let row = state
        .rows
        .get(&slot_id)
        .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;
    if row.scope != *scope {
        return Err(OperationLedgerError::SlotUnprepared { slot_id });
    }
    Ok(row)
}

#[async_trait::async_trait]
impl OperationLedger for InMemoryOperationLedger {
    #[tracing::instrument(level = "debug", skip_all, name = "operation_ledger.read_occurrence", fields(backend = "in_memory", outcome = tracing::field::Empty))]
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        let result = {
            let state = self.execution.inner.lock();
            let key = SlotKey {
                workspace_id: key.scope().workspace_id.clone(),
                org_id: key.scope().org_id.clone(),
                execution_id: key.execution_id().to_owned(),
                node_key: key.node_key().to_owned(),
                occurrence: key.occurrence().to_owned(),
            };
            state
                .operation_ledger
                .by_key
                .get(&key)
                .map(|slot| {
                    state
                        .operation_ledger
                        .rows
                        .get(slot)
                        .map(|row| row.record.clone())
                        .ok_or(OperationLedgerError::CorruptRecord { slot_id: *slot })
                })
                .transpose()
        };
        let outcome = crate::operation_ledger::occurrence_read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.prepare",
        skip(self, binding),
        fields(
            backend = "in_memory",
            execution_id = binding.execution_id,
            node_key = binding.node_key,
            outcome = tracing::field::Empty,
        )
    )]
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let result = (|| {
            crate::operation_ledger::stored_attempt_generation(binding.attempt_generation)?;
            let protocol = crate::operation_ledger::initial_protocol(
                binding,
                self.execution.clock.now().timestamp_millis(),
            )?;
            let mut state = self.execution.inner.lock();
            validate_execution(
                &state,
                binding.scope,
                binding.execution_id,
                Some(fencing),
                self.execution.clock.now(),
            )?;
            let state = &mut state.operation_ledger;
            let key = SlotKey::of(binding);

            if let Some(slot_id) = state.by_key.get(&key).copied() {
                let row = state
                    .rows
                    .get(&slot_id)
                    .ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
                // Decide before writing: a mismatch must leave no durable
                // delta, and this critical section cannot roll back.
                decide_prepare(slot_id, &row.record, binding)
            } else {
                let slot_id = EffectSlotId::from_storage_bytes(*uuid::Uuid::new_v4().as_bytes());
                let operation_id = OperationId::from_bytes(*uuid::Uuid::new_v4().as_bytes());
                let record = compose_record(
                    slot_id,
                    operation_id,
                    binding.attempt_generation,
                    binding.destination,
                    binding.fingerprint,
                    OperationState::Prepared,
                )
                .with_protocol(protocol);
                let prepared = record.operation();
                state.by_key.insert(key, slot_id);
                state.rows.insert(
                    slot_id,
                    LedgerRow {
                        scope: binding.scope.clone(),
                        execution_id: binding.execution_id.to_owned(),
                        record,
                        adjudication: None,
                    },
                );
                Ok(PrepareOutcome::Prepared(prepared))
            }
        })();

        let outcome = prepare_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::inmem", outcome, "operation ledger prepare");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.read_exact",
        skip(self),
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        let result = {
            let state = self.execution.inner.lock();
            visible_row(&state.operation_ledger, scope, slot_id).map(|row| row.record.clone())
        };

        let outcome = read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::inmem", outcome, "operation ledger read");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.advance",
        skip_all,
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &nebula_storage_port::dto::OperationCommand,
    ) -> Result<nebula_storage_port::dto::OperationAdvance, OperationLedgerError> {
        let result = (|| {
            let mut state = self.execution.inner.lock();
            let row = visible_row(&state.operation_ledger, scope, slot_id)?;
            validate_execution(
                &state,
                scope,
                &row.execution_id,
                Some(fencing),
                self.execution.clock.now(),
            )?;
            let execution_id = row.execution_id.clone();
            let decision = crate::operation_ledger::decide_advance(
                &row.record,
                command,
                self.execution.clock.now().timestamp_millis(),
                OperationCallId::from_bytes(*uuid::Uuid::new_v4().as_bytes()),
            )?;
            let sequence = state.next_seq.get(&execution_id).copied().unwrap_or(1);
            if decision.journal.is_some() && sequence.checked_add(1).is_none() {
                return Err(OperationLedgerError::InvalidProtocol);
            }
            if decision.changed {
                state
                    .operation_ledger
                    .rows
                    .get_mut(&slot_id)
                    .ok_or(OperationLedgerError::CorruptRecord { slot_id })?
                    .record = decision.record;
                if let Some(journal) = decision.journal {
                    state
                        .rows
                        .get_mut(&execution_id)
                        .ok_or(OperationLedgerError::ExecutionLeaseRejected)?
                        .journal
                        .push((sequence, journal));
                    state.next_seq.insert(execution_id, sequence + 1);
                }
            }
            Ok(decision.response)
        })();

        let label = crate::operation_ledger::advance_label(&result);
        tracing::Span::current().record("outcome", label);
        tracing::debug!(
            target: "nebula_storage::inmem",
            outcome = label,
            "operation ledger fenced commit"
        );
        result
    }
}

#[async_trait::async_trait]
impl OperationLedgerAdjudicator for InMemoryOperationLedger {
    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.adjudicate",
        // `evidence` is operator prose and is deliberately not a span field:
        // it is persisted for review, not broadcast to every trace consumer.
        skip_all,
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: &nebula_storage_port::dto::FrozenOutcomeEvidence,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        let result = (|| {
            let mut state = self.execution.inner.lock();
            let row = visible_row(&state.operation_ledger, scope, slot_id)?;
            validate_execution(
                &state,
                scope,
                &row.execution_id,
                None,
                self.execution.clock.now(),
            )?;
            let execution_id = row.execution_id.clone();
            let decision = crate::operation_ledger::decide_adjudicate_protocol(
                &row.record,
                outcome,
                evidence,
            )?;
            let sequence = state.next_seq.get(&execution_id).copied().unwrap_or(1);
            if decision.journal.is_some() && sequence.checked_add(1).is_none() {
                return Err(OperationLedgerError::InvalidProtocol);
            }
            if decision.changed {
                let row = state
                    .operation_ledger
                    .rows
                    .get_mut(&slot_id)
                    .ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
                row.record = decision.record;
                row.adjudication = Some(AdjudicationAudit {
                    evidence: evidence.to_owned(),
                    recorded_at: self.execution.clock.now(),
                });
                if let Some(journal) = decision.journal {
                    state
                        .rows
                        .get_mut(&execution_id)
                        .ok_or(OperationLedgerError::ExecutionLeaseRejected)?
                        .journal
                        .push((sequence, journal));
                    state.next_seq.insert(execution_id, sequence + 1);
                }
            }
            Ok(())
        })();
        tracing::Span::current().record("outcome", write_label(&result));
        result
    }
}

fn validate_execution(
    state: &super::execution::State,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), OperationLedgerError> {
    let row = state
        .rows
        .get(execution_id)
        .filter(|row| row.scope == *scope)
        .ok_or(OperationLedgerError::ExecutionLeaseRejected)?;
    if let Some(fencing) = fencing {
        crate::operation_ledger::require_live_lease(
            fencing,
            row.fencing_generation,
            row.lease_holder.is_some()
                && row.lease_expires_at.is_some_and(|deadline| deadline > now),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::dto::{
        FrozenOutcomeEvidence, OperationCommand, OutcomeEvidenceSource, PreparedEffectContract,
        PreparedEffectPolicy,
    };
    use nebula_storage_port::store::ExecutionStore;
    use nebula_storage_port::{
        AttemptGeneration, DestinationCapability, KnownOutcome, RequestFingerprint,
    };

    #[tokio::test]
    async fn adjudication_retains_original_audit_without_debug_payload() {
        let execution = super::super::InMemoryExecutionStore::new();
        let scope = Scope::new("workspace", "org");
        execution
            .create(&scope, "execution", "workflow", serde_json::json!({}))
            .await
            .unwrap();
        let fencing = execution
            .acquire_lease(
                &scope,
                "execution",
                "runner",
                std::time::Duration::from_secs(30),
            )
            .await
            .unwrap()
            .unwrap();
        let ledger = InMemoryOperationLedger::new(&execution);
        let contract = PreparedEffectContract::new(
            RequestFingerprint::new(1, [9; 32]),
            PreparedEffectPolicy::builder(DestinationCapability::Opaque)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(std::time::Duration::from_mins(1))
                .build()
                .unwrap(),
        )
        .unwrap();
        let binding = EffectSlotBinding {
            scope: &scope,
            execution_id: "execution",
            node_key: "node",
            occurrence: "once",
            attempt_generation: AttemptGeneration::new(1),
            fingerprint: RequestFingerprint::new(1, [1; 32]),
            destination: DestinationCapability::Opaque,
            contract: &contract,
        };
        let slot = ledger
            .prepare(&binding, fencing)
            .await
            .unwrap()
            .operation()
            .slot_id();
        ledger
            .advance(
                &scope,
                slot,
                fencing,
                &OperationCommand::MarkUnknown {
                    expected_revision: 0,
                },
            )
            .await
            .unwrap();
        let before = execution.clock.now();
        let success = FrozenOutcomeEvidence::v1_json(
            OutcomeEvidenceSource::Adjudication(OperationCallId::from_bytes([3; 16])),
            KnownOutcome::Succeeded,
            b"{}".to_vec(),
        )
        .unwrap();
        let failure = FrozenOutcomeEvidence::v1_json(
            OutcomeEvidenceSource::Adjudication(OperationCallId::from_bytes([4; 16])),
            KnownOutcome::Failed,
            b"{}".to_vec(),
        )
        .unwrap();
        ledger
            .adjudicate(&scope, slot, &success, "audit-canary")
            .await
            .unwrap();
        assert!(
            ledger
                .adjudicate(&scope, slot, &failure, "replacement")
                .await
                .is_err()
        );
        assert!(!format!("{ledger:?}").contains("audit-canary"));
        let state = execution.inner.lock();
        let audit = state.operation_ledger.rows[&slot]
            .adjudication
            .as_ref()
            .unwrap();
        assert_eq!(audit.evidence, "audit-canary");
        assert!(audit.recorded_at >= before);
    }
}
