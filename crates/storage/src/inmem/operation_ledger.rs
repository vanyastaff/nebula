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
use std::sync::Arc;

use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, EffectSlotBinding, EffectSlotId, KnownOutcome, OperationId,
    OperationLedgerError, OperationRecord, OperationState, PrepareOutcome, Scope,
};
use parking_lot::Mutex;

use crate::operation_ledger::{
    CommitDecision, compose_record, decide_adjudication, decide_commit, decide_prepare,
    prepare_label, read_label, write_label,
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
    record: OperationRecord,
}

#[derive(Debug, Default)]
struct LedgerState {
    /// Slots addressed by the key a caller can rebuild.
    by_key: HashMap<SlotKey, EffectSlotId>,
    /// Slots addressed by the identity a caller carries afterwards.
    rows: HashMap<EffectSlotId, LedgerRow>,
}

/// In-memory reference implementation of the durable operation ledger.
#[derive(Clone, Debug, Default)]
pub struct InMemoryOperationLedger {
    inner: Arc<Mutex<LedgerState>>,
}

impl InMemoryOperationLedger {
    /// Build an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let result = {
            let mut state = self.inner.lock();
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
                let operation_id =
                    OperationId::from_storage_bytes(*uuid::Uuid::new_v4().as_bytes());
                let record = compose_record(
                    slot_id,
                    operation_id,
                    binding.attempt_generation,
                    binding.destination,
                    binding.fingerprint,
                    OperationState::Prepared,
                );
                state.by_key.insert(key, slot_id);
                state.rows.insert(
                    slot_id,
                    LedgerRow {
                        scope: binding.scope.clone(),
                        record,
                    },
                );
                Ok(PrepareOutcome::Prepared(record.operation()))
            }
        };

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
            let state = self.inner.lock();
            visible_row(&state, scope, slot_id).map(|row| row.record)
        };

        let outcome = read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::inmem", outcome, "operation ledger read");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.commit_outcome",
        skip(self),
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn commit_outcome(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        attempt_generation: AttemptGeneration,
        outcome: KnownOutcome,
    ) -> Result<(), OperationLedgerError> {
        let result = {
            let mut state = self.inner.lock();
            let stored = visible_row(&state, scope, slot_id)?.record;
            match decide_commit(slot_id, &stored, attempt_generation, outcome)? {
                CommitDecision::AlreadyRecorded => Ok(()),
                CommitDecision::Apply(target) => {
                    let row = state
                        .rows
                        .get_mut(&slot_id)
                        .ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
                    row.record = OperationRecord::new(
                        row.record.operation(),
                        row.record.fingerprint(),
                        target,
                    );
                    Ok(())
                },
            }
        };

        let label = write_label(&result);
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
        skip(self, evidence),
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: KnownOutcome,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        let result = {
            let mut state = self.inner.lock();
            let stored = visible_row(&state, scope, slot_id)?.record;
            let target = decide_adjudication(slot_id, &stored, outcome)?;
            if evidence.trim().is_empty() {
                // An adjudication without a reason is not reviewable, which is
                // the only thing that makes a hand-decided outcome acceptable.
                return Err(OperationLedgerError::CorruptRecord { slot_id });
            }
            let row = state
                .rows
                .get_mut(&slot_id)
                .ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
            row.record =
                OperationRecord::new(row.record.operation(), row.record.fingerprint(), target);
            Ok(())
        };

        let label = write_label(&result);
        tracing::Span::current().record("outcome", label);
        tracing::debug!(
            target: "nebula_storage::inmem",
            outcome = label,
            "operation ledger adjudication"
        );
        result
    }
}
