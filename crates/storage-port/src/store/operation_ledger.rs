//! Object-safe operation-ledger roles for the remote-effect protocol.
//!
//! The roles are split by what they authorize, not by convenience. An effect
//! caller prepares and commits; it cannot overwrite a terminal outcome or
//! resolve an unknown one. Only the privileged adjudicator can, and every
//! adjudication is audited — because deciding an ambiguous effect's outcome by
//! hand is exactly the operation that must never happen silently.

use core::fmt;

use crate::dto::{
    EffectOccurrenceKey, EffectSlotBinding, EffectSlotId, OperationLedgerError, OperationRecord,
    PrepareOutcome,
};
use crate::scope::Scope;

/// Durable preparation and outcome recording for remote effects.
///
/// Runtime drives bounded invocation and authenticated read-only reconciliation.
/// An unknown operation can be resolved through a recorded read-only query,
/// but this capability never authorizes privileged manual adjudication.
#[async_trait::async_trait]
pub trait OperationLedger: Send + Sync + fmt::Debug {
    /// Read by the scoped natural address after an uncertain preparation.
    /// Absence and a foreign tenant both return `None`; this read grants no invocation authority.
    ///
    /// # Errors
    /// Returns a bounded storage failure when the durable record cannot be read.
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError>;
    /// Durably prepare one effect slot before the provider is invoked.
    ///
    /// The ledger mints the slot and operation identities; the caller supplies
    /// only the binding. Preparing the same slot again with the same
    /// fingerprint and complete contract returns the original binding — including the original
    /// operation identity — so every retry and recovery reaches the provider
    /// under one identity.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::OperationMismatch`] when the slot is
    /// already bound to a different canonical request, with no durable change.
    /// [`OperationLedgerError::AcknowledgementUnknown`] means the commit may
    /// have landed and **authorizes zero provider calls** until
    /// [`Self::read_occurrence`] confirms the exact durable binding when no
    /// slot identity was acknowledged. A matching live execution lease is
    /// mandatory even when an existing preparation is replayed.
    /// Counters above `i64::MAX` return [`OperationLedgerError::InvalidAttemptGeneration`].
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: crate::FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError>;

    /// Read one slot's durable record without mutating anything.
    ///
    /// This is the database-only reconciliation an ambiguous prepare
    /// acknowledgement permits, and the only thing it permits.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::SlotUnprepared`] when the slot has no
    /// durable preparation or belongs to another tenant. These cases are
    /// deliberately indistinguishable.
    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError>;

    /// Atomically advance the finite effect protocol under the execution fence.
    ///
    /// The execution's current live lease must match `fencing` in the same
    /// transaction, including on an identical terminal recommit.
    /// Fresh grants consume durable limits before returning. An uncertain grant
    /// acknowledgement grants no egress; reloading an outstanding call cannot
    /// manufacture invocation authority. An unexplained outstanding invocation
    /// permits only read-only reconciliation or durable `OutcomeUnknown`.
    /// Outcome evidence, terminal state and the owner journal commit together.
    /// Exact evidence recommit is idempotent, including its original bytes.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::ExecutionLeaseRejected`] when the lease
    /// is not current and live, [`OperationLedgerError::OutcomeAlreadyRecorded`] when a
    /// *different* terminal outcome exists, and
    /// [`OperationLedgerError::SlotUnprepared`] when nothing was prepared.
    /// Stale revisions or call identities return [`OperationLedgerError::ProtocolConflict`].
    /// Exhausted effect grants record `OutcomeUnknown` without a permit;
    /// exhausted read-only queries return [`OperationLedgerError::RecoveryExhausted`].
    /// Commit uncertainty returns [`OperationLedgerError::AcknowledgementUnknown`].
    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: crate::FencingToken,
        command: &crate::dto::OperationCommand,
    ) -> Result<crate::dto::OperationAdvance, OperationLedgerError>;
}

/// Privileged, audited resolution of an operation whose outcome is unknown.
///
/// Deliberately a separate role: runtime control must not be able to declare an
/// ambiguous effect successful just because it holds the ledger. An
/// adjudication is a human or operator decision about something the system
/// could not determine, so it is capability-gated and leaves an audit record.
#[async_trait::async_trait]
pub trait OperationLedgerAdjudicator: Send + Sync + fmt::Debug {
    /// Resolve an `OutcomeUnknown` operation to a known outcome.
    ///
    /// `evidence` is an operator-supplied, secret-free note recording *why*
    /// the outcome is now known — a reconciliation query result, a provider
    /// support ticket. It is persisted with the adjudication so the decision
    /// is reviewable rather than anonymous. The adapter serializes this write
    /// under the owning execution lock and then the ledger lock. This separate
    /// operator capability does not require an active runner lease.
    /// `outcome` must identify an adjudication source and contain bounded frozen
    /// result evidence. Repeating the same evidence and audit note is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::SlotUnprepared`] when the slot has no
    /// record, and [`OperationLedgerError::OutcomeAlreadyRecorded`] when the
    /// operation is not in `OutcomeUnknown` — adjudication resolves
    /// uncertainty, it does not overrule a determined answer.
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: &crate::dto::FrozenOutcomeEvidence,
        evidence: &str,
    ) -> Result<(), OperationLedgerError>;
}
