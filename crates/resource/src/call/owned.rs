//! The unit runtime's side of execution-owned effects: it drives a unit
//! submitted with `submit_effect` / `session_effect` through the row's
//! [`UnitEffectOwner`] (see the [`owner`](super::owner) module for the
//! seam).
//!
//! Per unit, in order:
//!
//! - **submit** ([`OwnedEffect::submit`]) — refused when the owner closed;
//!   otherwise the occurrence label is fixed (the author's, or the next
//!   ordinal) and an in-flight ticket taken;
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
    effect::{EffectContract, EffectRecovery, IdempotencyKeyPart, OccurrenceLabel, Recorded},
    error::OperationError,
    managed::{UnitShared, cancelled_before_grant},
    owner::{
        Crossing, ErrorKindCode, OwnerRefusal, OwnerTicket, RecordedOutcome, SlotPhase, UnitCall,
        UnitEffectOwner, UnitIntent, UnitOutcome, UnitSlot,
    },
};
use crate::{dedup::SlotIdentity, error::ErrorKind};

/// Longest occurrence label the owner's ledger accepts, in bytes.
const MAX_OCCURRENCE_LABEL_LEN: usize = 512;

/// Largest canonical request, in bytes.
const MAX_CANONICAL_REQUEST_LEN: usize = 1024 * 1024;

/// Computes an owned unit's canonical request and developer key part from
/// its operation, at the first poll.
pub(super) type RequestFn<O> =
    Box<dyn FnOnce(&O) -> Result<(Vec<u8>, Option<IdempotencyKeyPart>), OperationError> + Send>;

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

/// What `submit_effect` / `session_effect` declare about a unit's effect.
pub(super) struct EffectDeclaration<O, T> {
    pub(super) contract: EffectContract,
    pub(super) recovery: EffectRecovery,
    pub(super) recorded: Recorded,
    pub(super) occurrence: Option<OccurrenceLabel>,
    pub(super) request: RequestFn<O>,
    pub(super) codec: OutputCodec<T>,
}

impl<O, T> EffectDeclaration<O, T> {
    /// Refuses a declaration whose contract is malformed or whose recovery
    /// disagrees with `effect` (a `Read` included).
    pub(super) fn check(&self, effect: Effect) -> Result<(), OperationError> {
        self.contract.validate()?;
        self.recovery.check(effect)
    }
}

/// The parts of an owned unit's declaration its first poll uses.
pub(super) struct EffectPlan<O, T> {
    pub(super) request: RequestFn<O>,
    pub(super) codec: OutputCodec<T>,
}

/// An owned unit admitted at submit: its shared owned state and its
/// first-poll plan.
pub(super) type OwnedSubmit<O, T> = (OwnedEffect, EffectPlan<O, T>);

/// The owned-effect state of one unit, shared by its handle, its runtime
/// task and its attempts.
pub(super) struct OwnedEffect {
    owner: Arc<dyn UnitEffectOwner>,
    resource_key: ResourceKey,
    binding: SlotIdentity,
    effect: Effect,
    contract: EffectContract,
    recovery: EffectRecovery,
    recorded: Recorded,
    occurrence: String,
    /// Set by a `Runnable` prepare.
    slot: OnceLock<UnitSlot>,
    /// The unit's latest granted call not yet explained or settled.
    pending: Mutex<Option<PendingCall>>,
    _ticket: OwnerTicket,
}

/// A granted call and how its attempt settled so far.
#[derive(Debug, Clone, Copy)]
struct PendingCall {
    call: UnitCall,
    /// `MaybeSent` until the attempt settles.
    sent: SentState,
    /// The attempt reported a provider throttle.
    throttled: bool,
}

impl fmt::Debug for OwnedEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEffect")
            .field("occurrence", &self.occurrence)
            .field("contract", &self.contract)
            .field("recovery", &self.recovery)
            .field("prepared", &self.slot.get().is_some())
            .finish_non_exhaustive()
    }
}

