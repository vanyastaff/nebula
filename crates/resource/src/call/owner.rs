//! The execution-owner seam: the port through which a managed row's
//! effectful units reach the owner that records them.
//!
//! The resource runtime never writes durable effect state. A row an action
//! gets with effect-owner authority
//! ([`Manager::handle_any_journaled`](crate::Manager::handle_any_journaled))
//! carries a [`UnitEffectOwner`]; every unit submitted with
//! [`ResourceHandle::submit_effect`](super::ResourceHandle::submit_effect) or
//! [`ResourceHandle::session_effect`](super::ResourceHandle::session_effect) is
//! driven through it:
//!
//! 1. **Submit** — the unit takes an in-flight [`OwnerTicket`]
//!    ([`track`](UnitEffectOwner::track)) and, without an author label, an
//!    ordinal ([`next_ordinal`](UnitEffectOwner::next_ordinal)); a
//!    [closed](UnitEffectOwner::is_closed) owner refuses it.
//! 2. **Prepare** — the first poll hands the owner a [`UnitIntent`]; the
//!    returned [`UnitSlot`]'s [`SlotPhase`] replays a recorded outcome,
//!    refuses an unknown one, or lets the unit run.
//! 3. **Grant** — each attempt, after its checkout and credential reads and
//!    before its registration, asks for a [`UnitCall`]: the only authority
//!    for one provider call.
//! 4. **Explain / settle** — every granted call is either explained
//!    ([`Crossing`]) or settled ([`UnitOutcome`]) exactly once.
//!
//! The engine implements the owner over its operation ledger; this crate
//! only drives the seam.

use std::{fmt, num::NonZeroU32, sync::Mutex};

use nebula_core::ResourceKey;

use super::{
    cost::Effect,
    effect::{EffectContract, EffectRecovery, IdempotencyKey, IdempotencyKeyPart, Recorded},
};
use crate::{dedup::SlotIdentity, error::ErrorKind};

/// The owner of a row's execution-owned effects: prepares, grants and
/// records them. Implemented by the engine; object safe
/// (`Arc<dyn UnitEffectOwner>`).
///
/// Every method but [`next_ordinal`](Self::next_ordinal),
/// [`track`](Self::track) and [`is_closed`](Self::is_closed) is a durable
/// step. A refusal ([`OwnerRefusal`]) never means a provider call happened.
#[async_trait::async_trait]
pub trait UnitEffectOwner: Send + Sync + fmt::Debug {
    /// The next ordinal of an unlabeled unit of `contract` on `key`, in
    /// program (submit) order. Synchronous: it is taken when the unit is
    /// submitted.
    fn next_ordinal(&self, key: &ResourceKey, contract: EffectContract) -> u32;

    /// Durably prepares the effect `intent` describes (recovering an
    /// unacknowledged earlier prepare) and returns its slot.
    ///
    /// # Errors
    ///
    /// Why the owner could not prepare it.
    async fn prepare(&self, intent: &UnitIntent<'_>) -> Result<UnitSlot, OwnerRefusal>;

    /// Grants one provider call on `slot`. `Ok` is the only authority for a
    /// provider call.
    ///
    /// # Errors
    ///
    /// [`OwnerRefusal::Unknown`] when the slot's outcome became unknown (an
    /// opaque effect that may have been sent, an exhausted budget, a
    /// stable-key window that passed); otherwise why the owner refused.
    async fn grant(&self, slot: &UnitSlot) -> Result<UnitCall, OwnerRefusal>;

    /// Records how a granted `call` crossed the provider boundary, when it
    /// did not settle.
    ///
    /// # Errors
    ///
    /// Why the owner could not record it.
    async fn explain(
        &self,
        slot: &UnitSlot,
        call: UnitCall,
        crossing: Crossing,
    ) -> Result<(), OwnerRefusal>;

    /// Records the outcome of a granted `call` (recommitting the exact
    /// evidence when an acknowledgement was lost).
    ///
    /// # Errors
    ///
    /// Why the owner could not record it; the unit then fails with an
    /// unknown outcome.
    async fn settle(
        &self,
        slot: &UnitSlot,
        call: UnitCall,
        outcome: UnitOutcome<'_>,
    ) -> Result<(), OwnerRefusal>;

    /// An in-flight ticket held by every submitted unit until it is gone;
    /// the owner drains them before finalizing its node.
    fn track(&self) -> OwnerTicket;

    /// Whether the owner closed (its node finished): a closed owner refuses
    /// every new unit.
    fn is_closed(&self) -> bool;
}

/// What an owner prepares: one effect of one unit.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct UnitIntent<'a> {
    /// The row the effect runs against.
    pub resource_key: &'a ResourceKey,
    /// The row's credential slot identity.
    pub binding: &'a SlotIdentity,
    /// The operation's integration contract.
    pub contract: EffectContract,
    /// The operation's declared effect: `Idempotent` or `Write`.
    pub effect: Effect,
    /// How an unknown outcome is recovered.
    pub recovery: EffectRecovery,
    /// What the owner records of a success.
    pub recorded: Recorded,
    /// Attempts the unit may be granted
    /// ([`Operation::max_attempts`](super::Operation::max_attempts)).
    pub max_invocations: NonZeroU32,
    /// The occurrence label, `unit/v1/{resource_key}/{contract_id}/{label}`:
    /// visible ASCII, at most 512 bytes.
    pub occurrence: &'a str,
    /// The canonical request, 1 byte to 1 MiB; digest it, never store it.
    pub canonical_request: &'a [u8],
    /// The developer part of the provider idempotency key.
    pub key_part: Option<&'a IdempotencyKeyPart>,
}

