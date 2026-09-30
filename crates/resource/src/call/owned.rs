//! The unit runtime's side of execution-owned effects: it drives an
//! `Idempotent` or `Write` unit submitted on a journaled row through the
//! row's [`EffectJournal`] (see the [`journal`](super::journal) module for
//! the seam).
//!
//! Per unit, in order:
//!
//! - **submit** ([`OwnedEffect::submit`]) — refused when the owner closed;
//!   otherwise the occurrence label is fixed from the next ordinal of the
//!   unit's kind and name, and an in-flight ticket taken;
//! - **first poll** ([`prepare`]) — before anything is spawned, checked out,
//!   booked or read: replay, refusal, or run;
//! - **each attempt** — the previous attempt's call is explained when the
//!   attempt starts ([`OwnedEffect::flush_previous`]); after the checkout
//!   and the credential reads, before the registration, the owner grants
//!   the call ([`OwnedEffect::grant`]); a registration that then refuses
//!   explains it `NotCrossed` ([`OwnedEffect::release_refused`]);
//! - **settle** ([`OwnedEffect::finish`]) — the last call is settled or
//!   explained from the unit's result.
//!
//! A unit cancelled before its first grant leaves only its prepare behind:
//! the slot stays prepared and a resumed unit runs it again. A grant still
//! in flight when the unit's deadline cut the operation off is not
//! explained here: the owner sees an outstanding call and treats it as an
//! ambiguous crossing.

use std::{
    fmt,
    num::NonZeroU32,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use nebula_core::ResourceKey;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    cost::{Effect, SentState},
    declaration::MAX_RECORDED_OUTPUT_LEN,
    error::OperationError,
    journal::{
        CallGrant, CallOutcome, Crossing, EffectJournal, ErrorKindCode, InFlight, JournalIntent,
        JournalRefusal, JournalSlot, RecordedOutcome, Recovery, SlotPhase, UnitKind,
    },
    managed::{UnitShared, cancelled_before_grant},
};
use crate::{dedup::SlotIdentity, error::ErrorKind};

/// Longest occurrence label the owner's ledger accepts, in bytes.
const MAX_OCCURRENCE_LABEL_LEN: usize = 512;

/// How a unit's output is recorded and replayed: JSON.
pub(super) struct OutputCodec<T> {
    encode: fn(&T) -> Result<Vec<u8>, serde_json::Error>,
    decode: fn(&[u8]) -> Result<T, serde_json::Error>,
}

impl<T> Clone for OutputCodec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for OutputCodec<T> {}

impl<T: Serialize + DeserializeOwned> OutputCodec<T> {
    pub(super) fn json() -> Self {
        Self {
            encode: |output| serde_json::to_vec(output),
            decode: |bytes| serde_json::from_slice(bytes),
        }
    }
}

/// The journal declaration the runtime derived for one unit at submit.
pub(super) struct JournalDeclaration {
    pub(super) kind: UnitKind,
    /// The operation key or the session name.
    pub(super) name: &'static str,
    pub(super) version: u32,
    pub(super) effect: Effect,
    pub(super) recovery: Recovery,
    pub(super) record_output: bool,
    pub(super) canonical_request: Vec<u8>,
    pub(super) key_part: Option<String>,
}

/// The owned-effect state of one unit, shared by its handle, its runtime
/// task and its attempts.
pub(super) struct OwnedEffect {
    owner: Arc<dyn EffectJournal>,
    resource_key: ResourceKey,
    binding: SlotIdentity,
    declaration: JournalDeclaration,
    occurrence: String,
    /// Set by a `Runnable` prepare.
    slot: OnceLock<JournalSlot>,
    /// The unit's latest granted call not yet explained or settled.
    pending: Mutex<Option<PendingCall>>,
    _ticket: InFlight,
}

/// A granted call and how its attempt settled so far.
#[derive(Debug, Clone, Copy)]
struct PendingCall {
    call: CallGrant,
    /// `MaybeSent` until the attempt settles.
    sent: SentState,
    /// What the attempt's classified result said about the call.
    note: CallNote,
}

/// What a finished attempt's classified result said about its call, beyond
/// its sent state ([`Attempt::finish`](super::Attempt::finish)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallNote {
    /// Nothing more: a success, an unclassified or unsettled attempt.
    Plain,
    /// The provider throttled the call and applied nothing.
    Throttled,
    /// The provider definitively rejected the call with this kind.
    Rejected(ErrorKindCode),
}