impl OwnedEffect {
    /// The owned state of a unit of `declaration`, declaring `effect`, on
    /// `key`: refused `Cancelled` when `owner` closed; otherwise with its
    /// occurrence label and an in-flight ticket.
    pub(super) fn submit<O, T>(
        owner: &Arc<dyn UnitEffectOwner>,
        binding: &SlotIdentity,
        key: &ResourceKey,
        effect: Effect,
        declaration: &EffectDeclaration<O, T>,
    ) -> Result<Self, OperationError> {
        if owner.is_closed() {
            return Err(OperationError::new(
                ErrorKind::Cancelled,
                "effect owner closed; unit refused",
            ));
        }
        let tail = match &declaration.occurrence {
            Some(label) => label.as_str().to_owned(),
            None => format!("#{:06}", owner.next_ordinal(key, declaration.contract)),
        };
        let occurrence = format!("unit/v1/{key}/{}/{tail}", declaration.contract.id());
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
            effect,
            contract: declaration.contract,
            recovery: declaration.recovery,
            recorded: declaration.recorded,
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
    pub(super) fn slot(&self) -> Option<&UnitSlot> {
        self.slot.get()
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Option<PendingCall>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take_pending(&self) -> Option<PendingCall> {
        self.pending().take()
    }

    /// The current attempt settled `sent` (or was dropped: `MaybeSent`).
    pub(super) fn record(&self, sent: SentState) {
        if let Some(pending) = self.pending().as_mut() {
            pending.sent = sent;
        }
    }

    /// The current attempt reported a provider throttle.
    pub(super) fn note_throttled(&self) {
        if let Some(pending) = self.pending().as_mut() {
            pending.throttled = true;
        }
    }

    /// Explains the previous attempt's call when a new attempt starts: it
    /// did not cross when it was unsent or throttled, and may have
    /// otherwise.
    pub(super) async fn flush_previous(&self) -> Result<(), OperationError> {
        let (Some(pending), Some(slot)) = (self.take_pending(), self.slot()) else {
            return Ok(());
        };
        let crossing = match pending.sent {
            SentState::NotSent => Crossing::NotCrossed,
            SentState::Sent if pending.throttled => Crossing::NotCrossed,
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
            throttled: false,
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
    /// | `Ok` | any | settle `Applied` (`AppliedWithoutOutput` for `DigestOnly`) |
    /// | `Err` | `NotSent`, or `Sent` and `Exhausted` | explain `NotCrossed` |
    /// | `Err` | `MaybeSent`, or `Sent` and a retryable kind | explain `Ambiguous` |
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
                let encoded = match self.recorded {
                    Recorded::Output => {
                        let encoded = (codec.encode)(&output).ok();
                        if encoded.is_none() {
                            tracing::warn!(
                                target: "nebula.resource",
                                occurrence = %self.occurrence,
                                "effect output could not be serialized; recorded without output"
                            );
                        }
                        encoded
                    },
                    Recorded::DigestOnly => None,
                };
                let outcome = match &encoded {
                    Some(bytes) => UnitOutcome::Applied(bytes),
                    None => UnitOutcome::AppliedWithoutOutput,
                };
                match self.owner.settle(slot, pending.call, outcome).await {
                    Ok(()) => Ok(output),
                    Err(refusal) => Err(self.unrecorded("settle", refusal)),
                }
            },
            Err(error) => {
                let kind = error.kind();
                let recorded = match sent {
                    SentState::NotSent => Err(Crossing::NotCrossed),
                    SentState::Sent if matches!(kind, ErrorKind::Exhausted { .. }) => {
                        Err(Crossing::NotCrossed)
                    },
                    SentState::Sent if !kind.is_default_retryable() => Ok(ErrorKindCode::of(kind)),
                    _ => Err(Crossing::Ambiguous),
                };
                let (step, written) = match recorded {
                    Ok(code) => (
                        "settle",
                        self.owner
                            .settle(slot, pending.call, UnitOutcome::Rejected(code))
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

    /// The unit error of an owner `refusal` of `step`, logged.
    fn refused(&self, step: &'static str, refusal: OwnerRefusal) -> OperationError {
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
    fn unrecorded(&self, step: &'static str, refusal: OwnerRefusal) -> OperationError {
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
fn refusal_error(refusal: OwnerRefusal) -> OperationError {
    match refusal {
        OwnerRefusal::Unknown => OperationError::new(
            ErrorKind::OutcomeUnknown,
            "effect outcome unknown; no provider call granted",
        ),
        OwnerRefusal::Mismatch => {
            OperationError::new(ErrorKind::Permanent, "effect occurrence mismatch")
        },
        OwnerRefusal::Closed => {
            OperationError::new(ErrorKind::Cancelled, "effect owner closed; unit refused")
        },
        OwnerRefusal::Unavailable
        | OwnerRefusal::AcknowledgementUnknown
        | OwnerRefusal::LeaseLost => OperationError::new(
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

/// The first poll of an owned unit: the canonical `request`, then the
/// owner's prepare, raced against the unit's cancel and `deadline`.
pub(super) async fn prepare<T>(
    shared: &UnitShared,
    effect: &OwnedEffect,
    max_invocations: NonZeroU32,
    request: Result<(Vec<u8>, Option<IdempotencyKeyPart>), OperationError>,
    codec: OutputCodec<T>,
    deadline: tokio::time::Instant,
) -> Prepared<T> {
    let (canonical_request, key_part) = match request {
        Ok(request) => request,
        Err(error) => return Prepared::Refused(error, SentState::NotSent),
    };
    if canonical_request.is_empty() || canonical_request.len() > MAX_CANONICAL_REQUEST_LEN {
        return Prepared::Refused(
            OperationError::new(
                ErrorKind::Permanent,
                "canonical request must be 1 byte to 1 MiB",
            ),
            SentState::NotSent,
        );
    }
    let intent = UnitIntent {
        resource_key: &effect.resource_key,
        binding: &effect.binding,
        contract: effect.contract,
        effect: effect.effect,
        recovery: effect.recovery,
        recorded: effect.recorded,
        max_invocations,
        occurrence: &effect.occurrence,
        canonical_request: &canonical_request,
        key_part: key_part.as_ref(),
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
            let sent = if refusal == OwnerRefusal::Unknown {
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
            Prepared::Refused(refusal_error(OwnerRefusal::Unknown), SentState::MaybeSent)
        },
    }
}

#[cfg(test)]
#[path = "../call_effect_tests.rs"]
mod tests;
