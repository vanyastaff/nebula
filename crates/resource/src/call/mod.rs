//! Managed call facade: provider calls made through a lease as admitted,
//! budgeted, settled units of work.
//!
//! A lease ([`ResourceGuard`](crate::ResourceGuard)) becomes a [`Lease`]
//! facade with [`into_lease`](crate::ResourceGuard::into_lease). Action
//! code then describes each provider call as an [`Operation`] and
//! [`submit`](Lease::submit)s it; the facade has no `Deref` to the
//! instance, so a call cannot bypass it by accident.
//!
//! # Unit and attempt
//!
//! - A **unit** ([`Submission`]) is one submitted operation: owned intent, a
//!   runtime-owned task, a deadline, an attempt budget, the credential slots
//!   pinned at its first grant, and exactly one settled outcome (Design
//!   CONTRACT.md:36-46, DX-API.md:106-114).
//! - An **attempt** ([`Attempt`]) is one admitted provider call.
//!   [`OperationCx::attempt`] is the unit's single linearization point: budget,
//!   admission against the lease, quota booking at the attempt's [`Cost`],
//!   the strict credential read, registration and grant (CONTRACT.md:64,
//!   :88-92). There is no retry loop
//!   here: an operation that retries asks for another attempt, within
//!   [`Operation::max_attempts`] (DX-API.md:36, CONTRACT.md:96).
//!
//! A unit is lazy. Its first poll waits for one of the lease's unit slots on
//! the caller's task, then the runtime runs it on its own task until it
//! settles. Dropped before its first poll it never ran; dropped later, the
//! caller only stops waiting — the runtime still settles the unit, and the
//! lease is released only after the last unit ends (DX-API.md:114).
//!
//! # Settled outcome
//!
//! Each attempt is settled by the author with a [`SentState`]; an attempt
//! dropped unsettled counts as `MaybeSent`. The unit folds them:
//!
//! - no attempt granted → `NotSent`, whatever the author reported (the
//!   runtime proves nothing was sent);
//! - otherwise the worst attempt, `MaybeSent > Sent > NotSent`;
//! - the deadline or a panic after a grant → `MaybeSent`.
//!
//! A failed unit's [`OperationError`] carries that state and the operation's
//! [`Effect`], and [`OperationError::is_retryable`] decides from them: a unit that
//! may have been applied is retried only when its effect is replay safe.
//! Converted into a resource [`Error`](crate::Error), a retry-unsafe unit
//! becomes [`ErrorKind::OutcomeUnknown`](crate::ErrorKind::OutcomeUnknown)
//! (CONTRACT.md:100, :110-112).
//!
//! | Situation | Kind | Sent |
//! |---|---|---|
//! | lease closing (removal, shutdown) | `Cancelled` | `NotSent` |
//! | credential suspension closed the lease | `CredentialUnavailable` | `NotSent` |
//! | credential revoke tainted the row | `Revoked` | `NotSent` |
//! | strict read refused (blocked, outage, absent, new material) | `CredentialUnavailable` | `NotSent` |
//! | the unit's pinned slots were rotated since its first grant | `CredentialUnavailable { Rebinding }` | `NotSent` |
//! | local quota slot past the deadline | `Exhausted` | `NotSent` |
//! | limit store down, unit slots full until the deadline | `Backpressure` | `NotSent` |
//! | provider throttled (`Attempt::report(Verdict::Throttled)`) | `Exhausted` | `Sent` |
//! | retry-unsafe effect with an unknown outcome | `OutcomeUnknown` (as `Error`) | `Sent` / `MaybeSent` |
//!
//! Author errors bridge in through the usual route: a
//! `#[derive(ClassifyError)]` enum converts into [`Error`](crate::Error),
//! and `?` converts that into [`OperationError`], keeping only its kind.
//!
//! # Closing
//!
//! [`OperationCx::closing`] and [`Lease::closing`] are the lease generation's
//! [`LeaseClosing`](crate::LeaseClosing). Once it fires, new attempts are
//! refused; an attempt already granted is not aborted — an operation that
//! wants to stop early selects on [`closed`](crate::LeaseClosing::closed)
//! (CONTRACT.md:74). A credential refresh or a config reload leaves the
//! lease's generation open, so neither interrupts a unit (canon §13.2).
//!
//! # Credentials
//!
//! On a manager with a credential availability observer (a strict manager),
//! every attempt on a credential-bound row reads each bound credential's
//! availability after every wait of the attempt, outside every lock, and is
//! registered under the manager's admission lock: a blocked credential
//! suspends the row and closes its leases, a store outage refuses without
//! changing the row, newer material refuses `Rebinding` until it is
//! installed (CONTRACT.md:40-41, :64, :77; QUOTA-DX.md:33). Attempts share
//! reads join-next with each other and with acquires; an attempt only takes
//! a read issued after it arrived. A slot-less row and an interim manager
//! read nothing. A row serving a facade reports
//! [`CredentialAdmissionProfile::StrictPerAttempt`](crate::CredentialAdmissionProfile::StrictPerAttempt).
//!
//! [`PinSlots::pin_slots`] runs once per unit, at its first grant — after
//! that attempt's read, so the first attempt runs on the binding its read
//! validated — bracketed by the slots' generations and retaken when a
//! rotation raced it. Every attempt of the unit sees that snapshot through
//! [`Attempt::credentials`], the only way the facade discloses credential material
//! (CONTRACT.md:73, DX-API.md:136). A rotation mid-unit reaches the next
//! unit: on a strict manager a later attempt whose pin it superseded is
//! refused `Rebinding`, unsent, and the unit's settled outcome decides
//! whether it is retried (a `Write` whose earlier attempt was sent has an
//! unknown outcome). There is no re-pin mid-unit.
//!
//! # Rate limit
//!
//! [`into_lease`](crate::ResourceGuard::into_lease) latches the row's
//! profile to [`RateLimitProfile::PerAttempt`](crate::RateLimitProfile::PerAttempt):
//! each granted attempt books its [`Cost`], and acquires only honour pauses
//! (QUOTA-DX.md:32, :34). A quota wait is raced against the lease's
//! generation and against [`Submission::cancel`], so a closed lease never sends
//! (CONTRACT.md:88-92).
//!
//! # Per-unit checkout and sessions
//!
//! A [`ResourceHandle`] ([`Manager::handle`](crate::Manager::handle))
//! runs the same [`Operation`]s without a lease: every attempt books its
//! quota and waits for the row gate with nothing held, then checks out an
//! instance of its own through the acquire pipeline's admission and
//! releases it when the attempt ends (see the `row` module docs for the
//! order, including the second read after a create and the second lock of a
//! strict row). On a pooled [`SessionProvider`],
//! [`ResourceHandle::session`] runs one session per unit — one attempt, never
//! retried, the cost booked once — and settles it from what
//! [`SessionProvider::close`] reported: `Committed` is `Sent`, `RolledBack`
//! `NotSent`, `Unknown` `MaybeSent` with the instance destroyed; an
//! abandoned session (deadline, panic) is `MaybeSent` and destroys its
//! instance too. A [`SessionBinding::Connection`] session runs only on an
//! instance built at the credential slot epoch its unit pinned
//! (CONTRACT.md:73, :80, :110-114; DX-API.md:136, :151-153).
//!
//! A row facade is bound to the caller that built it. Its units inherit the
//! caller context's cancellation token: once it fires, a unit whose first
//! attempt was not granted yet — waiting for quota, the row gate, a strict
//! read or its checkout, or not started — settles `Cancelled` and `NotSent`,
//! and the grant itself re-checks it, so the cancel and the first grant race
//! there and nowhere later. After the first grant the cancel is ignored:
//! dispatched work runs on to the unit's deadline (DX-API.md:114). The
//! caller's deadline, when it has one (an action's: the execution budget),
//! bounds every unit's deadline below [`OPERATION_DEADLINE_CAP`]. Actions reach a
//! row through a `#[resource]` field of type `ResourceHandle<R>`, served by the
//! engine's resource accessor through the type-erased
//! [`Manager::handle_any`](crate::Manager::handle_any).
//!
//! # Execution-owned effects
//!
//! An action row built without effect-owner authority
//! ([`Manager::handle_any_read_only`](crate::Manager::handle_any_read_only))
//! runs reads only. A row built with it
//! ([`Manager::handle_any_journaled`](crate::Manager::handle_any_journaled))
//! carries the execution's [`UnitEffectOwner`](owner::UnitEffectOwner), and
//! its effects go through [`ResourceHandle::submit_effect`] (an
//! [`EffectOperation`]) or [`ResourceHandle::session_effect`]; a plain
//! [`submit`](ResourceHandle::submit) or [`session`](ResourceHandle::session) of an
//! `Idempotent` or `Write` unit is refused `Permanent` / `NotSent`. The
//! resource runtime never writes durable effect state; it drives the owner
//! seam ([`owner`]) at fixed points of the unit:
//!
//! 1. **Submit** — the declaration is checked ([`EffectContract`], an
//!    [`EffectRecovery`] that agrees with the [`Effect`]; a read is
//!    refused), the occurrence label
//!    `unit/v1/{resource_key}/{contract_id}/{label}` is fixed — the author's
//!    [`OccurrenceLabel`], or `#` and the unit's six-digit submit ordinal
//!    per resource and contract, so labels sort in program order — and an
//!    in-flight ticket is taken. A closed owner refuses `Cancelled`.
//! 2. **First poll** — the owner prepares the effect from the canonical
//!    request and the optional [`IdempotencyKeyPart`] before anything is
//!    booked, read or checked out: a recorded success replays its output
//!    without a provider call; a recorded rejection, a digest-only success
//!    ([`Recorded::DigestOnly`]) and an unknown outcome fail without one.
//! 3. **Each attempt** — the previous attempt's call is explained when the
//!    attempt starts; after the checkout and the credential reads, right
//!    before the registration, the owner grants the attempt's call (the
//!    only provider-call authority); a registration that refuses after the
//!    grant is explained as not crossed.
//! 4. **Settle** — the unit's last call is recorded from its result: a
//!    success with its output, a definitive rejection with its kind
//!    ([`ErrorKindCode`](owner::ErrorKindCode)), otherwise how the call
//!    crossed (`NotSent` or a throttle: not crossed; `MaybeSent` or a
//!    retryable failure after `Sent`: ambiguous). A record that fails after
//!    a possible crossing fails the unit `OutcomeUnknown`.
//!
//! [`OperationCx::idempotency_key`] and [`SessionCx::idempotency_key`] are the provider
//! idempotency key ([`IdempotencyKey`]) the owner recorded before the first
//! attempt: the same for every attempt, retry and resume. A unit cancelled
//! before its first grant leaves only its prepare behind, which a resumed
//! unit runs again. The lease facade [`Lease`] has no owner. Interim: the
//! engine does not hand out owned rows yet; its owner over the operation
//! ledger comes with the engine wiring.
//!
//! # Interim defaults
//!
//! Where the design package leaves a value open, this module picks one and
//! revisits it before the surface is frozen (QUOTA-DX.md:57: not frozen,
//! hence not in any prelude):
//!
//! - A unit's deadline is at most [`OPERATION_DEADLINE_CAP`] (5 minutes, the
//!   rate limit's `DEFAULT_MAX_PENALTY`); [`Submission::with_deadline`] can only
//!   shorten it.
//! - At most one unit runs at a time on a lease whose topology checks out
//!   exclusively (`Pooled`, `Bounded`), and 64 on a shared instance
//!   (`Resident`, custom). Further units wait for a slot until their
//!   deadline, then fail with `Backpressure`.
//! - A cost booked for an attempt that is cancelled before it reaches the
//!   provider is not refunded (QUOTA-DX.md:32 baseline).
//! - A pooled [`Lease`] lease stays checked out while its units wait for
//!   quota and while their strict credential reads run; a [`ResourceHandle`]
//!   checks out per attempt instead, after those waits (QUOTA-DX.md:41,
//!   :43).
//! - The row gate of a [`ResourceHandle`] is sized to the topology's capacity
//!   at its first use and ignores a reload that resizes the pool; a pool
//!   saturated by plain leases refuses a row attempt `Backpressure` (the
//!   booked cost is forfeited).
//! - A row attempt refuses a suspended row without reading it; the next
//!   acquire or activation reopens it.
//! - Sessions are `Pooled`-only; the unit deadline cap also bounds a
//!   session, so long-lived subscriptions (`LISTEN`/`NOTIFY`, IMAP `IDLE`)
//!   are not supported (an interval profile needs an ADR revising the
//!   per-unit rule). A unit of a row awaited inside a session of the same
//!   row is refused `Permanent`; other nested-session semantics, refunds,
//!   ordering across several budgets, the owner of commit-unknown
//!   recovery and sessions on shared instances are open in the design
//!   package.
//! - `Sent` with `Exhausted` means the provider refused and applied
//!   nothing, so it stays retryable for any effect.
//! - The effect vocabulary is [`Effect::Read`], [`Effect::Idempotent`] and
//!   [`Effect::Write`] (the default; DX-API.md:110).
//! - A superseded pin refuses `Rebinding` with the reason's one-second
//!   retry hint; the unit is not re-prepared on the new material by the
//!   runtime (an opt-in resubmission of a cloneable operation is a
//!   follow-up).
//!
//! # Streaming
//!
//! A [`StreamOperation`] submitted with [`Lease::submit_streaming`] or
//! [`ResourceHandle::submit_streaming`] runs as one ordinary unit that also
//! sends items through a bounded [`StreamSink`]; the caller pulls them from
//! [`Streaming`], then the unit's error, if any, once. A mid-stream failure
//! is never an item, a dropped or cancelled consumer ends the operation at
//! its next send, and the lease closing is honoured by selecting on
//! [`OperationCx::closing`] (CONTRACT.md:57). On a row, each attempt still checks
//! out per attempt after its quota and gate waits; items sent while an
//! attempt is alive keep its checkout, and a consumer gone mid-stream
//! releases it with its gate permit. [`StreamOperation`] documents the
//! delivery rules.
//!
//! # Observability
//!
//! Each unit runs in a `nebula.resource.unit` span recording its key,
//! operation type, attempts granted, sent state and outcome.
//! [`ResourceOpsMetrics`](crate::ResourceOpsMetrics) counts attempts granted
//! and refused by the facade — separate from a driver's own retries inside
//! an attempt (CONTRACT.md:134) — units settled by sent state, row
//! checkouts by whether they created their instance, and sessions by how
//! they ended; a session unit's span names its operation `session`. A unit
//! that ends with an unknown outcome publishes
//! [`ResourceEvent::OperationOutcomeUnknown`](crate::ResourceEvent::OperationOutcomeUnknown).

