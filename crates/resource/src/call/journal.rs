//! The effect journal seam: the port through which a resource handle's
//! effectful units reach the execution owner that records them.
//!
//! The resource runtime never writes durable effect state. A row an action
//! gets with effect-owner authority
//! ([`Manager::handle_any_journaled`](crate::Manager::handle_any_journaled))
//! carries a [`EffectJournal`]; every `Idempotent` or `Write` unit
//! submitted with [`ResourceHandle::submit`](super::ResourceHandle::submit)
//! or [`ResourceHandle::session`](super::ResourceHandle::session) is driven
//! through it (a `Read` never is):
//!
//! 1. **Submit** — the unit takes an in-flight [`InFlight`]
//!    ([`track`](EffectJournal::track)); a
//!    [closed](EffectJournal::is_closed) owner refuses it. A submission is
//!    lazy: one dropped before its first poll never reaches the owner.
//! 2. **Prepare** — the first poll takes the unit's positional ordinal per
//!    resource and unit kind ([`next_ordinal`](EffectJournal::next_ordinal))
//!    and hands the owner a [`JournalIntent`]; the returned
//!    [`JournalSlot`]'s [`SlotPhase`] replays a recorded outcome, refuses an
//!    unknown one, or lets the unit run.
//! 3. **Grant** — each attempt, after its checkout and credential reads and
//!    before its registration, asks for a [`CallGrant`]: the only authority
//!    for one provider call, within the grant's budget (the unit's deadline
//!    shrinks to it).
//! 4. **Explain / settle** — every granted call is either explained
//!    ([`Crossing`]) or settled ([`CallOutcome`]) exactly once.
//!
//! The engine implements the owner over its operation ledger; this crate
//! only drives the seam.

use std::{fmt, num::NonZeroU32, sync::Mutex, time::Duration};

use nebula_core::ResourceKey;

use super::{cost::Effect, declaration::IdempotencyKey};
use crate::{dedup::SlotIdentity, error::ErrorKind};

/// The owner of a row's execution-owned effects: prepares, grants and
/// records them. Implemented by the engine; object safe
/// (`Arc<dyn EffectJournal>`).
///
/// Every method but [`next_ordinal`](Self::next_ordinal),
/// [`track`](Self::track) and [`is_closed`](Self::is_closed) is a durable
/// step. A refusal ([`JournalRefusal`]) never means a provider call happened.
#[async_trait::async_trait]
pub trait EffectJournal: Send + Sync + fmt::Debug {
    /// The next ordinal of a unit of `kind` on `key`, in the order units
    /// start preparing: positional, whatever the unit's operation or session
    /// name and version. Synchronous: it is taken by the unit's first poll,
    /// right before [`prepare`](Self::prepare) — never at submit, so a
    /// submission dropped unpolled takes no position.
    ///
    /// The name and version stay out of the occurrence on purpose: they are
    /// part of the effect's contract, so an operation whose `KEY` or
    /// `VERSION` changed under a recorded occurrence is a mismatch the owner
    /// refuses with nothing sent, never a fresh occurrence that sends the
    /// effect again. Units polled in another order than an earlier run
    /// polled them meet each other's recorded positions the same way: a
    /// different intent is a mismatch, an identical one is interchangeable.
    fn next_ordinal(&self, key: &ResourceKey, kind: UnitKind) -> u32;

