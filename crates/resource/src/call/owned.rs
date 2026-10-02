//! The unit runtime's side of execution-owned effects: it drives an
//! `Idempotent` or `Write` unit submitted on a journaled row through the
//! row's [`EffectJournal`] (see the [`journal`](super::journal) module for
//! the seam).
//!
//! Per unit, in order:
//!
//! - **submit** ([`OwnedEffect::submit`]) — refused when the owner closed;
//!   otherwise an in-flight ticket is taken. Nothing is positioned yet: a
//!   submission dropped unpolled leaves no trace;
//! - **first poll** ([`prepare`]) — before anything is spawned, checked out,
//!   booked or read: the occurrence label is fixed from the owner's next
//!   positional ordinal (one sequence for all its effect units, whatever
//!   their resource or kind), then replay, refusal, or run;
//! - **each attempt** — the previous attempt's call is explained when the
//!   attempt starts ([`OwnedEffect::flush_previous`]); after the checkout
//!   and the credential reads, before the registration, the owner grants
//!   the call ([`OwnedEffect::grant`]) — unless a reload changed the row's
//!   configuration since the unit was submitted — and the unit's deadline
//!   shrinks to the grant's budget; a registration that then refuses
//!   explains it `NotCrossed` ([`OwnedEffect::release_refused`]);
//! - **settle** ([`OwnedEffect::finish`]) — the last call is settled or
//!   explained from the unit's result; a failure with nothing crossed is
//!   recorded with the owner ([`OwnedEffect::record_unsent`]).
//!
//! A unit cancelled before its first grant leaves only its prepare behind:
//! the slot stays prepared and a resumed unit runs it again. A grant still
//! in flight when the unit's deadline cut the operation off is not
//! explained here: the owner sees an outstanding call and treats it as an
//! ambiguous crossing.
//!
//! A recorded read ([`Recovery::Observation`]) settles differently
//! ([`OwnedEffect::finish`]): its answer reaches the caller only after the
//! owner recorded it (record before return), an answer that cannot be
//! recorded — over the cap, unserializable, or refused by the owner — is
//! withheld, and every failure without a recorded answer is recorded with
//! the owner whatever was sent. Its outcome is never unknown.

use std::{
    fmt,
    num::NonZeroU32,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use nebula_core::ResourceKey;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    cost::{Effect, SentState},
    declaration::{MAX_RECORDED_ANSWER_LEN, MAX_RECORDED_OUTPUT_LEN},
    error::OperationError,
    journal::{
        CallGrant, CallOutcome, Crossing, EffectJournal, ErrorKindCode, InFlight, JournalIntent,
        JournalRefusal, JournalSlot, RecordedOutcome, Recovery, SlotPhase, UnitKind, UnsentFailure,
    },
    managed::{UnitShared, cancelled_before_grant},
};
use crate::{dedup::SlotIdentity, error::ErrorKind};

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
    /// The row's configuration fingerprint when the unit was submitted.
    config_fingerprint: u64,
    declaration: JournalDeclaration,
    /// Assigned when the unit's first poll starts its prepare — never at
    /// submit — so only a unit that reaches its owner takes a position.
    occurrence: OnceLock<String>,
    /// Set by a `Runnable` prepare.
    slot: OnceLock<JournalSlot>,
    /// The unit's latest granted call not yet explained or settled.
    pending: Mutex<Option<PendingCall>>,
    /// Set once the unit's position was finished for its owner.
    concluded: std::sync::atomic::AtomicBool,
    /// A recorded read's answer arrived and was withheld from the caller
    /// (not recordable, or not recorded): the unit settles `MaybeSent`.
    answer_withheld: std::sync::atomic::AtomicBool,
    /// The owner's in-flight ticket: released when the unit settles
    /// ([`conclude`](Self::conclude)), or with the owned state as a
    /// fallback — never while the unit may still reach the provider.
    ticket: Mutex<Option<InFlight>>,
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
    /// Nothing more: an unclassified or unsettled attempt.
    Plain,
    /// The call succeeded: the provider applied it, whatever the unit
    /// does afterwards.
    Applied,
    /// The provider throttled the call and applied nothing.
    Throttled,
    /// The provider definitively rejected the call with this kind.
    Rejected(ErrorKindCode),
}

