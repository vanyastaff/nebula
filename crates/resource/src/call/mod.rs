//! Managed call facade: provider calls made through a registered row as
//! admitted, budgeted, settled units of work.
//!
//! A row becomes a [`ResourceHandle`] facade with
//! [`Manager::handle`](crate::Manager::handle) (or, for an action, the
//! engine's type-erased [`Manager::handle_any`](crate::Manager::handle_any)
//! family). Action code then describes each provider call as an
//! [`Operation`] and [`submit`](ResourceHandle::submit)s it; the facade has
//! no `Deref` to an instance, so a call cannot bypass it by accident. A
//! raw [`ResourceGuard`](crate::ResourceGuard) from
//! [`Manager::acquire`](crate::Manager::acquire) stays a host-only
//! capability and never becomes a facade: holding one instance across
//! several units belongs to a future, qualified explicit-session profile.
//!
//! # Unit and attempt
//!
//! - A **unit** ([`Submission`]) is one submitted operation: owned intent, a
//!   runtime-owned task, a deadline, an attempt budget, the credential slots
//!   pinned at its first grant, and exactly one settled outcome (Design
//!   CONTRACT.md:36-46, DX-API.md:106-114).
//! - An **attempt** ([`Attempt`]) is one admitted provider call.
//!   [`OperationCx::attempt`] is the unit's single linearization point: budget,
//!   the row's admission, quota booking at the attempt's [`Cost`], the row
//!   gate, the strict credential read, the attempt's own checkout,
//!   registration and grant (CONTRACT.md:64, :88-92).
//! - A **call** ([`OperationCx::call`]) is the usual way to use attempts:
//!   a closure makes the provider request on each attempt and returns a
//!   classified result; the runtime finishes the attempt from it and takes
//!   another only when the classification says it is safe, within
//!   [`Operation::max_attempts`] (one by default: no hidden retry) and the
//!   unit's deadline (DX-API.md:36, CONTRACT.md:96). An operation that
//!   holds the attempt itself finishes it with [`Attempt::finish`].
//!
//! A unit is lazy. Its first poll hands it to the runtime, which runs it on
//! its own task until it settles. Dropped before its first poll it never
//! ran; dropped later, the caller only stops waiting — the runtime still
//! settles the unit, and an attempt's checkout is released only after that
//! attempt ends (DX-API.md:114).
//!
//! # Settled outcome
//!
//! Each attempt is finished from its call's result, classified once by the
//! [`OperationError`] constructor that built the error; the runtime derives
//! the rest (see [`OperationError`] for the full table):
//!
//! | Result | Sent | Rate limit | Journal | Retried inside `call` |
//! |---|---|---|---|---|
//! | `Ok` | `Sent` | pass | applied | — |
//! | [`throttled`](OperationError::throttled) / [`throttled_key`](OperationError::throttled_key) | `Sent` (`Exhausted`) | throttled / key throttled | not crossed | yes |
//! | [`unreachable`](OperationError::unreachable) | `NotSent` | nothing | not crossed | yes |
//! | [`interrupted`](OperationError::interrupted) | `MaybeSent` | nothing | ambiguous | only a replay-safe effect |
//! | [`rejected`](OperationError::rejected) / [`rejected_as`](OperationError::rejected_as) | `Sent` | pass | rejected | no |
//! | unclassified ([`OperationError::new`], `?`) | `MaybeSent` | nothing | ambiguous | only replay-safe with a retryable kind |
//!
//! An attempt dropped unfinished counts as `MaybeSent`. The unit folds its
//! attempts:
//!
//! - no attempt granted → `NotSent`, whatever the attempts reported (the
//!   runtime proves nothing was sent);
//! - otherwise the worst attempt, `MaybeSent > Sent > NotSent`, where a
//!   throttled attempt only counts when it was the last (the provider
//!   applied nothing);
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
//! | row closing (removal, replacement, shutdown) | `Cancelled` | `NotSent` |
//! | credential suspension of the row | `CredentialUnavailable` | `NotSent` |
//! | credential revoke tainted the row | `Revoked` | `NotSent` |
//! | strict read refused (blocked, outage, absent, new material) | `CredentialUnavailable` | `NotSent` |
//! | the unit's pinned slots were rotated since its first grant | `CredentialUnavailable { Rebinding }` | `NotSent` |
//! | local quota slot past the deadline | `Exhausted` | `NotSent` |
//! | limit store down, row gate full until the deadline, phase not accepting | `Backpressure` | `NotSent` |
//! | provider throttled ([`OperationError::throttled`]) | `Exhausted` | `Sent` |
//! | retry-unsafe effect with an unknown outcome | `OutcomeUnknown` (as `Error`) | `Sent` / `MaybeSent` |
//!
//! Author errors bridge in through the usual route: a
//! `#[derive(ClassifyError)]` enum converts into [`Error`](crate::Error),
//! and `?` converts that into [`OperationError`], keeping only its kind.
//!
//! # Closing
//!
//! [`OperationCx::closing`] is the [`LeaseClosing`](crate::LeaseClosing) of
//! the row generation the unit started under. Once it fires, new attempts
//! are refused; an attempt already granted is not aborted — an operation
//! that wants to stop early selects on
//! [`closed`](crate::LeaseClosing::closed) (CONTRACT.md:74). A credential
//! refresh or a config reload leaves that generation open, so neither
//! interrupts a unit (canon §13.2).
//!
//! # Credentials
//!
//! On a manager with a credential availability observer (a strict manager),
//! every attempt on a credential-bound row reads each bound credential's
//! availability after every wait of the attempt, outside every lock, and is
//! registered under the manager's admission lock: a blocked credential
//! suspends the row and closes its generation, a store outage refuses without
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
//! Building a [`ResourceHandle`] latches the row's profile to
//! [`RateLimitProfile::PerAttempt`](crate::RateLimitProfile::PerAttempt):
//! each granted attempt books its [`Cost`], and acquires only honour pauses
//! (QUOTA-DX.md:32, :34). A quota wait is raced against the unit's
//! generation, the manager's shutdown and [`Submission::cancel`], so a
//! closed row never sends (CONTRACT.md:88-92).
//!
//! # Per-unit checkout and sessions
//!
//! A [`ResourceHandle`] holds no lease: every attempt books its
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
//! One [`submit`](ResourceHandle::submit) (and one
//! [`session`](ResourceHandle::session)) serves every caller; the runtime
//! routes each unit by its [`Effect`] and the authority of the caller that
//! built the row:
//!
//! | Authority | `Read` | `Idempotent` / `Write` | streamed `Idempotent` / `Write` |
//! |---|---|---|---|
//! | unjournaled (library row) | runs | runs | runs |
//! | read-only ([`Manager::handle_any_read_only`](crate::Manager::handle_any_read_only)) | runs | refused `Permanent` / `NotSent` | refused |
//! | journaled ([`Manager::handle_any_journaled`](crate::Manager::handle_any_journaled)) | runs, never prepared | through the [`EffectJournal`](journal::EffectJournal) | refused ("streaming effects are not journaled in v1") |
//!
//! The resource runtime never writes durable effect state; for a journaled
//! effect it derives the journal declaration from the [`Operation`] (or the
//! [`SessionSpec`]) and drives the journal seam ([`journal`]) at fixed
//! points of the unit:
//!
//! 1. **Submit** — the declaration is checked (key, version, key window,
//!    developer key part), the canonical request computed (the operation's
//!    JSON with sorted keys, 1 byte to 1 MiB), the recovery derived
//!    (`Idempotent`: a stable key within [`Operation::KEY_WINDOW`]; `Write`:
//!    opaque), and an in-flight ticket is taken. A closed owner refuses
//!    `Cancelled`. The submission is lazy: dropped before its first poll,
//!    it never reaches the owner and takes no position.
//! 2. **First poll** — the occurrence label
//!    `unit/v1/#{ordinal:06}` is fixed from the unit's positional ordinal
//!    in the owner's one sequence for all its effect units, in the order
//!    they start preparing. The resource, kind, operation key and version
//!    are bound by the contract, not the label: changing them under a
//!    recorded occurrence is a mismatch, never a fresh effect; units polled
//!    in another order than an earlier run — across resources and between
//!    operations and sessions — meet each other's positions and fail the
//!    same way unless their intents are identical. The owner then prepares the effect from the canonical
//!    request and the optional developer key part before anything is
//!    booked, read or checked out: a recorded success replays its output
//!    without a provider call; a recorded rejection, a digest-only success
//!    ([`Operation::RECORD_OUTPUT`] off, or an output over 1 MiB) and an
//!    unknown outcome fail without one.
//! 3. **Each attempt** — the previous attempt's call is explained when the
//!    attempt starts; after the checkout and the credential reads, right
//!    before the registration, the owner grants the attempt's call (the
//!    only provider-call authority); a registration that refuses after the
//!    grant is explained as not crossed.
//! 4. **Settle** — the unit's last call is recorded from its result and
//!    the last attempt's classification: a success with its output, a
//!    [`rejected`](OperationError::rejected) call with its kind
//!    ([`ErrorKindCode`](journal::ErrorKindCode)), otherwise how the call
//!    crossed (unreachable or throttled: not crossed; interrupted or
//!    unclassified: ambiguous). A record that fails after a possible
//!    crossing fails the unit `OutcomeUnknown`. A unit that fails with
//!    nothing crossed also tells the owner how it failed
//!    ([`UnsentFailure`](journal::UnsentFailure)) before the failure is
//!    returned, so a later
//!    run that must not send the effect again fails it the same way.
//!
//! [`OperationCx::idempotency_key`] and [`SessionCx::idempotency_key`] are the provider
//! idempotency key ([`IdempotencyKey`]) to send: for a journaled effect the
//! key the owner recorded before the first attempt, otherwise — for a unit
//! that declared a developer key part — a key derived locally from the
//! resource, the operation key and version, and the part (base64url
//! SHA-256). Either is the same for every attempt, retry and resume. A unit
//! cancelled before its first grant leaves only its prepare behind, which a
//! resumed unit runs again. Interim: the engine does not hand out owned rows yet; its owner over the
//! operation ledger comes with the engine wiring.
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
//! - A cost booked for an attempt that is cancelled before it reaches the
//!   provider is not refunded (QUOTA-DX.md:32 baseline).
//! - A [`ResourceHandle`] checks out per attempt, after the attempt's quota
//!   and row-gate waits, so a unit waiting for quota holds no connection
//!   (QUOTA-DX.md:41, :43). Holding one instance across several units is
//!   out of scope until a qualified explicit-session profile exists.
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
//! A [`StreamOperation`] submitted with
//! [`ResourceHandle::submit_streaming`] runs as one ordinary unit that also
//! sends items through a bounded [`StreamSink`]; the caller pulls them from
//! [`Streaming`], then the unit's error, if any, once. A mid-stream failure
//! is never an item, a dropped or cancelled consumer ends the operation at
//! its next send, and the row closing is honoured by selecting on
//! [`OperationCx::closing`] (CONTRACT.md:57). Each attempt still checks
//! out per attempt after its quota and gate waits; items sent while an
//! attempt is alive keep its checkout, and a consumer gone mid-stream
//! releases it with its gate permit. [`StreamOperation`] documents the
//! delivery rules.
//!
//! # Observability
//!
//! Each unit runs in a `nebula.resource.unit` span recording its resource
//! key, its operation key ([`Operation::KEY`], a session's name — bounded
//! cardinality), a journaled effect's occurrence, attempts granted, sent
//! state and outcome.
//! [`ResourceOpsMetrics`](crate::ResourceOpsMetrics) counts attempts granted
//! and refused by the facade — separate from a driver's own retries inside
//! an attempt (CONTRACT.md:134) — units settled by sent state, row
//! checkouts by whether they created their instance, and sessions by how
//! they ended. A unit
//! that ends with an unknown outcome publishes
//! [`ResourceEvent::OperationOutcomeUnknown`](crate::ResourceEvent::OperationOutcomeUnknown).