impl fmt::Debug for UnitIntent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnitIntent")
            .field("resource_key", self.resource_key)
            .field("contract", &self.contract)
            .field("effect", &self.effect)
            .field("recovery", &self.recovery)
            .field("recorded", &self.recorded)
            .field("max_invocations", &self.max_invocations)
            .field("occurrence", &self.occurrence)
            .field("canonical_request_len", &self.canonical_request.len())
            .field("key_part", &self.key_part.is_some())
            .finish_non_exhaustive()
    }
}

/// Where a prepared effect stands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SlotPhase {
    /// No outcome is recorded and a call may be granted: the unit runs.
    Runnable,
    /// An outcome is recorded: the unit replays it without a provider call.
    Replay(RecordedOutcome),
    /// The outcome is unknown: the unit fails `OutcomeUnknown` without a
    /// provider call until the effect is reconciled.
    Unknown,
}

/// A recorded outcome, as a resumed unit replays it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordedOutcome {
    /// The effect applied; its output serialized as JSON.
    Succeeded(Vec<u8>),
    /// The effect applied and only a digest was kept
    /// ([`Recorded::DigestOnly`]).
    OutputUnavailable,
    /// The provider definitively rejected the effect.
    Failed(ErrorKindCode),
}

/// A prepared effect, as its owner identifies it.
///
/// Opaque to the runtime: built by the owner with [`new`](Self::new) and
/// handed back to it on every later step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSlot {
    id: [u8; 16],
    idempotency_key: IdempotencyKey,
    revision: u64,
    phase: SlotPhase,
}

impl UnitSlot {
    /// The slot `id` at `revision`, whose provider idempotency key is
    /// `idempotency_key`, in `phase`.
    #[must_use]
    pub fn new(
        id: [u8; 16],
        idempotency_key: IdempotencyKey,
        revision: u64,
        phase: SlotPhase,
    ) -> Self {
        Self {
            id,
            idempotency_key,
            revision,
            phase,
        }
    }

    /// The owner's slot id.
    #[must_use]
    pub fn id(&self) -> &[u8; 16] {
        &self.id
    }

    /// The provider idempotency key of the effect.
    #[must_use]
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// The revision the slot was prepared at.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Where the effect stands.
    #[must_use]
    pub fn phase(&self) -> &SlotPhase {
        &self.phase
    }
}

/// One granted provider call, as its owner identifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitCall([u8; 16]);

impl UnitCall {
    /// The call `id`.
    #[must_use]
    pub const fn from_bytes(id: [u8; 16]) -> Self {
        Self(id)
    }

    /// The call id.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// How a granted call that did not settle crossed the provider boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Crossing {
    /// Nothing reached the provider, or the provider applied nothing (it
    /// throttled): the call may be granted again.
    NotCrossed,
    /// The request may have been applied: a stable-key effect may be
    /// granted again within its window, an opaque one's outcome is unknown.
    Ambiguous,
}