    /// Durably prepares the effect `intent` describes (recovering an
    /// unacknowledged earlier prepare) and returns its slot.
    ///
    /// # Errors
    ///
    /// Why the owner could not prepare it.
    async fn prepare(&self, intent: &JournalIntent<'_>) -> Result<JournalSlot, JournalRefusal>;

    /// Grants one provider call on `slot`. `Ok` is the only authority for a
    /// provider call, and only within the grant's
    /// [`budget`](CallGrant::budget), when it has one.
    ///
    /// # Errors
    ///
    /// [`JournalRefusal::Unknown`] when the slot's outcome became unknown (an
    /// opaque effect that may have been sent, an exhausted budget, a
    /// stable-key window that passed); otherwise why the owner refused.
    async fn grant(&self, slot: &JournalSlot) -> Result<CallGrant, JournalRefusal>;

    /// Records how a granted `call` crossed the provider boundary, when it
    /// did not settle.
    ///
    /// # Errors
    ///
    /// Why the owner could not record it.
    async fn explain(
        &self,
        slot: &JournalSlot,
        call: CallGrant,
        crossing: Crossing,
    ) -> Result<(), JournalRefusal>;

    /// Records the outcome of a granted `call` (recommitting the exact
    /// evidence when an acknowledgement was lost).
    ///
    /// # Errors
    ///
    /// Why the owner could not record it; the unit then fails with an
    /// unknown outcome.
    async fn settle(
        &self,
        slot: &JournalSlot,
        call: CallGrant,
        outcome: CallOutcome<'_>,
    ) -> Result<(), JournalRefusal>;

    /// An in-flight ticket held by every submitted unit until it is gone;
    /// the owner drains them before finalizing its node.
    fn track(&self) -> InFlight;

    /// Whether the owner closed (its node finished): a closed owner refuses
    /// every new unit.
    fn is_closed(&self) -> bool;
}

/// What kind of unit an effect belongs to: its occurrence namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitKind {
    /// A submitted [`Operation`](super::Operation), named by its key.
    Operation,
    /// A session, named by its [`SessionSpec`](super::SessionSpec) name.
    Session,
}

impl UnitKind {
    /// The kind's segment of an occurrence label: `op` or `session`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operation => "op",
            Self::Session => "session",
        }
    }
}

impl fmt::Display for UnitKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How the owner recovers an effect whose outcome it does not know, derived
/// from the unit's [`Effect`]: an [`Idempotent`](Effect::Idempotent) unit
/// recovers by [`StableKey`](Self::StableKey) within its key window, a
/// [`Write`](Effect::Write) is [`Opaque`](Self::Opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Recovery {
    /// The provider deduplicates requests carrying the same idempotency key
    /// for `window`: within it, an ambiguous attempt may be sent again
    /// with the same [`IdempotencyKey`].
    StableKey {
        /// How long the provider remembers a key. Non-zero.
        window: Duration,
    },
    /// Nothing tells a repeat apart: an ambiguous attempt is never sent
    /// again, its outcome is unknown until reconciled.
    Opaque,
}

impl Recovery {
    /// Stable lowercase name: `stable_key` or `opaque`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StableKey { .. } => "stable_key",
            Self::Opaque => "opaque",
        }
    }
}

/// What an owner prepares: one effect of one unit.
///
/// Derived by the runtime from the operation (or the session spec): its
/// contract is `(operation, version)`, its canonical request the
/// key-sorted JSON of the operation value, its recovery the effect's.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct JournalIntent<'a> {
    /// The row the effect runs against.
    pub resource_key: &'a ResourceKey,
    /// The row's credential slot identity.
    pub binding: &'a SlotIdentity,
    /// The row's configuration as the unit found it when it was prepared:
    /// its [`ResourceConfig::fingerprint`](crate::ResourceConfig::fingerprint)
    /// (a configuration holds no secrets; credentials are bound separately,
    /// by `binding`). An owner binds it into the effect's destination, so an
    /// effect recorded against one configuration (one endpoint) is never
    /// granted again after a reload points the row elsewhere.
    pub config_fingerprint: u64,
    /// Whether an operation or a session.
    pub kind: UnitKind,
    /// The operation key or the session name: unique within the resource.
    pub operation: &'a str,
    /// The operation's interface version: the journal contract, with
    /// `operation`. At least 1.
    pub version: u32,
    /// The unit's declared effect: `Idempotent` or `Write`.
    pub effect: Effect,
    /// How an unknown outcome is recovered.
    pub recovery: Recovery,
    /// Whether a success is recorded with its output (replayed on resume)
    /// or digest-only (a resume fails `Permanent`).
    pub record_output: bool,
    /// Attempts the unit may be granted
    /// ([`Operation::max_attempts`](super::Operation::max_attempts)).
    pub max_invocations: NonZeroU32,
    /// The occurrence label, `unit/v1/{resource_key}/{kind}/#{ordinal:06}`:
    /// visible ASCII, at most 512 bytes. Positional — `operation` and
    /// `version` are not part of it, so a changed operation under a recorded
    /// occurrence is a mismatch.
    pub occurrence: &'a str,
    /// The canonical request (canonicalization version 1: key-sorted
    /// compact JSON), 1 byte to 1 MiB; digest it, never store it.
    pub canonical_request: &'a [u8],
    /// The developer part of the provider idempotency key: 1 to 256 bytes
    /// of visible ASCII.
    pub key_part: Option<&'a str>,
}