pub(crate) mod canonical;
mod cost;
mod declaration;
mod error;
pub mod journal;
mod managed;
mod owned;
mod pin;
mod row;
mod session;
mod stream;
mod strict;
mod work;

use std::{future::Future, num::NonZeroU32, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

pub use cost::{Cost, Effect, SentState};
pub use declaration::IdempotencyKey;
pub use error::OperationError;
pub(crate) use managed::UnitScope;
pub use managed::{Attempt, OPERATION_DEADLINE_CAP, OperationCx, Submission};
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
/// makes the provider call through [`OperationCx::call`], classifying its
/// result with an [`OperationError`] constructor. Declare the effect of
/// repeating the call with [`EFFECT`](Self::EFFECT) and how many attempts
/// one unit may take with [`max_attempts`](Self::max_attempts).
///
/// # What an execution journal records
///
/// An operation carries only what a journal needs, and the runtime derives
/// the rest. On a row an action got with effect-owner authority, an
/// `Idempotent` or `Write` unit is recorded under the contract
/// `(KEY, VERSION)`, from the key-sorted JSON of the operation value (its
/// canonical request: logical intent only — no credentials, signatures or
/// timestamps), with the developer key part of
/// [`idempotency_key`](Self::idempotency_key), and replayed from its
/// serialized output on resume. Hence the serde bounds on the operation
/// and its [`Output`](Self::Output); fields that are not intent (a probe, a
/// channel) are `#[serde(skip)]`.
///
/// - [`KEY`](Self::KEY): unique within the resource; 1 to 64 bytes of
///   `[A-Za-z0-9_.-]`, starting and ending alphanumeric.
/// - [`VERSION`](Self::VERSION): the interface major; bump it when the
///   request or output shape changes meaning. At least 1.
/// - [`KEY_WINDOW`](Self::KEY_WINDOW): for an `Idempotent` operation, how
///   long the provider deduplicates a key; non-zero.
/// - [`RECORD_OUTPUT`](Self::RECORD_OUTPUT): `false` records a success
///   digest-only, so a resume fails `Permanent` instead of replaying. An
///   output over 1 MiB is recorded digest-only too.
///
/// A declaration that breaks these rules fails the build when the
/// operation is submitted (a post-monomorphization error, reported by
/// `cargo build` but not `cargo check`), and is refused `Permanent` /
/// `NotSent` at submit as well.
///
/// ```
/// use nebula_resource::{
///     PinSlots, Provider,
///     call::{Cost, Effect, OperationCx, OperationError, Operation},
/// };
/// use serde::{Deserialize, Serialize};
///
/// /// Reads a counter the instance exposes.
/// #[derive(Serialize, Deserialize)]
/// struct ReadCounter;
///
/// impl<R> Operation<R> for ReadCounter
/// where
///     R: Provider<Instance = u64> + PinSlots,
/// {
///     type Output = u64;
///     const KEY: &'static str = "counter.read";
///     const EFFECT: Effect = Effect::Read;
///
///     async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
///         cx.call(Cost::ONE, async |counter, _credentials| Ok(*counter)).await
///     }
/// }
///
/// /// Charges an order once: a repeat with the same key is absorbed.
/// #[derive(Serialize, Deserialize)]
/// struct Charge {
///     order: u64,
///     cents: u64,
/// }
///
/// impl<R: Provider + PinSlots> Operation<R> for Charge {
///     type Output = u64;
///     const KEY: &'static str = "billing.charge";
///     const EFFECT: Effect = Effect::Idempotent;
///
///     fn idempotency_key(&self) -> Option<String> {
///         Some(format!("order-{}", self.order))
///     }
///
///     async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
///         // The key to send: the same for every attempt, retry and resume.
///         // Read it before the call borrows `cx`.
///         let key = cx.idempotency_key().copied();
///         let cents = self.cents;
///         cx.call(Cost::ONE, async move |_client, _credentials| {
///             // Send the charge with `key`, classify the answer.
///             let _ = key;
///             Ok(cents)
///         })
///         .await
///     }
/// }
/// ```
pub trait Operation<R: Provider + PinSlots>: Serialize + DeserializeOwned + Send + 'static {
    /// What a successful unit yields; recorded and replayed as JSON.
    type Output: Serialize + DeserializeOwned + Send + 'static;

    /// The operation's key, unique within the resource: 1 to 64 bytes of
    /// `[A-Za-z0-9_.-]`, starting and ending alphanumeric.
    const KEY: &'static str;

    /// The operation's interface version, the journal contract with
    /// [`KEY`](Self::KEY): a resumed effect of another version is a
    /// different effect. At least 1.
    const VERSION: u32 = 1;

    /// What repeating the call does to the provider. `Write` unless the
    /// operation declares otherwise.
    const EFFECT: Effect = Effect::Write;

    /// How long the provider deduplicates an idempotency key: within it, an
    /// ambiguous attempt of an `Idempotent` unit may be sent again with the
    /// same key. Only read for `Idempotent`; non-zero. 24 hours by default.
    const KEY_WINDOW: Duration = Duration::from_hours(24);

    /// Whether a journal records a success with its output (replayed on
    /// resume) or digest-only. `true` by default.
    const RECORD_OUTPUT: bool = true;

    /// The developer part of the provider idempotency key — `order-123` —
    /// built deterministically from the operation's input, never random
    /// and never a retry number: 1 to 256 bytes of visible ASCII. With a
    /// part the key is shared across executions (make it specific enough);
    /// without one a journal scopes it to the execution, node and
    /// occurrence. `None` by default.
    fn idempotency_key(&self) -> Option<String> {
        None
    }

    /// How many attempts one unit may be granted; one by default.
    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN
    }

    /// Runs the call: every provider request goes through
    /// [`OperationCx::call`] (or an [`OperationCx::attempt`] finished with
    /// [`Attempt::finish`]).
    fn run(
        self,
        cx: &mut OperationCx<'_, R>,
    ) -> impl Future<Output = Result<Self::Output, OperationError>> + Send;
}

#[cfg(test)]
#[path = "../call_tests.rs"]
mod tests;