/// How a granted call settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnitOutcome<'a> {
    /// The effect applied; its output serialized as JSON.
    Applied(&'a [u8]),
    /// The effect applied; only a digest is recorded.
    AppliedWithoutOutput,
    /// The provider definitively rejected the effect.
    Rejected(ErrorKindCode),
}

/// Why an owner refused a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OwnerRefusal {
    /// The owner's store is unavailable.
    Unavailable,
    /// The owner could not learn whether its write was acknowledged.
    AcknowledgementUnknown,
    /// The effect differs from the one recorded under its occurrence (its
    /// contract, request or key changed).
    Mismatch,
    /// The owner closed: its node finished.
    Closed,
    /// The owner lost the lease that authorizes its writes.
    LeaseLost,
    /// The effect's outcome is unknown: no call is granted.
    Unknown,
}

impl OwnerRefusal {
    /// Stable lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::AcknowledgementUnknown => "acknowledgement_unknown",
            Self::Mismatch => "mismatch",
            Self::Closed => "closed",
            Self::LeaseLost => "lease_lost",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for OwnerRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An in-flight unit, as its owner counts them: dropping it releases it.
pub struct OwnerTicket {
    release: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl OwnerTicket {
    /// A ticket that runs `release` once, when it is dropped.
    #[must_use]
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Mutex::new(Some(Box::new(release))),
        }
    }
}

impl Drop for OwnerTicket {
    fn drop(&mut self) {
        let release = self
            .release
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(release) = release {
            release();
        }
    }
}

impl fmt::Debug for OwnerTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerTicket")
            .finish_non_exhaustive()
    }
}

/// The closed, secret-free code of a provider's definitive rejection: an
/// [`ErrorKind`] without its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKindCode {
    /// [`ErrorKind::Transient`].
    Transient,
    /// [`ErrorKind::Permanent`].
    Permanent,
    /// [`ErrorKind::Exhausted`].
    Exhausted,
    /// [`ErrorKind::Backpressure`].
    Backpressure,
    /// [`ErrorKind::NotFound`].
    NotFound,
    /// [`ErrorKind::Cancelled`].
    Cancelled,
    /// [`ErrorKind::Revoked`].
    Revoked,
    /// [`ErrorKind::Ambiguous`].
    Ambiguous,
    /// [`ErrorKind::CredentialUnavailable`].
    CredentialUnavailable,
    /// [`ErrorKind::OutcomeUnknown`].
    OutcomeUnknown,
}

impl ErrorKindCode {
    /// The code of `kind`.
    #[must_use]
    pub fn of(kind: &ErrorKind) -> Self {
        match kind {
            ErrorKind::Transient => Self::Transient,
            ErrorKind::Permanent => Self::Permanent,
            ErrorKind::Exhausted { .. } => Self::Exhausted,
            ErrorKind::Backpressure => Self::Backpressure,
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::Cancelled => Self::Cancelled,
            ErrorKind::Revoked => Self::Revoked,
            ErrorKind::Ambiguous => Self::Ambiguous,
            ErrorKind::CredentialUnavailable { .. } => Self::CredentialUnavailable,
            ErrorKind::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }

    /// Stable lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Permanent => "permanent",
            Self::Exhausted => "exhausted",
            Self::Backpressure => "backpressure",
            Self::NotFound => "not_found",
            Self::Cancelled => "cancelled",
            Self::Revoked => "revoked",
            Self::Ambiguous => "ambiguous",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    /// The kind a replayed rejection fails with. Only non-retryable kinds
    /// are recorded as rejections; a retryable code is replayed as
    /// `Permanent`, because a recorded rejection is final.
    pub(crate) fn replayed_kind(self) -> ErrorKind {
        match self {
            Self::NotFound => ErrorKind::NotFound,
            Self::Cancelled => ErrorKind::Cancelled,
            Self::Ambiguous => ErrorKind::Ambiguous,
            Self::OutcomeUnknown => ErrorKind::OutcomeUnknown,
            Self::Permanent
            | Self::Transient
            | Self::Exhausted
            | Self::Backpressure
            | Self::Revoked
            | Self::CredentialUnavailable => ErrorKind::Permanent,
        }
    }
}

impl fmt::Display for ErrorKindCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
