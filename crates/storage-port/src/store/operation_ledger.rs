//! Object-safe operation-ledger roles for the remote-effect protocol.
//!
//! The roles are split by what they authorize, not by convenience. An effect
//! caller prepares and commits; it cannot overwrite a terminal outcome or
//! resolve an unknown one. Only the privileged adjudicator can, and every
//! adjudication is audited — because deciding an ambiguous effect's outcome by
//! hand is exactly the operation that must never happen silently.

use core::fmt;

use crate::dto::{
    AttemptGeneration, EffectSlotBinding, EffectSlotId, KnownOutcome, OperationLedgerError,
    OperationRecord, PrepareOutcome,
};
use crate::scope::Scope;

/// Durable preparation and outcome recording for remote effects.
///
/// This is the capability runtime control holds while driving an effect. It
/// cannot resolve an operation that has reached `OutcomeUnknown`.
#[async_trait::async_trait]
pub trait OperationLedger: Send + Sync + fmt::Debug {
    /// Durably prepare one effect slot before the provider is invoked.
    ///
    /// The ledger mints the slot and operation identities; the caller supplies
    /// only the binding. Preparing the same slot again with the same
    /// fingerprint returns the original binding — including the original
    /// operation identity — so every retry and recovery reaches the provider
    /// under one identity.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::OperationMismatch`] when the slot is
    /// already bound to a different canonical request, with no durable change.
    /// [`OperationLedgerError::AcknowledgementUnknown`] means the commit may
    /// have landed and **authorizes zero provider calls** until
    /// [`Self::read_exact`] confirms the exact durable binding.
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
    ) -> Result<PrepareOutcome, OperationLedgerError>;

    /// Read one slot's durable record without mutating anything.
    ///
    /// This is the database-only reconciliation an ambiguous prepare
    /// acknowledgement permits, and the only thing it permits.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::SlotUnprepared`] when the slot has no
    /// durable preparation, and [`OperationLedgerError::TenantDenied`] when it
    /// belongs to another tenant — the two are deliberately
    /// indistinguishable to a caller that cannot see the tenant boundary.
    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError>;

    /// Record a known outcome against a prepared operation, under its fence.
    ///
    /// `attempt_generation` must match the slot's durable binding: a
    /// superseded worker cannot decide the current attempt's outcome.
    /// Committing the same outcome again is idempotent, which is what lets a
    /// caller whose acknowledgement was lost recommit the same frozen evidence.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::StaleFence`] when the generation is
    /// behind, [`OperationLedgerError::OutcomeAlreadyRecorded`] when a
    /// *different* terminal outcome exists, and
    /// [`OperationLedgerError::SlotUnprepared`] when nothing was prepared.
    async fn commit_outcome(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        attempt_generation: AttemptGeneration,
        outcome: KnownOutcome,
    ) -> Result<(), OperationLedgerError>;
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
    /// is reviewable rather than anonymous.
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
        outcome: KnownOutcome,
        evidence: &str,
    ) -> Result<(), OperationLedgerError>;
}
