use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use nebula_core::OperationId;
use nebula_storage_port::{
    FencingToken, Scope,
    dto::{
        AttemptGeneration, DestinationCapability, EffectOccurrenceKey, EffectSlotBinding,
        EffectSlotId, OperationAdvance, OperationCommand, OperationLedgerError, OperationRecord,
        PrepareOutcome, PreparedOperation,
    },
    store::OperationLedger,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Boundary {
    Prepare,
    Grant,
    Outcome,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Fault {
    Before,
    After,
    AfterReadUnavailable,
    PreparedWithoutPersistence,
}

#[derive(Debug, Default)]
pub(super) struct OutcomeGate {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
    pub fencing: parking_lot::Mutex<Option<FencingToken>>,
}

#[derive(Debug)]
pub(super) struct FaultLedger {
    pub inner: Arc<dyn OperationLedger>,
    pub boundary: Boundary,
    pub fault: Fault,
    fired: AtomicBool,
    pub natural_reads: std::sync::atomic::AtomicUsize,
    pub outcome_attempts: parking_lot::Mutex<Vec<nebula_storage_port::dto::FrozenOutcomeEvidence>>,
    pub outcome_gate: Option<Arc<OutcomeGate>>,
}

impl FaultLedger {
    pub(super) fn new(inner: Arc<dyn OperationLedger>, boundary: Boundary, fault: Fault) -> Self {
        Self {
            inner,
            boundary,
            fault,
            fired: AtomicBool::new(false),
            natural_reads: std::sync::atomic::AtomicUsize::new(0),
            outcome_attempts: parking_lot::Mutex::new(Vec::new()),
            outcome_gate: None,
        }
    }
    fn fires(&self, boundary: Boundary) -> bool {
        self.boundary == boundary && !self.fired.swap(true, Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl OperationLedger for FaultLedger {
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        self.natural_reads.fetch_add(1, Ordering::SeqCst);
        if matches!(self.fault, Fault::AfterReadUnavailable) && self.fired.load(Ordering::SeqCst) {
            return Err(OperationLedgerError::Unavailable);
        }
        self.inner.read_occurrence(key).await
    }
    async fn read_exact(
        &self,
        scope: &Scope,
        slot: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        if matches!(self.fault, Fault::AfterReadUnavailable) && self.fired.load(Ordering::SeqCst) {
            return Err(OperationLedgerError::Unavailable);
        }
        self.inner.read_exact(scope, slot).await
    }
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let fire = self.fires(Boundary::Prepare);
        if fire && matches!(self.fault, Fault::PreparedWithoutPersistence) {
            return Ok(PrepareOutcome::Prepared(PreparedOperation::new(
                EffectSlotId::from_storage_bytes([0xa1; 16]),
                OperationId::from_bytes([0xb2; 16]),
                AttemptGeneration::new(1),
                DestinationCapability::Opaque,
            )));
        }
        if fire && matches!(self.fault, Fault::Before) {
            return Err(OperationLedgerError::AcknowledgementUnknown);
        }
        let outcome = self.inner.prepare(binding, fencing).await?;
        if fire {
            return Err(OperationLedgerError::AcknowledgementUnknown);
        }
        Ok(outcome)
    }
    async fn advance(
        &self,
        scope: &Scope,
        slot: EffectSlotId,
        fencing: FencingToken,
        command: &OperationCommand,
    ) -> Result<OperationAdvance, OperationLedgerError> {
        let fire = match command {
            OperationCommand::GrantInvocation { .. } => self.fires(Boundary::Grant),
            OperationCommand::RecordOutcome(evidence) => {
                self.outcome_attempts.lock().push(evidence.clone());
                self.fires(Boundary::Outcome)
            },
            _ => false,
        };
        if matches!(command, OperationCommand::RecordOutcome(_))
            && let Some(gate) = &self.outcome_gate
        {
            *gate.fencing.lock() = Some(fencing);
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        if fire && matches!(self.fault, Fault::Before) {
            return Err(OperationLedgerError::AcknowledgementUnknown);
        }
        let outcome = self.inner.advance(scope, slot, fencing, command).await?;
        if fire {
            return Err(OperationLedgerError::AcknowledgementUnknown);
        }
        Ok(outcome)
    }
}