impl fmt::Debug for JournalIntent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalIntent")
            .field("resource_key", self.resource_key)
            .field("config_fingerprint", &self.config_fingerprint)
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field("version", &self.version)
            .field("effect", &self.effect)
            .field("recovery", &self.recovery)
            .field("record_output", &self.record_output)
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
    /// The effect applied and only a digest was kept (the unit declared no
    /// recorded output, or its output was over the recording cap).
    OutputUnavailable,
    /// The provider definitively rejected the effect.
    Failed(ErrorKindCode),
}

/// A prepared effect, as its owner identifies it.
///
/// Opaque to the runtime: built by the owner with [`new`](Self::new) and
/// handed back to it on every later step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalSlot {
    id: [u8; 16],
    idempotency_key: IdempotencyKey,
    revision: u64,
    phase: SlotPhase,
}

impl JournalSlot {
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

/// One granted provider call, as its owner identifies it, and how long the
/// owner's guarantees cover it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallGrant {
    id: [u8; 16],
    budget: Option<Duration>,
}

impl CallGrant {
    /// The call `id`, bounded only by the unit's deadline.
    #[must_use]
    pub const fn from_bytes(id: [u8; 16]) -> Self {
        Self { id, budget: None }
    }

    /// The same call, which must start and finish within `budget` of the
    /// moment the owner hands it out: past it the owner can no longer
    /// vouch for the call (a stable key's deduplication window ends). The
    /// runtime bounds the unit by the earlier of its deadline and the
    /// budget, and refuses a call whose budget is zero.
    #[must_use]
    pub const fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = Some(budget);
        self
    }

    /// The call id.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.id
    }

    /// How long the call may run from the moment it was handed out; `None`
    /// when only the unit's deadline bounds it.
    #[must_use]
    pub const fn budget(&self) -> Option<Duration> {
        self.budget
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
pub enum CallOutcome<'a> {
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
pub enum JournalRefusal {
    /// The owner's store is unavailable.
    Unavailable,
    /// The owner could not learn whether its write was acknowledged.
    AcknowledgementUnknown,
    /// The effect differs from the one recorded under its occurrence (its
    /// operation, version, request or key part changed).
    Mismatch,
    /// The owner closed: its node finished.
    Closed,
    /// The owner lost the lease that authorizes its writes.
    LeaseLost,
    /// The effect's outcome is unknown: no call is granted.
    Unknown,
}

impl JournalRefusal {
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

impl fmt::Display for JournalRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An in-flight unit, as its owner counts them: dropping it releases it.
pub struct InFlight {
    release: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl InFlight {
    /// A ticket that runs `release` once, when it is dropped.
    #[must_use]
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Mutex::new(Some(Box::new(release))),
        }
    }
}

impl Drop for InFlight {
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

impl fmt::Debug for InFlight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("InFlight").finish_non_exhaustive()
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