impl fmt::Debug for OwnedEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEffect")
            .field("occurrence", &self.occurrence)
            .field("recovery", &self.declaration.recovery)
            .field("prepared", &self.slot.get().is_some())
            .finish_non_exhaustive()
    }
}

impl OwnedEffect {
    /// The owned state of a unit of `declaration` on `key`: refused
    /// `Cancelled` when `owner` closed; otherwise with its occurrence label
    /// `unit/v1/{key}/{kind}/{name}/v{version}/#{ordinal:06}` and an
    /// in-flight ticket.
    pub(super) fn submit(
        owner: &Arc<dyn EffectJournal>,
        binding: &SlotIdentity,
        key: &ResourceKey,
        declaration: JournalDeclaration,
    ) -> Result<Self, OperationError> {
        if owner.is_closed() {
            return Err(OperationError::new(
                ErrorKind::Cancelled,
                "effect owner closed; unit refused",
            ));
        }
        let ordinal = owner.next_ordinal(key, declaration.kind, declaration.name);
        let occurrence = format!(
            "unit/v1/{key}/{}/{}/v{}/#{ordinal:06}",
            declaration.kind, declaration.name, declaration.version
        );
        let fits = occurrence.len() <= MAX_OCCURRENCE_LABEL_LEN
            && occurrence.bytes().all(|byte| (0x21..=0x7E).contains(&byte));
        if !fits {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "effect occurrence label must be at most 512 bytes of visible ASCII",
            ));
        }
        Ok(Self {
            owner: Arc::clone(owner),
            resource_key: key.clone(),
            binding: binding.clone(),
            declaration,
            occurrence,
            slot: OnceLock::new(),
            pending: Mutex::new(None),
            _ticket: owner.track(),
        })
    }

    /// The occurrence label the owner records the effect under.
    pub(super) fn occurrence(&self) -> &str {
        &self.occurrence
    }

    /// The prepared slot, once the first poll prepared a runnable one.
    pub(super) fn slot(&self) -> Option<&JournalSlot> {
        self.slot.get()
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Option<PendingCall>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take_pending(&self) -> Option<PendingCall> {
        self.pending().take()
    }

    /// The current attempt settled `sent` as `note` says (or was dropped:
    /// `MaybeSent`).
    pub(super) fn record(&self, sent: SentState, note: CallNote) {
        if let Some(pending) = self.pending().as_mut() {
            pending.sent = sent;
            pending.note = note;
        }
    }

    /// Explains the previous attempt's call when a new attempt starts: it
    /// did not cross when it was unsent, throttled or rejected, and may
    /// have otherwise.
    pub(super) async fn flush_previous(&self) -> Result<(), OperationError> {
        let (Some(pending), Some(slot)) = (self.take_pending(), self.slot()) else {
            return Ok(());
        };
        let crossing = match (pending.sent, pending.note) {
            (SentState::NotSent, _)
            | (SentState::Sent, CallNote::Throttled | CallNote::Rejected(_)) => {
                Crossing::NotCrossed
            },
            _ => Crossing::Ambiguous,
        };
        self.owner
            .explain(slot, pending.call, crossing)
            .await
            .map_err(|refusal| self.refused("explain", refusal))
    }

    /// Asks the owner for the current attempt's call; on a grant the call
    /// is pending until the attempt is explained or settled.
    pub(super) async fn grant(&self) -> Result<(), OperationError> {
        let Some(slot) = self.slot() else {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "effect slot missing at the grant",
            ));
        };
        let call = self
            .owner
            .grant(slot)
            .await
            .map_err(|refusal| self.refused("grant", refusal))?;
        *self.pending() = Some(PendingCall {
            call,
            sent: SentState::MaybeSent,
            note: CallNote::Plain,
        });
        Ok(())
    }

    /// The attempt's registration refused after the owner granted its
    /// call: nothing crossed.
    pub(super) async fn release_refused(&self) {
        let (Some(pending), Some(slot)) = (self.take_pending(), self.slot()) else {
            return;
        };
        if let Err(refusal) = self
            .owner
            .explain(slot, pending.call, Crossing::NotCrossed)
            .await
        {
            let _ = self.refused("explain", refusal);
        }
    }

    /// Records the unit's last call from its `result` (`abnormal`: the
    /// deadline or a panic ended it) and returns the unit's result — an
    /// unknown outcome when the owner could not record a call that may have
    /// crossed.
    ///
    /// | Result | Last call | Recorded |
    /// |---|---|---|
    /// | `Ok` | any | settle `Applied` (`AppliedWithoutOutput` without `record_output`, or for an output over the 1 MiB cap) |
    /// | `Err` | `NotSent`, or throttled | explain `NotCrossed` |
    /// | `Err` | rejected | settle `Rejected` with the rejection's kind |
    /// | `Err` | `MaybeSent`, or `Sent` (a success the unit then failed) and a retryable kind | explain `Ambiguous` |
    /// | `Err` | `Sent` and a non-retryable kind | settle `Rejected` |
    pub(super) async fn finish<T>(
        &self,
        result: Result<T, OperationError>,
        abnormal: bool,
        codec: OutputCodec<T>,
    ) -> Result<T, OperationError> {
        let (Some(pending), Some(slot)) = (self.take_pending(), self.slot()) else {
            return result;
        };
        let sent = if abnormal {
            SentState::MaybeSent
        } else {
            pending.sent
        };
        match result {
            Ok(output) => {
                let encoded = if self.declaration.record_output {
                    self.encode(&output, codec)
                } else {
                    None
                };
                let outcome = match &encoded {
                    Some(bytes) => CallOutcome::Applied(bytes),
                    None => CallOutcome::AppliedWithoutOutput,
                };
                match self.owner.settle(slot, pending.call, outcome).await {
                    Ok(()) => Ok(output),
                    Err(refusal) => Err(self.unrecorded("settle", refusal)),
                }
            },
            Err(error) => {
                let kind = error.kind();
                let recorded = match (sent, pending.note) {
                    (SentState::NotSent, _) | (SentState::Sent, CallNote::Throttled) => {
                        Err(Crossing::NotCrossed)
                    },
                    (SentState::Sent, CallNote::Rejected(code)) => Ok(code),
                    (SentState::Sent, CallNote::Plain) if !kind.is_default_retryable() => {
                        Ok(ErrorKindCode::of(kind))
                    },
                    _ => Err(Crossing::Ambiguous),
                };
                let (step, written) = match recorded {
                    Ok(code) => (
                        "settle",
                        self.owner
                            .settle(slot, pending.call, CallOutcome::Rejected(code))
                            .await,
                    ),
                    Err(crossing) => (
                        "explain",
                        self.owner.explain(slot, pending.call, crossing).await,
                    ),
                };
                match written {
                    Ok(()) => Err(error),
                    Err(refusal) if matches!(recorded, Err(Crossing::NotCrossed)) => {
                        let _ = self.refused(step, refusal);
                        Err(error)
                    },
                    Err(refusal) => Err(self.unrecorded(step, refusal)),
                }
            },
        }
    }

    /// The output as the owner records it; `None` — recorded digest-only,
    /// so a resume fails `Permanent` instead of replaying — when it does
    /// not serialize or is over the ledger's 1 MiB evidence cap.
    fn encode<T>(&self, output: &T, codec: OutputCodec<T>) -> Option<Vec<u8>> {
        let Ok(encoded) = (codec.encode)(output) else {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = %self.occurrence,
                "effect output could not be serialized; recorded without output"
            );
            return None;
        };
        if encoded.len() > MAX_RECORDED_OUTPUT_LEN {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = %self.occurrence,
                output_len = encoded.len(),
                "effect output is over the recording cap; recorded without output"
            );
            return None;
        }
        Some(encoded)
    }

    /// The unit error of an owner `refusal` of `step`, logged.
    fn refused(&self, step: &'static str, refusal: JournalRefusal) -> OperationError {
        tracing::warn!(
            target: "nebula.resource",
            occurrence = %self.occurrence,
            step,
            refusal = refusal.as_str(),
            "effect owner refused a step"
        );
        refusal_error(refusal)
    }

    /// The unit error when the owner could not record a call that may have
    /// crossed: the outcome is unknown.
    fn unrecorded(&self, step: &'static str, refusal: JournalRefusal) -> OperationError {
        tracing::warn!(
            target: "nebula.resource",
            occurrence = %self.occurrence,
            step,
            refusal = refusal.as_str(),
            "effect owner could not record a call that may have crossed; outcome unknown"
        );
        OperationError::new(
            ErrorKind::OutcomeUnknown,
            "effect outcome could not be recorded by its owner",
        )
    }
}

