use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use nebula_core::OperationId;
use nebula_storage_port::{
    FencingToken, Scope,
    dto::{
        AttemptGeneration, DestinationCapability, EffectOccurrenceKey, EffectOccurrenceRecord,
        EffectSlotBinding, EffectSlotId, OperationAdvance, OperationCommand, OperationLedgerError,
        OperationRecord, PrepareOutcome, PreparedOperation,
    },
    store::{CheckpointStore, OperationLedger},
    {CheckpointSaved, IterationCheckpoint, IterationCheckpointError, IterationCheckpointKey},
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
    /// The command commits and its answer never comes back.
    AnswerLost,
    /// The command never reaches the store and never answers.
    Hang,
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
    /// Commands at the boundary that pass before the fault fires.
    skip: std::sync::atomic::AtomicU32,
    pub natural_reads: std::sync::atomic::AtomicUsize,
    pub outcome_attempts: parking_lot::Mutex<Vec<nebula_storage_port::dto::FrozenOutcomeEvidence>>,
    pub outcome_gate: Option<Arc<OutcomeGate>>,
    /// Fired when a command whose answer is lost committed.
    pub answer_lost: tokio::sync::Notify,
    /// Fired whenever an outcome was recorded.
    pub recorded: tokio::sync::Notify,
}

impl FaultLedger {
    pub(super) fn new(inner: Arc<dyn OperationLedger>, boundary: Boundary, fault: Fault) -> Self {
        Self {
            inner,
            boundary,
            fault,
            fired: AtomicBool::new(false),
            skip: std::sync::atomic::AtomicU32::new(0),
            natural_reads: std::sync::atomic::AtomicUsize::new(0),
            outcome_attempts: parking_lot::Mutex::new(Vec::new()),
            outcome_gate: None,
            answer_lost: tokio::sync::Notify::new(),
            recorded: tokio::sync::Notify::new(),
        }
    }
    /// The same fault, fired at the boundary's command after the first
    /// `skip` pass.
    pub(super) fn skipping(self, skip: u32) -> Self {
        self.skip.store(skip, Ordering::SeqCst);
        self
    }
    fn fires(&self, boundary: Boundary) -> bool {
        if self.boundary != boundary {
            return false;
        }
        if self
            .skip
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return false;
        }
        !self.fired.swap(true, Ordering::SeqCst)
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
    async fn read_occurrences(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_key: &str,
    ) -> Result<Vec<EffectOccurrenceRecord>, OperationLedgerError> {
        self.inner
            .read_occurrences(scope, execution_id, node_key)
            .await
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
        if fire && matches!(self.fault, Fault::AnswerLost) {
            self.answer_lost.notify_one();
            return std::future::pending().await;
        }
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
        if fire && matches!(self.fault, Fault::Hang) {
            return std::future::pending().await;
        }
        let outcome = self.inner.advance(scope, slot, fencing, command).await?;
        if matches!(command, OperationCommand::RecordOutcome(_)) {
            self.recorded.notify_one();
        }
        if fire {
            return Err(OperationLedgerError::AcknowledgementUnknown);
        }
        Ok(outcome)
    }
}

/// How a [`FaultCheckpoints`] store misbehaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckpointFault {
    /// Every row is gone: loads find nothing, saves are acknowledged and
    /// dropped. The node replays from iteration 0, exactly as before
    /// checkpoints existed.
    Lost,
    /// Saves fail before reaching the store: nothing is written.
    SavesUnavailable,
    /// Saves commit and their acknowledgement is lost.
    SaveAckLost,
    /// Loads do not answer.
    LoadsUnavailable,
    /// Saves never reach the store and never answer (a crash point after a
    /// passed barrier, before its checkpoint).
    SavesHang,
}

/// A checkpoint store over a real one that fails as scripted.
#[derive(Debug)]
pub(super) struct FaultCheckpoints {
    pub inner: Arc<dyn CheckpointStore>,
    pub fault: CheckpointFault,
    /// Saves that reached this store.
    pub saves: std::sync::atomic::AtomicUsize,
    /// Fired whenever a save reached this store.
    pub save_entered: tokio::sync::Notify,
}

impl FaultCheckpoints {
    pub(super) fn new(inner: Arc<dyn CheckpointStore>, fault: CheckpointFault) -> Self {
        Self {
            inner,
            fault,
            saves: std::sync::atomic::AtomicUsize::new(0),
            save_entered: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl CheckpointStore for FaultCheckpoints {
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        match self.fault {
            CheckpointFault::Lost => Ok(None),
            CheckpointFault::LoadsUnavailable => Err(IterationCheckpointError::Unavailable),
            _ => self.inner.load_iteration_checkpoint(key).await,
        }
    }

    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.save_entered.notify_one();
        match self.fault {
            CheckpointFault::Lost => Ok(CheckpointSaved::Recorded),
            CheckpointFault::SavesHang => std::future::pending().await,
            CheckpointFault::SavesUnavailable => Err(IterationCheckpointError::Unavailable),
            CheckpointFault::SaveAckLost => {
                self.inner
                    .save_iteration_checkpoint(key, checkpoint, fencing)
                    .await?;
                Err(IterationCheckpointError::AcknowledgementUnknown)
            },
            CheckpointFault::LoadsUnavailable => {
                self.inner
                    .save_iteration_checkpoint(key, checkpoint, fencing)
                    .await
            },
        }
    }
}