mod cost;
mod effect;
mod error;
mod managed;
mod owned;
pub mod owner;
mod pin;
mod row;
mod session;
mod stream;
mod strict;

use std::{future::Future, num::NonZeroU32};

pub use cost::{Cost, Effect, SentState};
pub use effect::{
    EffectContract, EffectOperation, EffectRecovery, IdempotencyKey, IdempotencyKeyPart,
    OccurrenceLabel, Recorded,
};
pub use error::OperationError;
pub(crate) use managed::UnitScope;
pub use managed::{Attempt, Lease, OPERATION_DEADLINE_CAP, OperationCx, Submission};
pub use pin::PinSlots;
pub use row::ResourceHandle;
pub use session::{
    SessionBinding, SessionClosed, SessionCx, SessionEnd, SessionFuture, SessionProvider,
    SessionSpec,
};
pub use stream::{ConsumerGone, StreamOperation, StreamSink, Streaming};

use crate::resource::Provider;

/// One kind of provider call, described as data and run by the facade.
///
/// The value is the call's owned intent (a message, a query); [`run`](Self::run)
/// asks the [`OperationCx`] for attempts and settles each. Declare the effect of
/// repeating the call with [`EFFECT`](Self::EFFECT) and how many attempts
/// one unit may take with [`max_attempts`](Self::max_attempts).
///
/// ```
/// use nebula_resource::{
///     PinSlots, Provider,
///     call::{Cost, Effect, OperationCx, OperationError, Operation, SentState},
/// };
///
/// /// Reads a counter the instance exposes.
/// struct ReadCounter;
///
/// impl<R> Operation<R> for ReadCounter
/// where
///     R: Provider<Instance = u64> + PinSlots,
/// {
///     type Output = u64;
///     const EFFECT: Effect = Effect::Read;
///
///     async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
///         let attempt = cx.attempt(Cost::ONE).await?;
///         let value = *attempt.instance();
///         attempt.settle(SentState::Sent);
///         Ok(value)
///     }
/// }
/// ```
pub trait Operation<R: Provider + PinSlots>: Send + 'static {
    /// What a successful unit yields.
    type Output: Send + 'static;

    /// What repeating the call does to the provider. `Write` unless the
    /// operation declares otherwise.
    const EFFECT: Effect = Effect::Write;

    /// How many attempts one unit may be granted; one by default.
    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN
    }

    /// Runs the call: every provider request goes through
    /// [`OperationCx::attempt`].
    fn run(
        self,
        cx: &mut OperationCx<'_, R>,
    ) -> impl Future<Output = Result<Self::Output, OperationError>> + Send;
}

#[cfg(test)]
#[path = "../call_tests.rs"]
mod tests;
