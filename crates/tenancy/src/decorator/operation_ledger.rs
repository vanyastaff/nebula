//! Scope-enforcing [`OperationLedger`] decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    EffectOccurrenceKey, EffectSlotBinding, EffectSlotId, FrozenOutcomeEvidence, OperationAdvance,
    OperationCommand, OperationLedgerError, OperationRecord, PrepareOutcome,
};
use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{FencingToken, Scope};

/// Forces every operation-ledger read and mutation into one bound tenant.
#[derive(Clone)]
pub struct ScopedOperationLedger {
    inner: Arc<dyn OperationLedger>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedOperationLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedOperationLedger")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedOperationLedger {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn OperationLedger>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl OperationLedger for ScopedOperationLedger {
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        let scoped_key = EffectOccurrenceKey::new(
            &self.bound,
            key.execution_id(),
            key.node_key(),
            key.occurrence(),
        );
        self.inner.read_occurrence(&scoped_key).await
    }

    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let scoped_binding = EffectSlotBinding {
            scope: &self.bound,
            execution_id: binding.execution_id,
            node_key: binding.node_key,
            occurrence: binding.occurrence,
            attempt_generation: binding.attempt_generation,
            fingerprint: binding.fingerprint,
            destination: binding.destination,
            contract: binding.contract,
        };
        self.inner.prepare(&scoped_binding, fencing).await
    }

    async fn read_exact(
        &self,
        _scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        self.inner.read_exact(&self.bound, slot_id).await
    }

    async fn advance(
        &self,
        _scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &OperationCommand,
    ) -> Result<OperationAdvance, OperationLedgerError> {
        self.inner
            .advance(&self.bound, slot_id, fencing, command)
            .await
    }
}

/// Forces privileged operation adjudication into one bound tenant.
#[derive(Clone)]
pub struct ScopedOperationLedgerAdjudicator {
    inner: Arc<dyn OperationLedgerAdjudicator>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedOperationLedgerAdjudicator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedOperationLedgerAdjudicator")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedOperationLedgerAdjudicator {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn OperationLedgerAdjudicator>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl OperationLedgerAdjudicator for ScopedOperationLedgerAdjudicator {
    async fn adjudicate(
        &self,
        _scope: &Scope,
        slot_id: EffectSlotId,
        outcome: &FrozenOutcomeEvidence,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        self.inner
            .adjudicate(&self.bound, slot_id, outcome, evidence)
            .await
    }
}