/// The unit error of an owner refusal.
fn refusal_error(refusal: JournalRefusal) -> OperationError {
    match refusal {
        JournalRefusal::Unknown => OperationError::new(
            ErrorKind::OutcomeUnknown,
            "effect outcome unknown; no provider call granted",
        ),
        JournalRefusal::Mismatch => {
            OperationError::new(ErrorKind::Permanent, "effect occurrence mismatch")
        },
        JournalRefusal::Closed => {
            OperationError::new(ErrorKind::Cancelled, "effect owner closed; unit refused")
        },
        JournalRefusal::Unavailable
        | JournalRefusal::AcknowledgementUnknown
        | JournalRefusal::LeaseLost => OperationError::new(
            ErrorKind::Backpressure,
            "effect owner unavailable; unit refused",
        ),
    }
}

/// How an owned unit's first poll ended.
pub(super) enum Prepared<T> {
    /// Runnable: the unit runs with its slot prepared.
    Run,
    /// A recorded success, replayed without a provider call.
    Replayed(T),
    /// No run: the unit settles with this error and sent state.
    Refused(OperationError, SentState),
}

/// The first poll of an owned unit: the owner's prepare, raced against the
/// unit's cancel and `deadline`.
pub(super) async fn prepare<T>(
    shared: &UnitShared,
    effect: &OwnedEffect,
    max_invocations: NonZeroU32,
    codec: OutputCodec<T>,
    deadline: tokio::time::Instant,
) -> Prepared<T> {
    let declaration = &effect.declaration;
    let intent = JournalIntent {
        resource_key: &effect.resource_key,
        binding: &effect.binding,
        kind: declaration.kind,
        operation: declaration.name,
        version: declaration.version,
        effect: declaration.effect,
        recovery: declaration.recovery,
        record_output: declaration.record_output,
        max_invocations,
        occurrence: &effect.occurrence,
        canonical_request: &declaration.canonical_request,
        key_part: declaration.key_part.as_deref(),
    };
    let prepared = tokio::select! {
        biased;
        () = shared.cancel_token().cancelled() => {
            let cancelled = shared.refuse_if_cancelled().err().unwrap_or_else(cancelled_before_grant);
            return Prepared::Refused(cancelled, SentState::NotSent);
        },
        () = tokio::time::sleep_until(deadline) => {
            return Prepared::Refused(
                OperationError::new(
                    ErrorKind::Backpressure,
                    "the effect owner did not prepare before the unit deadline",
                ),
                SentState::NotSent,
            );
        },
        prepared = effect.owner.prepare(&intent) => prepared,
    };
    let slot = match prepared {
        Ok(slot) => slot,
        Err(refusal) => {
            let sent = if refusal == JournalRefusal::Unknown {
                SentState::MaybeSent
            } else {
                SentState::NotSent
            };
            return Prepared::Refused(effect.refused("prepare", refusal), sent);
        },
    };
    match slot.phase().clone() {
        SlotPhase::Runnable => {
            let _ = effect.slot.set(slot);
            Prepared::Run
        },
        SlotPhase::Replay(RecordedOutcome::Succeeded(bytes)) => match (codec.decode)(&bytes) {
            Ok(output) => Prepared::Replayed(output),
            Err(_) => Prepared::Refused(
                OperationError::new(
                    ErrorKind::Permanent,
                    "recorded effect output does not deserialize",
                ),
                SentState::Sent,
            ),
        },
        SlotPhase::Replay(RecordedOutcome::OutputUnavailable) => Prepared::Refused(
            OperationError::new(ErrorKind::Permanent, "effect recorded without output"),
            SentState::Sent,
        ),
        SlotPhase::Replay(RecordedOutcome::Failed(code)) => Prepared::Refused(
            OperationError::new(code.replayed_kind(), "recorded provider rejection replayed"),
            SentState::Sent,
        ),
        SlotPhase::Unknown => {
            Prepared::Refused(refusal_error(JournalRefusal::Unknown), SentState::MaybeSent)
        },
    }
}

#[cfg(test)]
#[path = "../call_effect_tests.rs"]
mod tests;