impl fmt::Debug for OwnedEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEffect")
            .field("occurrence", &self.occurrence.get())
            .field("recovery", &self.declaration.recovery)
            .field("prepared", &self.slot.get().is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for OwnedEffect {
    /// Fallback for a unit that never settled through its runtime (its
    /// waiter dropped before the first poll finished): the position is
    /// finished for its owner when the owned state goes.
    fn drop(&mut self) {
        self.conclude();
    }
}

impl OwnedEffect {
    /// The unit settled — its outcome is produced and recorded, it can no
    /// longer reach the provider — so a position it took is finished for
    /// its owner ([`EffectJournal::finish_occurrence`]) and its in-flight
    /// ticket released, once. Called by the unit runtime only when the unit
    /// settles, not when the handle a caller may keep is dropped: a
    /// retained completed handle holds neither the position nor the
    /// owner's drain.
    pub(super) fn conclude(&self) {
        if self
            .concluded
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        if let Some(occurrence) = self.occurrence.get() {
            self.owner.finish_occurrence(occurrence);
        }
        let ticket = self
            .ticket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(ticket);
    }

    /// The owned state of a unit of `declaration` on `key`, whose row is at
    /// `config_fingerprint`: refused when `owner` does not admit it
    /// ([`EffectJournal::admit`]: `Cancelled` when it closed, `Permanent`
    /// between its runs); otherwise with an in-flight ticket. Its occurrence
    /// is assigned later,
    /// when its first poll starts the prepare
    /// ([`assign_occurrence`](Self::assign_occurrence)): a submission
    /// dropped before it is polled takes no position.
    pub(super) fn submit(
        owner: &Arc<dyn EffectJournal>,
        binding: &SlotIdentity,
        key: &ResourceKey,
        config_fingerprint: u64,
        declaration: JournalDeclaration,
    ) -> Result<Self, OperationError> {
        let ticket = owner.admit().map_err(refusal_error)?;
        Ok(Self {
            owner: Arc::clone(owner),
            resource_key: key.clone(),
            binding: binding.clone(),
            config_fingerprint,
            declaration,
            occurrence: OnceLock::new(),
            slot: OnceLock::new(),
            pending: Mutex::new(None),
            concluded: std::sync::atomic::AtomicBool::new(false),
            answer_withheld: std::sync::atomic::AtomicBool::new(false),
            ticket: Mutex::new(Some(ticket)),
        })
    }

    /// Whether the unit is a recorded read ([`Recovery::Observation`]).
    fn observes(&self) -> bool {
        self.declaration.recovery == Recovery::Observation
    }

    /// Whether a recorded read's answer arrived and was withheld from the
    /// caller because it was not recorded: the unit settles `MaybeSent`.
    pub(super) fn answer_withheld(&self) -> bool {
        self.answer_withheld
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Assigns the unit's positional occurrence label from the owner's next
    /// occurrence ([`EffectJournal::next_occurrence`]: `unit/v1/#{ordinal:06}`
    /// by default) — one sequence for all its effect units — once; later
    /// calls return the same label.
    ///
    /// The resource, kind (operation or session), name and version are not
    /// in the label: they are bound by the effect's contract, so a changed
    /// effect under a recorded occurrence — including effects of different
    /// resources or kinds reordered — is a mismatch rather than a fresh
    /// effect.
    fn assign_occurrence(&self) -> &str {
        self.occurrence.get_or_init(|| self.owner.next_occurrence())
    }

    /// The occurrence label the owner records the effect under; empty
    /// until the unit's first poll assigned it.
    pub(super) fn occurrence(&self) -> &str {
        self.occurrence.get().map_or("", String::as_str)
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
    /// is pending until the attempt is explained or settled, and the
    /// grant's deadline (when it has a budget) is returned: the unit must
    /// not run past it.
    ///
    /// Refused, with nothing asked of the owner, when the row's
    /// configuration (`config_fingerprint`, read now) is no longer the one
    /// the effect was prepared against: a reload may have pointed the row
    /// at another destination. A grant whose budget is already spent is
    /// explained `NotCrossed` and refused.
    pub(super) async fn grant(
        &self,
        config_fingerprint: u64,
    ) -> Result<Option<tokio::time::Instant>, OperationError> {
        let Some(slot) = self.slot() else {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "effect slot missing at the grant",
            ));
        };
        if config_fingerprint != self.config_fingerprint {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
                "resource configuration changed since the effect was prepared; attempt refused"
            );
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "resource configuration changed since the effect was prepared",
            ));
        }
        let call = self
            .owner
            .grant(slot)
            .await
            .map_err(|refusal| self.refused("grant", refusal))?;
        let deadline = match call.budget() {
            None => None,
            Some(budget) if budget.is_zero() => {
                if let Err(refusal) = self.owner.explain(slot, call, Crossing::NotCrossed).await {
                    let _ = self.refused("explain", refusal);
                }
                return Err(OperationError::new(
                    ErrorKind::Backpressure,
                    "effect grant expired before the call; attempt refused",
                ));
            },
            Some(budget) => tokio::time::Instant::now().checked_add(budget),
        };
        *self.pending() = Some(PendingCall {
            call,
            sent: SentState::MaybeSent,
            note: CallNote::Plain,
        });
        Ok(deadline)
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
    /// | `Err` | none pending (every call already explained) | record the unsent failure |
    /// | `Err` | `NotSent`, or throttled | explain `NotCrossed`, then record the unsent failure |
    /// | `Err` | rejected | settle `Rejected` with the rejection's kind |
    /// | `Err` | `MaybeSent`, or applied (a success the unit then failed) and a retryable kind | explain `Ambiguous` |
    /// | `Err` | applied and a non-retryable kind | settle `AppliedWithoutOutput` |
    ///
    /// A local failure after a successful call never records a provider
    /// rejection: the provider applied the effect, so the ledger says so
    /// (without an output, which the unit never produced) and a resume
    /// fails `Permanent` "recorded without output" — never sending it
    /// again. Only a call the provider rejected records a rejection.
    ///
    /// A recorded read settles by its own table
    /// ([`finish_observation`](Self::finish_observation)).
    pub(super) async fn finish<T>(
        &self,
        result: Result<T, OperationError>,
        abnormal: bool,
        codec: OutputCodec<T>,
    ) -> Result<T, OperationError> {
        let Some(slot) = self.slot() else {
            return result;
        };
        if self.observes() {
            return self.finish_observation(slot, result, abnormal, codec).await;
        }
        let Some(pending) = self.take_pending() else {
            // No call in flight: every call the unit was granted is already
            // explained, so a failure now sent nothing more.
            if let Err(error) = &result {
                self.record_unsent(slot, error.kind()).await;
            }
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
                    (SentState::Sent, CallNote::Rejected(code)) => Ok(CallOutcome::Rejected(code)),
                    (SentState::Sent, CallNote::Applied) if !kind.is_default_retryable() => {
                        Ok(CallOutcome::AppliedWithoutOutput)
                    },
                    _ => Err(Crossing::Ambiguous),
                };
                let (step, written) = match recorded {
                    Ok(outcome) => (
                        "settle",
                        self.owner.settle(slot, pending.call, outcome).await,
                    ),
                    Err(crossing) => (
                        "explain",
                        self.owner.explain(slot, pending.call, crossing).await,
                    ),
                };
                match written {
                    Ok(()) => {
                        if matches!(recorded, Err(Crossing::NotCrossed)) {
                            self.record_unsent(slot, kind).await;
                        }
                        Err(error)
                    },
                    Err(refusal) if matches!(recorded, Err(Crossing::NotCrossed)) => {
                        let _ = self.refused(step, refusal);
                        Err(error)
                    },
                    Err(refusal) => Err(self.unrecorded(step, refusal)),
                }
            },
        }
    }

    /// Records a recorded read's last call from its `result` and returns
    /// the unit's result. Record before return: an answer reaches the
    /// caller only once the owner recorded it.
    ///
    /// | Result | Last call | Recorded | Caller sees |
    /// |---|---|---|---|
    /// | `Ok` | pending | settle `Applied` with the answer | the answer, once settled |
    /// | `Ok` | pending, answer unserializable or over the cap | explain `Ambiguous`, record the failure | `Permanent`, `MaybeSent` |
    /// | `Ok` | none (no provider answer) | nothing | the output |
    /// | `Err` | rejected | settle `Rejected` | the rejection, once settled |
    /// | `Err` | `NotSent`, or throttled | explain `NotCrossed`, record the failure | the error |
    /// | `Err` | anything else | explain `Ambiguous`, record the failure | the error |
    /// | `Err` | none pending | record the failure | the error |
    ///
    /// A settle or explanation the owner does not take fails the unit
    /// `Transient` / `MaybeSent` — retryable, never an unknown outcome — and
    /// withholds the answer: the caller never sees one a replay would not.
    async fn finish_observation<T>(
        &self,
        slot: &JournalSlot,
        result: Result<T, OperationError>,
        abnormal: bool,
        codec: OutputCodec<T>,
    ) -> Result<T, OperationError> {
        let error = match (result, self.take_pending()) {
            (Ok(output), Some(pending)) => match self.encode_answer(&output, codec) {
                Ok(answer) => {
                    return match self
                        .owner
                        .settle(slot, pending.call, CallOutcome::Applied(&answer))
                        .await
                    {
                        Ok(()) => Ok(output),
                        Err(refusal) => Err(self.withheld("settle", refusal)),
                    };
                },
                Err(error) => {
                    self.answer_withheld
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Err(refusal) = self
                        .owner
                        .explain(slot, pending.call, Crossing::Ambiguous)
                        .await
                    {
                        return Err(self.withheld("explain", refusal));
                    }
                    error
                },
            },
            // No call is pending: the output is no provider answer of this
            // unit (every call it was granted is already explained).
            (Ok(output), None) => return Ok(output),
            (Err(error), Some(pending)) => {
                let sent = if abnormal {
                    SentState::MaybeSent
                } else {
                    pending.sent
                };
                let crossing = match (sent, pending.note) {
                    (SentState::Sent, CallNote::Rejected(code)) => {
                        // A definitive answer: recorded, replayed as is.
                        return match self
                            .owner
                            .settle(slot, pending.call, CallOutcome::Rejected(code))
                            .await
                        {
                            Ok(()) => Err(error),
                            Err(refusal) => Err(self.withheld("settle", refusal)),
                        };
                    },
                    (SentState::NotSent, _) | (SentState::Sent, CallNote::Throttled) => {
                        Crossing::NotCrossed
                    },
                    _ => Crossing::Ambiguous,
                };
                if let Err(refusal) = self.owner.explain(slot, pending.call, crossing).await {
                    return Err(self.withheld("explain", refusal));
                }
                error
            },
            (Err(error), None) => error,
        };
        // No answer is recorded: the failure is, whatever was sent, so a
        // run that must not ask again fails the same way.
        self.record_unsent(slot, error.kind()).await;
        Err(error)
    }

    /// A recorded read's answer as the owner records it, or why it cannot
    /// be: it does not serialize, or it is over
    /// [`MAX_RECORDED_ANSWER_LEN`]. Never digest-only.
    fn encode_answer<T>(
        &self,
        output: &T,
        codec: OutputCodec<T>,
    ) -> Result<Vec<u8>, OperationError> {
        let Ok(encoded) = (codec.encode)(output) else {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
                "recorded read's answer could not be serialized; withheld"
            );
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "recorded read's answer does not serialize; withheld",
            ));
        };
        if encoded.len() > MAX_RECORDED_ANSWER_LEN {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
                output_len = encoded.len(),
                "recorded read's answer is over the recording cap; withheld"
            );
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "recorded read's answer is over the recording cap; withheld",
            ));
        }
        Ok(encoded)
    }

    /// The unit error when the owner did not record a recorded read's
    /// answer or call: the answer is withheld and the unit may be asked
    /// again — retryable, never an unknown outcome.
    fn withheld(&self, step: &'static str, refusal: JournalRefusal) -> OperationError {
        self.answer_withheld
            .store(true, std::sync::atomic::Ordering::SeqCst);
        tracing::warn!(
            target: "nebula.resource",
            occurrence = self.occurrence(),
            step,
            refusal = refusal.as_str(),
            "effect owner did not record a recorded read; answer withheld"
        );
        OperationError::new(
            ErrorKind::Transient,
            "recorded read could not be recorded by its owner; answer withheld",
        )
    }

    /// Tells the owner how the unit failed while sending nothing
    /// ([`EffectJournal::record_unsent_failure`]) — or, for a recorded
    /// read, failed with no answer recorded — before the failure is
    /// returned: the unit's result stands either way, and an owner that
    /// could not record it fails closed on its side.
    pub(super) async fn record_unsent(&self, slot: &JournalSlot, kind: &ErrorKind) {
        if let Err(refusal) = self
            .owner
            .record_unsent_failure(slot, UnsentFailure::of(kind))
            .await
        {
            tracing::debug!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
                refusal = refusal.as_str(),
                "effect owner did not record how the unsent unit failed"
            );
        }
    }

    /// The output as the owner records it; `None` — recorded digest-only,
    /// so a resume fails `Permanent` instead of replaying — when it does
    /// not serialize or is over the ledger's 1 MiB evidence cap.
    fn encode<T>(&self, output: &T, codec: OutputCodec<T>) -> Option<Vec<u8>> {
        let Ok(encoded) = (codec.encode)(output) else {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
                "effect output could not be serialized; recorded without output"
            );
            return None;
        };
        if encoded.len() > MAX_RECORDED_OUTPUT_LEN {
            tracing::warn!(
                target: "nebula.resource",
                occurrence = self.occurrence(),
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
            occurrence = self.occurrence(),
            step,
            refusal = refusal.as_str(),
            "effect owner refused a step"
        );
        if self.observes() && refusal == JournalRefusal::Unknown {
            // A recorded read's outcome is never unknown: the owner's
            // ceiling for asking again is spent.
            return spent_read().refused_by_owner();
        }
        refusal_error(refusal).refused_by_owner()
    }

    /// The unit error when the owner could not record a call that may have
    /// crossed: the outcome is unknown.
    fn unrecorded(&self, step: &'static str, refusal: JournalRefusal) -> OperationError {
        tracing::warn!(
            target: "nebula.resource",
            occurrence = self.occurrence(),
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

/// The unit error of a recorded read whose owner may not ask it again: its
/// ceiling is spent. `Exhausted`, never an unknown outcome.
fn spent_read() -> OperationError {
    OperationError::new(
        ErrorKind::Exhausted { retry_after: None },
        "recorded read may not be asked again: its owner's ceiling is spent",
    )
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
        JournalRefusal::SlotCapExceeded => OperationError::new(
            ErrorKind::Permanent,
            "effect journal slot cap reached; unit refused",
        ),
        // The failure the earlier run saw, kind and payload; `Permanent` when
        // it was not recorded (a slot recorded before failures were).
        JournalRefusal::Superseded(failure) => OperationError::new(
            failure.map_or(ErrorKind::Permanent, UnsentFailure::kind),
            "effect failed unsent in an earlier run that moved past it; not sent again",
        ),
        JournalRefusal::ConcurrencyLimit => OperationError::new(
            ErrorKind::Permanent,
            "too many interleaved concurrent effects: the lower effects still open form more \
             separate runs than the journal records (64); unit refused",
        ),
        JournalRefusal::BetweenRuns => OperationError::new(
            ErrorKind::Permanent,
            "effect submitted while its owner has no open run (between stateful iterations); \
             unit refused",
        ),
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

/// Tells the owner, when dropped, that the unit labelled `occurrence`
/// stopped preparing ([`EffectJournal::release_occurrence`]).
struct ReleaseOccurrence<'a> {
    owner: &'a dyn EffectJournal,
    occurrence: &'a str,
}

impl Drop for ReleaseOccurrence<'_> {
    fn drop(&mut self) {
        self.owner.release_occurrence(self.occurrence);
    }
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
    // A unit already cancelled, or whose owner closed, never reaches the
    // owner: it takes no position.
    if shared.cancel_token().is_cancelled() {
        let cancelled = shared
            .refuse_if_cancelled()
            .err()
            .unwrap_or_else(cancelled_before_grant);
        return Prepared::Refused(cancelled, SentState::NotSent);
    }
    if effect.owner.is_closed() {
        return Prepared::Refused(
            effect.refused("prepare", JournalRefusal::Closed),
            SentState::NotSent,
        );
    }
    // The position is taken here, when preparing begins, in the order units
    // reach their owner — never at submit: a submission dropped before its
    // first poll consumes no ordinal, so a branch that builds and drops one
    // cannot shift the effects after it onto unrecorded positions.
    let occurrence = effect.assign_occurrence();
    // However this first poll ends — prepared, refused, cancelled, past the
    // deadline before the owner was reached, or dropped — the owner learns
    // the position stopped preparing.
    let _released = ReleaseOccurrence {
        owner: effect.owner.as_ref(),
        occurrence,
    };
    let declaration = &effect.declaration;
    let intent = JournalIntent {
        resource_key: &effect.resource_key,
        binding: &effect.binding,
        config_fingerprint: effect.config_fingerprint,
        kind: declaration.kind,
        operation: declaration.name,
        version: declaration.version,
        effect: declaration.effect,
        recovery: declaration.recovery,
        record_output: declaration.record_output,
        max_invocations,
        occurrence,
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
            let sent = if refusal == JournalRefusal::Unknown && !effect.observes() {
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
        // A recorded read is never unknown: its ceiling is spent.
        SlotPhase::Unknown if effect.observes() => {
            Prepared::Refused(spent_read(), SentState::NotSent)
        },
        SlotPhase::Unknown => {
            Prepared::Refused(refusal_error(JournalRefusal::Unknown), SentState::MaybeSent)
        },
    }
}

#[cfg(test)]
#[path = "../call_effect_tests.rs"]
mod tests;
