//! The effect journal of one node attempt of a journaled action.
//!
//! An action whose admitted contract is
//! [`Journaled`](nebula_action::effect::ActionEffectContract::Journaled)
//! submits its effects as units on resource handles
//! ([`ResourceHandle`](nebula_resource::call::ResourceHandle)). For a
//! stateless or stateful action on a durable turn the engine builds one
//! [`NodeEffectJournal`] per node attempt and hands it to the node's handles
//! ([`Manager::handle_any_journaled`](nebula_resource::Manager::handle_any_journaled));
//! the resource runtime drives every `Idempotent` or `Write` unit through
//! it (see [`nebula_resource::call::journal`]). The journal records each
//! unit as one slot of the operation ledger, through the same [`LedgerSlot`]
//! core as the remote-effect driver, under the turn's execution lease:
//!
//! | Unit step | Ledger command |
//! |---|---|
//! | prepare | `prepare` (natural key `(scope, execution, node, occurrence)`) |
//! | grant | `GrantInvocation` |
//! | explain | `RecordDisposition` (`BeforeBoundary` / `Ambiguous`) |
//! | settle | `RecordOutcome` (exact recommit on a lost acknowledgement) |
//!
//! **Lazy writes, few reads.** Building a journal costs nothing durable: no
//! ledger write happens until the first `Idempotent` or `Write` unit is
//! prepared, and a `Read` unit is never prepared. Concluding always reads
//! the node's occurrences once — even for a node that only read: a process
//! that died during an earlier dispatch of the node, before that attempt
//! was recorded, leaves the next attempt at the same generation, and only
//! the ledger knows the call it may have made. A node that prepares an
//! effect also reads them once before its first prepare (see
//! **Positions**).
//!
//! **Occurrences.** A unit's occurrence is the label the journal hands the
//! resource runtime ([`EffectJournal::next_occurrence`]),
//! `unit/v1/#{ordinal:06}` for a stateless action: positional, with one
//! sequence for all the node attempt's effect units — every resource,
//! operations and sessions — restarting at zero in every journal, in the
//! order units start preparing (a submission dropped unpolled takes none).
//! A stateful action's units are labelled per iteration,
//! `it{n}/unit/v1/#{ordinal:06}` (`n` in decimal without leading zeros, at
//! most 9999), the ordinal restarting at zero in every iteration. The
//! resource, unit kind, operation (or session) name and version are not
//! part of it but of the slot's contract identity, so a redeploy that
//! changes the effect at a recorded position is an occurrence mismatch
//! with nothing sent — never a fresh slot that sends the effect again
//! under another provider key.
//!
//! **Positions.** A run whose program takes another path than an earlier
//! attempt meets that attempt's slots at other positions, and every such
//! case fails safe:
//!
//! - units prepared in another order (concurrent units polled differently,
//!   effects of different resources reordered, a session before an
//!   operation) meet each other's slots: a different intent is a mismatch,
//!   an identical one is interchangeable;
//! - an effect added or removed before recorded ones moves the later ones
//!   onto recorded positions of other intents: a mismatch;
//! - a position an earlier attempt left empty below one it recorded (a
//!   unit whose prepare never became durable, then later effects) would let
//!   a later effect land on it fresh — under a new provider key while its
//!   settled slot stays further on. Before its first prepare the journal
//!   reads what earlier attempts recorded, and refuses a fresh slot at such
//!   a gap as a mismatch, with nothing written or sent. Positions order by
//!   `(iteration, ordinal)` (a flat label is iteration 0): an unrecorded
//!   position is a gap when a higher one of the same family — a later
//!   ordinal of its iteration, or any slot of a later iteration — is
//!   recorded;
//! - a recorded position this attempt passed by — another path that skips
//!   it, an iteration that ends without it, or a unit that took it and gave
//!   up before reaching the journal (past its deadline, cancelled, dropped)
//!   — would let a later effect land fresh above it, under a new provider
//!   key, although it may be the very effect recorded there. A fresh slot
//!   is prepared only when every recorded position of its family below it
//!   was met by this attempt: the resource runtime releases every position
//!   it hands out ([`EffectJournal::release_occurrence`]), and a fresh slot
//!   above a recorded position another unit is still preparing waits for
//!   that prepare to settle. Otherwise it is refused as a mismatch, with
//!   nothing written or sent;
//! - labels of the other family (flat for a stateful node, `it{n}/` for a
//!   stateless one) recorded by an earlier attempt mean the node's action
//!   changed kind: a fresh slot is refused as a mismatch.
//!
//! An engine retry of the node therefore reuses
//! the occurrences of its earlier attempts: a settled slot replays its
//! recorded outcome without a provider call, an opaque ambiguous one is
//! unknown, and a retryable failure — never recorded as a rejection — may
//! be granted again within the slot's budget.
//!
//! **Kinds** ([`JournalShape`]). Stateless actions are journaled with one
//! flat occurrence sequence per node attempt; stateful actions per
//! iteration. A control action decides flow and must not cause effects;
//! agent, stream and other actions keep read-only handles. Each says why in
//! the refusal of a write.
//!
//! **Iterations.** A stateful action runs all its iterations inside one
//! node attempt, and every attempt replays them from iteration 0 (no
//! iteration checkpoint is kept for a journaled action: the runtime refuses
//! a checkpoint sink on that path). The runtime brackets each iteration
//! with the journal's barrier ([`IterationGate`]):
//! [`begin_iteration`](NodeEffectJournal::begin_iteration) requires no unit
//! in flight and starts the iteration's label namespace;
//! [`end_iteration`](NodeEffectJournal::end_iteration) — after the
//! iteration returned, successfully or not — drains its units within the
//! node's drain limit and stops the loop when the journal holds a failure
//! (an unknown outcome, a mismatch, a deferring ledger or lease failure),
//! or when an iteration that returned `Ok` passed by an effect an earlier
//! attempt recorded in it or before it (a mismatch): no later iteration
//! sends anything past it. A unit still in flight past the drain limit
//! fails the barrier: the journal closes, and the node's one verdict
//! records a call the unit was granted as ambiguous (the node then fails
//! unknown; with no call granted the barrier failure is the verdict).
//! Replaying requires a deterministic action — an iteration fed inputs a
//! replay does not reproduce (clocks, randomness, unrecorded reads)
//! diverges, and the divergence halts the node as an occurrence mismatch
//! before any recorded effect is sent again.
//!
//! **Order.** Within a family, an effect at a lower position is never
//! applied after one at a higher position:
//!
//! - in one attempt, a unit whose ledger prepare began and never answered
//!   (cancelled or past its deadline mid-call, or the acknowledgement lost)
//!   leaves its position *uncertain*: its row may exist. No fresh slot above
//!   an uncertain position is prepared in that attempt — the prepare is
//!   refused as a deferring
//!   [`AcknowledgementUnknown`](OperationLedgerError::AcknowledgementUnknown),
//!   nothing is written or sent, and the node defers, so the next attempt
//!   reads what was written and replays in order;
//! - across attempts, a recorded slot `L` that changed nothing yet (only
//!   prepared, or every call explained not crossed) is refused as an
//!   occurrence mismatch, with nothing sent, when an earlier attempt
//!   recorded a slot `H` of its family that may have changed the provider
//!   (an outcome, or a call that crossed) and that the program ran *after*
//!   `L`: `H` is of a later iteration (the barrier drains one iteration
//!   before the next begins), or `L` lies below `H`'s **concurrency
//!   floor**. Every fresh slot records its floor with its first prepare
//!   ([`EffectSlotBinding::concurrent_floor`]): the lowest position of its
//!   iteration whose unit was still open — handed out and not yet gone
//!   ([`EffectJournal::finish_occurrence`]) — its own when none was. A unit
//!   below the floor was gone before `H` began; one at or above it ran
//!   concurrently with `H` (units awaited together), the program did not
//!   order them, and `L` replays under its recorded provider key — at
//!   least once, as a durable-execution engine re-sends an unsettled
//!   scheduled effect. A slot recorded without a floor (before floors
//!   existed) is read strictly: as if every unit below it was gone.
//!
//! **Cancellation.** A node cancelled mid-iteration ends the iteration at
//! once ([`IterationGate::cancel_iteration`]): a later submission (a
//! detached task's) is refused closed with no failure of its own, so the
//! conclusion drains only the units already in flight.
//!
//! **Admission.** A stateful node's journal admits a submitted unit only
//! while an iteration is open
//! ([`EffectJournal::admit`]): admission raises the in-flight count under
//! the same lock as the rollover checks it, so no unit slips into an
//! iteration it was not admitted in, and a unit a detached task submits
//! between one iteration's end and the next one's begin is refused
//! [`BetweenRuns`](JournalRefusal::BetweenRuns) unsent while the node's
//! verdict records
//! [`IterationUnitsOutstanding`](EffectExecutionError::IterationUnitsOutstanding).
//!
//! **Barrier read.** The first barrier reads what earlier attempts
//! recorded, if no prepare did, within what is left of the drain limit (at
//! least the verdict read's floor); a ledger that does not answer by then
//! defers the node like an unavailable ledger.
//!
//! **Cap.** One node attempt prepares at most [`MAX_NODE_SLOTS`] fresh
//! journaled effects (a stateful action at its iteration cap with one
//! effect per iteration fits); replays of recorded positions are not
//! counted. A further fresh prepare is refused
//! [`SlotCapExceeded`](JournalRefusal::SlotCapExceeded), nothing is sent,
//! and the node fails
//! [`JournalSlotCapExceeded`](EffectExecutionError::JournalSlotCapExceeded).
//!
//! **Provider key.** Every slot records the idempotency key the provider
//! receives ([`provider_idempotency_key`]); a unit always presents the key
//! read back from the prepared record, never a recomputation.
//!
//! **Crash residue.** A call granted and never explained (the process
//! died, or the unit outlived its node) is an outstanding invocation. The
//! journal never writes from `Drop`: the next prepare of the slot records
//! it as an ambiguous crossing first — an opaque effect's outcome becomes
//! unknown, a stable-key effect may be granted again within its window.
//!
//! **Grant budget.** A grant carries what is left of the ledger's window
//! for the call ([`LedgerSlot::call_timing`], as for a remote effect): the
//! resource runtime stops the unit there, so no call starts after a stable
//! key's deduplication window. A grant with nothing left is withheld: the
//! outcome becomes unknown when an earlier call of the slot may have
//! crossed, and the call is recorded not crossed otherwise.
//!
//! **Destination.** A slot's contract identity binds the row (resource key,
//! credential slot identity and configuration fingerprint) and how its
//! success is recorded: a reload that points the row elsewhere, or a
//! changed `RECORD_OUTPUT`, makes a recorded occurrence a mismatch, and
//! nothing is sent.
//!
//! **Verdict.** The journal never lets a node finish on a result its ledger
//! contradicts: on every exit of the node — after the action returns, and
//! before it runs when the node is cancelled or its input, credential
//! refresh or rate limit fails —
//! [`conclude_node`](NodeEffectJournal::conclude_node) drains the in-flight units,
//! closes the journal, records every granted-but-unexplained call as
//! ambiguous (within the drain limit) and reports a verdict that overrides
//! the node's result. Any slot whose call may have crossed without a
//! recorded outcome — this attempt's or an earlier dispatch's — fails the
//! node as unknown. A node about to succeed although an earlier attempt
//! recorded an effect (settled, or a call that crossed) this attempt never
//! prepared took another path past an applied mutation: it fails as an
//! occurrence mismatch. No error strategy recovers or routes past either
//! verdict ([`EffectExecutionError::halts_execution`]): the execution
//! stops. A node *failing* past such an effect keeps its own failure
//! ([`Concluded::SkippedRecordedEffect`]): the node's retry policy may take
//! it up — the retry meets the effect and replays it — but a final failure
//! halts the execution the same way instead of being ignored or routed.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    num::NonZeroU32,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use base64::Engine as _;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL, NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
        NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL, effect_journal_prepare_phase, effect_journal_step,
        effect_journal_verdict,
    },
};
use nebula_resource::{
    SlotIdentity,
    call::{
        Effect, IdempotencyKey, OPERATION_DEADLINE_CAP,
        journal::{
            CallGrant, CallOutcome, Crossing, EffectJournal, ErrorKindCode, InFlight,
            JournalIntent, JournalRefusal, JournalSlot, RecordedOutcome, Recovery, SlotPhase,
        },
    },
};
use nebula_storage_port::dto::{
    EffectOccurrenceRecord, EffectSlotId, KnownOutcome, OperationState, ProviderIdempotencyKey,
};
use serde::{Deserialize, Serialize};

use super::*;

/// Domain of a journaled effect's contract identity digest.
const CONTRACT_DOMAIN: &[u8] = b"nebula.effect-journal.contract.v1";
/// Domain of a journaled effect's request fingerprint digest.
const REQUEST_DOMAIN: &[u8] = b"nebula.effect-journal.request.v1";
/// Domain of the provider idempotency key digest.
const IDEMPOTENCY_KEY_DOMAIN: &[u8] = b"nebula.idempotency-key.v1";

/// Longest window the ledger accepts: one year.
const MAX_LEDGER_WINDOW: Duration = Duration::from_hours(365 * 24);

/// The least time the verdict's final occurrence read gets, even when the
/// drain spent the whole limit.
const FINAL_READ_FLOOR: Duration = Duration::from_secs(5);

/// When an occurrence read that shares a budget of `limit` from `started`
/// must have answered: what is left of the budget, and at least
/// [`FINAL_READ_FLOOR`] from now.
fn read_deadline(started: tokio::time::Instant, limit: Duration) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    let budget_end = started.checked_add(limit).unwrap_or(now);
    budget_end.max(now.checked_add(FINAL_READ_FLOOR).unwrap_or(budget_end))
}

/// Most provider calls one slot may be granted.
const MAX_SLOT_INVOCATIONS: u32 = 10_000;

/// Most fresh journaled effects — at no position an earlier attempt
/// recorded — one node attempt may prepare.
///
/// Bounds the new ledger rows a node attempt writes. It matches the
/// stateful runtime's iteration cap: a stateful action that prepares one
/// effect per iteration fits at its last iteration. Replays of recorded
/// positions are not counted, so a node whose ledger already holds more
/// stays replayable. A further fresh prepare is refused, nothing is sent,
/// and the node fails
/// [`JournalSlotCapExceeded`](EffectExecutionError::JournalSlotCapExceeded).
pub(crate) const MAX_NODE_SLOTS: u32 = 10_000;

/// Highest stateful iteration the journal labels (`it9999/`): the stateful
/// runtime runs iterations `0..10_000`.
const MAX_ITERATION: u32 = 9_999;

/// The positional part of every occurrence label, before its ordinal.
const UNIT_POSITION: &str = "unit/v1/#";

/// How long a journaled node waits for its units — after its action
/// returned, or at a stateful iteration's barrier: the unit deadline cap,
/// or less when the execution's wall-clock budget (`execution_deadline`)
/// ends sooner.
pub(crate) fn journal_drain_limit(execution_deadline: Option<Instant>) -> Duration {
    execution_deadline.map_or(OPERATION_DEADLINE_CAP, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(OPERATION_DEADLINE_CAP)
    })
}

/// The barrier the stateful runtime keeps around each iteration of a
/// journaled stateful action
/// ([`execute_stateful_handle`](crate::runtime::ActionRuntime)).
///
/// Implemented over the node attempt's [`NodeEffectJournal`]
/// ([`JournalIterationGate`]); object safe so the runtime's tests can drive
/// the loop with a scripted gate.
#[async_trait::async_trait]
pub(crate) trait IterationGate: Send + Sync {
    /// Opens `iteration`: its effect units are labelled `it{iteration}/`.
    ///
    /// # Errors
    ///
    /// The failure the journal already holds, or
    /// [`IterationUnitsOutstanding`](EffectExecutionError::IterationUnitsOutstanding)
    /// when a unit is still in flight. The runtime does not run the
    /// iteration.
    fn begin_iteration(&self, iteration: u32) -> Result<(), EffectExecutionError>;

    /// Closes the open iteration after its dispatch returned — successfully
    /// (`succeeded`) or not — once its units drained.
    ///
    /// # Errors
    ///
    /// The failure the journal holds (an unknown outcome, a mismatch, a
    /// deferring ledger or lease failure), an occurrence mismatch when a
    /// succeeding iteration passed an earlier attempt's recorded effect by,
    /// or
    /// [`IterationUnitsOutstanding`](EffectExecutionError::IterationUnitsOutstanding)
    /// when a unit outlived the drain limit. The runtime starts no further
    /// iteration.
    async fn end_iteration(&self, succeeded: bool) -> Result<(), EffectExecutionError>;

    /// Ends the open iteration because the node was cancelled while it
    /// ran: no unit is admitted afterwards (a later submission is refused
    /// [`Closed`](JournalRefusal::Closed), `Cancelled`, and recorded as no
    /// failure). Units already in flight are left to the node's conclusion,
    /// which drains them; nothing is waited for here.
    fn cancel_iteration(&self);
}

/// The [`IterationGate`] of a journaled stateful node attempt: its journal,
/// drained at each barrier within the node's drain limit.
pub(crate) struct JournalIterationGate {
    journal: NodeEffectJournal,
    execution_deadline: Option<Instant>,
}

#[async_trait::async_trait]
impl IterationGate for JournalIterationGate {
    fn begin_iteration(&self, iteration: u32) -> Result<(), EffectExecutionError> {
        self.journal.begin_iteration(iteration)
    }

    async fn end_iteration(&self, succeeded: bool) -> Result<(), EffectExecutionError> {
        self.journal
            .end_iteration(journal_drain_limit(self.execution_deadline), succeeded)
            .await
    }

    fn cancel_iteration(&self) {
        self.journal.cancel_iteration();
    }
}

impl fmt::Debug for JournalIterationGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalIterationGate")
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

/// Engine-private proof that a dispatch runs under a [`NodeEffectJournal`]:
/// only this module can mint it, so generic dispatch cannot run a journaled
/// action with write authority. It carries the journal's iteration barrier,
/// which a stateful action's loop keeps.
pub(crate) struct JournalAdmission {
    gate: JournalIterationGate,
}

impl JournalAdmission {
    /// The iteration barrier of the admitted node attempt.
    pub(crate) fn iteration_gate(&self) -> &dyn IterationGate {
        &self.gate
    }
}

/// Why a journaled control action has read-only resource handles.
const CONTROL_NOT_JOURNALED: &str =
    "control actions decide flow and must not cause effects; move effects to a stateless action";
/// Why a journaled agent action has read-only resource handles.
const AGENT_NOT_JOURNALED: &str = "agent effects are not journaled; the agent profile is planned";
/// Why a journaled action of any other kind has read-only resource handles.
const KIND_NOT_JOURNALED: &str = "effects of this action kind are not journaled";

/// How the node effect journal records a journaled action's effects, by
/// the action's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JournalShape {
    /// One run per node attempt: one flat occurrence sequence
    /// (`unit/v1/#n`). Stateless actions.
    Flat,
    /// One run per iteration: a stateful action, whose occurrences carry
    /// the iteration (`it{n}/unit/v1/#n`), its runtime loop keeping the
    /// journal's [`IterationGate`].
    Iterated,
    /// No journal: a control action (which decides flow and must not cause
    /// effects), an agent action (whose profile is planned), a stream
    /// action, and every other kind keep read-only handles.
    None,
}

impl JournalShape {
    /// The shape of `kind`'s effects.
    pub(crate) const fn of(kind: nebula_action::ActionKind) -> Self {
        match kind {
            nebula_action::ActionKind::Stateless => Self::Flat,
            nebula_action::ActionKind::Stateful => Self::Iterated,
            _ => Self::None,
        }
    }

    /// Whether a journaled action of this shape runs under a node effect
    /// journal in this version.
    pub(crate) const fn is_journaled(self) -> bool {
        matches!(self, Self::Flat | Self::Iterated)
    }

    /// Why a journaled action of `kind` has read-only handles although its
    /// turn has execution stores, or `None` when its kind is journaled.
    pub(crate) const fn read_only_detail(kind: nebula_action::ActionKind) -> Option<&'static str> {
        match (Self::of(kind), kind) {
            (Self::Flat | Self::Iterated, _) => None,
            (Self::None, nebula_action::ActionKind::Control) => Some(CONTROL_NOT_JOURNALED),
            (Self::None, nebula_action::ActionKind::Agent) => Some(AGENT_NOT_JOURNALED),
            (Self::None, _) => Some(KIND_NOT_JOURNALED),
        }
    }
}

/// How a node attempt's journal concluded without a verdict of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Concluded {
    /// Nothing contradicts the node's result.
    Clean,
    /// The node is failing, and an earlier attempt recorded an effect —
    /// settled, or a call that crossed — this attempt never met again. Its
    /// failure may be retried (the retry meets the effect and replays it),
    /// but never recovered or routed past by an error strategy.
    SkippedRecordedEffect,
}

/// Everything a journal is built from: the authority of the node attempt
/// that runs the action.
pub(crate) struct JournalAuthority {
    pub ledger: Arc<dyn OperationLedger>,
    pub scope: Scope,
    pub fencing: FencingToken,
    pub execution_id: ExecutionId,
    pub node_key: NodeKey,
    pub action_key: String,
    pub action_version: semver::Version,
    pub attempt_generation: u64,
    pub clock: Arc<dyn Clock>,
    pub metrics: MetricsRegistry,
    /// How the action's effects are journaled: an
    /// [`Iterated`](JournalShape::Iterated) journal admits units only while
    /// an iteration is open.
    pub shape: JournalShape,
}

/// The per-node-attempt journal of a journaled action's effects: the
/// [`EffectJournal`] the engine hands to the action's resource handles.
///
/// Cheap to clone; every clone is the same journal.
#[derive(Clone)]
pub(crate) struct NodeEffectJournal {
    inner: Arc<JournalInner>,
}

struct JournalInner {
    authority: JournalAuthority,
    /// The execution id as the ledger addresses it.
    execution: String,
    /// The node's occurrences as earlier attempts left them, read once
    /// before this attempt's first prepare.
    prior: tokio::sync::OnceCell<PriorOccurrences>,
    state: Mutex<JournalState>,
    /// Units admitted and not gone. Raised only under the `state` lock, in
    /// the same transition that checks the open iteration, so an iteration
    /// rolls over only with none in flight.
    in_flight: AtomicUsize,
    /// Most fresh slots the node attempt may prepare.
    slot_cap: u32,
    drained: tokio::sync::Notify,
    closed: AtomicBool,
    /// An iteration barrier already waited the drain limit out: the
    /// verdict does not wait for the leaked units again.
    barrier_failed: AtomicBool,
    /// Woken whenever a unit's claim on its position settles.
    claims_settled: tokio::sync::Notify,
}

#[derive(Default)]
struct JournalState {
    /// The next position of the node attempt's sequence of effect units
    /// (of the open iteration, for a stateful action).
    next_ordinal: u32,
    /// The last iteration a stateful action began; `None` for a stateless
    /// one (flat labels).
    iteration: Option<u32>,
    /// Whether that iteration is still open: between its
    /// [`end_iteration`](NodeEffectJournal::end_iteration) and the next
    /// [`begin_iteration`](NodeEffectJournal::begin_iteration) an iterated
    /// journal admits no unit.
    run_open: bool,
    /// The node was cancelled mid-iteration: every later submission is
    /// refused as closed, with no failure of its own.
    admission_cancelled: bool,
    /// The slots prepared in the open iteration, inspected at its end.
    iteration_slots: Vec<EffectSlotId>,
    /// Fresh slots (at no recorded position) the node attempt started to
    /// prepare, bounded by the slot cap ([`MAX_NODE_SLOTS`]). Replays of
    /// recorded positions are not counted.
    reserved: u32,
    /// The positions this attempt met, or is preparing.
    positions: MetPositions,
    /// The slots this journal prepared. A slot is used by one unit at a
    /// time; its async lock serializes that unit's ledger steps with the
    /// journal's conclusion. The sync lock around the map is never held
    /// across an await.
    slots: HashMap<EffectSlotId, Arc<tokio::sync::Mutex<LedgerSlot>>>,
    /// The failure that decides the node's verdict (a deferring one
    /// replaces a non-deferring one).
    failure: Option<EffectExecutionError>,
}

/// The positional family of an occurrence label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    /// `unit/v1/#{ordinal:06}`: a stateless action's one sequence.
    Flat,
    /// `it{n}/unit/v1/#{ordinal:06}`: a stateful action's iterations.
    Iterated,
}

/// Where an occurrence label sits: its family and `(iteration, ordinal)`
/// (a flat label is iteration 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Position {
    family: Family,
    iteration: u32,
    ordinal: u32,
}

impl Position {
    /// The position of a label the journal builds, read strictly: the
    /// iteration in decimal without leading zeros (at most
    /// [`MAX_ITERATION`]), the ordinal zero-padded to six digits. Any other
    /// label has no position.
    fn parse(label: &str) -> Option<Self> {
        let (family, iteration, rest) = match label.strip_prefix("it") {
            Some(tail) => {
                let (iteration, rest) = tail.split_once('/')?;
                let iteration = canonical_number(iteration, 1)?;
                if iteration > MAX_ITERATION {
                    return None;
                }
                (Family::Iterated, iteration, rest)
            },
            None => (Family::Flat, 0, label),
        };
        let ordinal = canonical_number(rest.strip_prefix(UNIT_POSITION)?, 6)?;
        Some(Self {
            family,
            iteration,
            ordinal,
        })
    }

    const fn order(self) -> (u32, u32) {
        (self.iteration, self.ordinal)
    }
}

/// `digits` as a number when they are exactly its rendering zero-padded to
/// `width`: no sign, no extra leading zero.
fn canonical_number(digits: &str, width: usize) -> Option<u32> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    (format!("{number:0width$}") == digits).then_some(number)
}

/// The label of position `ordinal` of `iteration` (`None`: a flat label).
fn occurrence_label(iteration: Option<u32>, ordinal: u32) -> String {
    match iteration {
        Some(iteration) => format!("it{iteration}/{UNIT_POSITION}{ordinal:06}"),
        None => format!("{UNIT_POSITION}{ordinal:06}"),
    }
}

/// A recorded position's `(iteration, ordinal)` and its label.
type RecordedPosition = ((u32, u32), String);

/// The occurrences earlier attempts of the node recorded, read before this
/// attempt prepared anything.
#[derive(Debug, Default)]
struct PriorOccurrences {
    /// Every recorded label.
    labels: HashSet<String>,
    /// The recorded positions of each family, in `(iteration, ordinal)`
    /// order.
    positions: HashMap<Family, Vec<RecordedPosition>>,
    /// Recorded labels whose slot changed nothing yet: prepared (perhaps
    /// by a prepare whose acknowledgement was lost) or with every call
    /// explained not crossed — no outcome, no call that may have crossed.
    unsettled: HashSet<String>,
    /// The recorded slots that may have changed the provider (a recorded
    /// outcome, or a call that crossed), per family and iteration: their
    /// ordinal and concurrency floor (their own ordinal when none was
    /// recorded).
    consequential: HashMap<(Family, u32), Vec<(u32, u32)>>,
    /// The latest iteration per family holding such a slot.
    last_consequential_iteration: HashMap<Family, u32>,
}

/// One recorded occurrence as the journal weighs it: its label, whether its
/// slot may have changed the provider ([`is_consequential`]) and the
/// concurrency floor recorded with it.
type RecordedOccurrence<'a> = (&'a str, bool, Option<u32>);

impl PriorOccurrences {
    /// Every label as consequential (a recorded outcome), with no floor.
    #[cfg(test)]
    fn new<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        Self::from_records(labels.into_iter().map(|label| (label, true, None)))
    }

    /// The recorded occurrences.
    fn from_records<'a>(records: impl IntoIterator<Item = RecordedOccurrence<'a>>) -> Self {
        let mut prior = Self::default();
        for (label, consequential, floor) in records {
            if let Some(position) = Position::parse(label) {
                prior
                    .positions
                    .entry(position.family)
                    .or_default()
                    .push((position.order(), label.to_owned()));
                if consequential {
                    // A legacy row without a floor counts as having
                    // started after everything below it finished.
                    let floor = floor.map_or(position.ordinal, |floor| floor.min(position.ordinal));
                    prior
                        .consequential
                        .entry((position.family, position.iteration))
                        .or_default()
                        .push((position.ordinal, floor));
                    let last = prior
                        .last_consequential_iteration
                        .entry(position.family)
                        .or_insert(position.iteration);
                    *last = (*last).max(position.iteration);
                }
            }
            if !consequential {
                prior.unsettled.insert(label.to_owned());
            }
            prior.labels.insert(label.to_owned());
        }
        for positions in prior.positions.values_mut() {
            positions.sort_unstable();
        }
        prior
    }

    /// Whether `occurrence` is recorded but changed nothing yet, while an
    /// earlier attempt applied (or may have applied) an effect that the
    /// program ran *after* it: running it now would apply it after that
    /// later effect, reversing the program's order.
    ///
    /// A consequential slot `H` of the same family ran after the recorded
    /// slot `L` when `H` is of a later iteration (an iteration's barrier
    /// drains it before the next begins), or when `L` lies below `H`'s
    /// concurrency floor — `L`'s unit was gone before `H` was first
    /// prepared. A slot at or above the floor ran concurrently with `H`:
    /// the program did not order them, and `L` replays under its recorded
    /// provider key (at least once).
    fn reorders_at(&self, occurrence: &str) -> bool {
        if !self.unsettled.contains(occurrence) {
            return false;
        }
        Position::parse(occurrence).is_some_and(|position| {
            let later_iteration = self
                .last_consequential_iteration
                .get(&position.family)
                .is_some_and(|&last| last > position.iteration);
            let later_in_order = self
                .consequential
                .get(&(position.family, position.iteration))
                .is_some_and(|slots| {
                    slots.iter().any(|&(ordinal, floor)| {
                        ordinal > position.ordinal && position.ordinal < floor
                    })
                });
            later_iteration || later_in_order
        })
    }

    /// The highest recorded `(iteration, ordinal)` of `family`.
    fn highest(&self, family: Family) -> Option<(u32, u32)> {
        self.positions
            .get(&family)
            .and_then(|positions| positions.last())
            .map(|(order, _)| *order)
    }

    /// Whether a fresh slot at `occurrence` is refused before anything
    /// else is consulted: it [leaves a gap](Self::leaves_gap_at) or
    /// [mixes families](Self::mixes_family_at). (A fresh slot above a
    /// recorded position this attempt never met is refused as well, once
    /// the attempt's own prepares below it settled — see
    /// [`NodeEffectJournal::first_unmet_below`].)
    fn refuses(&self, occurrence: &str) -> bool {
        self.leaves_gap_at(occurrence) || self.mixes_family_at(occurrence)
    }

    /// Whether `occurrence` is an unrecorded position below one an earlier
    /// attempt recorded in its family — a later ordinal of its iteration,
    /// or any slot of a later iteration: that attempt left it empty, so the
    /// program reached its later effects by another path, and an effect
    /// prepared here may be one already recorded further on.
    fn leaves_gap_at(&self, occurrence: &str) -> bool {
        if self.labels.contains(occurrence) {
            return false;
        }
        Position::parse(occurrence).is_some_and(|position| {
            self.highest(position.family)
                .is_some_and(|highest| highest > position.order())
        })
    }

    /// Whether `occurrence` is unrecorded and an earlier attempt recorded
    /// positions of the other family: the node's action changed kind
    /// (stateless and stateful), and its effects cannot be matched.
    fn mixes_family_at(&self, occurrence: &str) -> bool {
        if self.labels.contains(occurrence) {
            return false;
        }
        Position::parse(occurrence).is_some_and(|position| {
            self.positions
                .keys()
                .any(|&family| family != position.family)
        })
    }
}

/// Which recorded positions this node attempt met again.
#[derive(Debug, Default)]
struct MetPositions {
    /// Labels whose slot this attempt prepared (a recorded one replayed or
    /// resumed, a fresh one created).
    met: HashSet<String>,
    /// Labels handed to a unit whose prepare has not finished: not met
    /// yet, not given up
    /// ([`EffectJournal::release_occurrence`] settles them).
    claimed: HashSet<String>,
    /// Per family, how many of the earlier attempts' recorded positions —
    /// in order — this attempt is known to have met: every position before
    /// it was.
    frontier: HashMap<Family, usize>,
    /// Per family, the lowest position whose ledger prepare began and never
    /// answered (its unit was cancelled or ran past its deadline mid-call,
    /// or the ledger lost the acknowledgement): its row may exist. No fresh
    /// slot above it is prepared in this attempt.
    uncertain: HashMap<Family, (u32, u32)>,
    /// Per family, the positions whose unit is open: handed out
    /// ([`EffectJournal::next_occurrence`]) and not yet gone
    /// ([`EffectJournal::finish_occurrence`]). A unit open when another's
    /// fresh slot is prepared ran concurrently with it.
    open: HashMap<Family, std::collections::BTreeSet<(u32, u32)>>,
}

/// Marks a position uncertain when its ledger prepare is dropped before it
/// answered, or answered without saying whether the row was written.
struct PrepareInFlight<'a> {
    journal: &'a NodeEffectJournal,
    occurrence: &'a str,
    answered: bool,
}

impl Drop for PrepareInFlight<'_> {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        if let Some(position) = Position::parse(self.occurrence) {
            let order = position.order();
            let mut state = self.journal.state();
            let lowest = state
                .positions
                .uncertain
                .entry(position.family)
                .or_insert(order);
            *lowest = (*lowest).min(order);
        }
    }
}

/// Where the first recorded position below a candidate that this attempt
/// has not met stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unmet {
    /// Every recorded position below the candidate was met.
    None,
    /// The first unmet one is being prepared by another unit: wait.
    Pending,
    /// The first unmet one is neither met nor being prepared: the program
    /// passed it by. Its label.
    Skipped(String),
}

/// A unit's slot binding as the journal derives it from its intent.
struct DerivedBinding {
    contract: PreparedEffectContract,
    fingerprint: RequestFingerprint,
    provider_key: ProviderIdempotencyKey,
}

/// The recorded outcome of a journaled effect, as its frozen evidence
/// payload. Its tags differ from a remote effect's evidence, so neither
/// decodes as the other.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum JournalEvidence {
    /// The effect applied; its output as JSON.
    #[serde(rename = "JournalOutput")]
    Output {
        operation_id: [u8; 16],
        output: Value,
    },
    /// The effect applied; its output is not kept.
    #[serde(rename = "JournalOutputUnavailable")]
    OutputUnavailable { operation_id: [u8; 16] },
    /// The provider definitively rejected the effect.
    #[serde(rename = "JournalRejected")]
    Rejected {
        operation_id: [u8; 16],
        code: String,
    },
}

impl NodeEffectJournal {
    /// The journal of one node attempt, writing under `authority`. Builds
    /// nothing durable.
    pub(crate) fn new(authority: JournalAuthority) -> Self {
        Self::with_slot_cap(authority, MAX_NODE_SLOTS)
    }

    /// [`new`](Self::new) with another cap on the fresh slots one node
    /// attempt prepares (tests exercise the cap without ten thousand
    /// effects).
    fn with_slot_cap(authority: JournalAuthority, slot_cap: u32) -> Self {
        let execution = authority.execution_id.to_string();
        Self {
            inner: Arc::new(JournalInner {
                authority,
                execution,
                prior: tokio::sync::OnceCell::new(),
                state: Mutex::new(JournalState::default()),
                in_flight: AtomicUsize::new(0),
                slot_cap,
                drained: tokio::sync::Notify::new(),
                closed: AtomicBool::new(false),
                barrier_failed: AtomicBool::new(false),
                claims_settled: tokio::sync::Notify::new(),
            }),
        }
    }

    /// An in-flight ticket. Callers hold the state lock, so the count rises
    /// only in a transition that also sees the open iteration.
    fn ticket(&self) -> InFlight {
        self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        InFlight::new(move || {
            if inner.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
                inner.drained.notify_waiters();
            }
        })
    }

    /// The node's occurrences as earlier attempts recorded them, read once
    /// per node attempt, before its first prepare or its first iteration
    /// barrier.
    async fn prior(&self) -> Result<&PriorOccurrences, EffectExecutionError> {
        let authority = &self.inner.authority;
        self.inner
            .prior
            .get_or_try_init(|| async {
                let slots = authority
                    .ledger
                    .read_occurrences(
                        &authority.scope,
                        &self.inner.execution,
                        authority.node_key.as_str(),
                    )
                    .await?;
                Ok::<_, EffectExecutionError>(PriorOccurrences::from_records(slots.iter().map(
                    |slot| {
                        (
                            slot.occurrence(),
                            is_consequential(slot.record()),
                            slot.record().protocol().and_then(
                                nebula_storage_port::dto::OperationProtocolRecord::concurrent_floor,
                            ),
                        )
                    },
                )))
            })
            .await
    }

    /// The first position of `family` an earlier attempt recorded below
    /// `order` that this attempt has not met, and whether a unit is still
    /// preparing it.
    fn first_unmet_below(
        &self,
        prior: &PriorOccurrences,
        family: Family,
        order: (u32, u32),
    ) -> Unmet {
        let Some(recorded) = prior.positions.get(&family) else {
            return Unmet::None;
        };
        let mut state = self.state();
        let positions = &mut state.positions;
        let mut frontier = positions.frontier.get(&family).copied().unwrap_or(0);
        while recorded
            .get(frontier)
            .is_some_and(|(_, label)| positions.met.contains(label))
        {
            frontier += 1;
        }
        positions.frontier.insert(family, frontier);
        match recorded.get(frontier) {
            Some((recorded_order, label)) if *recorded_order < order => {
                if positions.claimed.contains(label) {
                    Unmet::Pending
                } else {
                    Unmet::Skipped(label.clone())
                }
            },
            _ => Unmet::None,
        }
    }

    /// Waits until every recorded position below `occurrence` that a unit of
    /// this attempt is preparing settled, then reports the first one this
    /// attempt passed by, if any: a fresh slot above it may be the effect
    /// recorded there, and preparing it would send that effect again.
    async fn skipped_below(&self, prior: &PriorOccurrences, occurrence: &str) -> Option<String> {
        let position = Position::parse(occurrence)?;
        loop {
            let settled = self.inner.claims_settled.notified();
            tokio::pin!(settled);
            settled.as_mut().enable();
            match self.first_unmet_below(prior, position.family, position.order()) {
                Unmet::None => return None,
                Unmet::Skipped(label) => return Some(label),
                Unmet::Pending => settled.await,
            }
        }
    }

    /// The admission witness for dispatching the journal's action, whose
    /// iteration barrier drains within the drain limit left before
    /// `execution_deadline`.
    pub(crate) fn admission(&self, execution_deadline: Option<Instant>) -> JournalAdmission {
        JournalAdmission {
            gate: JournalIterationGate {
                journal: self.clone(),
                execution_deadline,
            },
        }
    }

    /// Opens stateful `iteration`: its units are labelled
    /// `it{iteration}/unit/v1/#{ordinal:06}`, the ordinal restarting at
    /// zero.
    ///
    /// # Errors
    ///
    /// - the failure the journal already holds;
    /// - [`IterationUnitsOutstanding`](EffectExecutionError::IterationUnitsOutstanding)
    ///   when a unit of the node is still in flight (or the journal closed):
    ///   its position would cross into this iteration's namespace. Nothing
    ///   was waited for: the journal stays open, so the node's conclusion
    ///   drains the unit within its full limit and a call it was granted
    ///   may still settle;
    /// - [`InvalidContract`](EffectExecutionError::InvalidContract) for an
    ///   iteration past [`MAX_ITERATION`] or not after the open one.
    pub(crate) fn begin_iteration(&self, iteration: u32) -> Result<(), EffectExecutionError> {
        // One transition under the state lock: admission raises the
        // in-flight count under the same lock, so no unit is admitted
        // between the check and the rollover.
        let mut state = self.state();
        if let Some(failure) = state.failure {
            return Err(failure);
        }
        let in_flight = self.inner.in_flight.load(Ordering::SeqCst);
        if in_flight > 0 || self.is_closed() {
            drop(state);
            tracing::warn!(
                execution_id = %self.inner.authority.execution_id,
                node_key = %self.inner.authority.node_key,
                iteration,
                in_flight,
                "effect units still in flight as a stateful iteration begins; stopping"
            );
            let failure = EffectExecutionError::IterationUnitsOutstanding { iteration };
            self.note_failure(failure);
            return Err(self.state().failure.unwrap_or(failure));
        }
        if iteration > MAX_ITERATION || state.iteration.is_some_and(|open| iteration <= open) {
            drop(state);
            self.note_failure(EffectExecutionError::InvalidContract);
            return Err(EffectExecutionError::InvalidContract);
        }
        state.iteration = Some(iteration);
        state.run_open = true;
        state.next_ordinal = 0;
        state.iteration_slots.clear();
        Ok(())
    }

    /// Closes the open stateful iteration after its dispatch returned
    /// (`succeeded` when it returned `Ok`): drains its units for at most
    /// `drain_limit`, then reports whether the next iteration may start.
    ///
    /// # Errors
    ///
    /// - [`IterationUnitsOutstanding`](EffectExecutionError::IterationUnitsOutstanding)
    ///   when a unit outlived `drain_limit`: the journal closes, so the unit
    ///   records nothing more, and the node's verdict records a call it was
    ///   granted as ambiguous (the node then fails unknown; a unit that was
    ///   granted no call leaves this error as the verdict);
    /// - the failure the journal holds — a mismatch, an invalid record, a
    ///   deferring ledger or lease failure;
    /// - [`JournalOutcomeUnknown`](EffectExecutionError::JournalOutcomeUnknown)
    ///   when a slot of the iteration has an unknown outcome — recorded
    ///   unknown, a call never explained, or one that may have crossed with
    ///   no recorded outcome — even if the action swallowed the unit's
    ///   error;
    /// - [`OccurrenceMismatch`](EffectExecutionError::OccurrenceMismatch)
    ///   when the iteration succeeded although an earlier attempt recorded
    ///   an effect in it (or in an iteration before it) this attempt never
    ///   met: the replay took another path past an applied mutation, and no
    ///   later iteration may run on it. Reading what earlier attempts
    ///   recorded takes the node's one occurrence read, if no prepare took
    ///   it yet. A failing iteration keeps its own failure: the node's
    ///   conclusion lets a retry meet the effect again;
    /// - a deferring [`Unavailable`](OperationLedgerError::Unavailable)
    ///   ledger failure when that read does not answer within what is left
    ///   of `drain_limit` (at least [`FINAL_READ_FLOOR`]), as for the
    ///   verdict's final read.
    ///
    /// The iteration closes first: until the next
    /// [`begin_iteration`](Self::begin_iteration) an iterated journal admits
    /// no unit.
    pub(crate) async fn end_iteration(
        &self,
        drain_limit: Duration,
        succeeded: bool,
    ) -> Result<(), EffectExecutionError> {
        let iteration = {
            let mut state = self.state();
            state.run_open = false;
            state.iteration.unwrap_or(0)
        };
        let started = tokio::time::Instant::now();
        if !self.drain(drain_limit).await {
            let in_flight = self.inner.in_flight.load(Ordering::SeqCst);
            return Err(self.fail_barrier(iteration, in_flight));
        }
        if let Some(failure) = self.state().failure {
            return Err(failure);
        }
        if succeeded {
            let read_deadline = read_deadline(started, drain_limit);
            self.met_every_recorded_effect_through(iteration, read_deadline)
                .await?;
        }
        let slots: Vec<_> = {
            let state = self.state();
            state
                .iteration_slots
                .iter()
                .filter_map(|slot_id| {
                    state
                        .slots
                        .get(slot_id)
                        .map(|entry| (*slot_id, Arc::clone(entry)))
                })
                .collect()
        };
        // Every unit is gone: no slot lock is held. A slot still locked
        // could be mid-step and is never counted as resolved.
        let unresolved: Vec<EffectSlotId> = slots
            .iter()
            .filter(|(_, entry)| {
                entry
                    .try_lock()
                    .map_or(true, |slot| is_unresolved(slot.record()))
            })
            .map(|(slot_id, _)| *slot_id)
            .collect();
        let Some(first) = unresolved.first() else {
            return Ok(());
        };
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            iteration,
            unresolved = unresolved.len(),
            "a stateful iteration left an effect outcome unknown; no further iteration runs"
        );
        let unknown = EffectExecutionError::JournalOutcomeUnknown {
            slot_id: *first,
            unresolved: u32::try_from(unresolved.len()).unwrap_or(u32::MAX),
        };
        self.note_failure(unknown);
        Err(self.state().failure.unwrap_or(unknown))
    }

    /// The concurrency floor of `occurrence` now: the lowest ordinal of its
    /// iteration (its family's one run, for a flat label) whose unit is
    /// still open — at most its own, since its own unit is open while it
    /// prepares. Every position below the floor finished before this one
    /// began.
    fn concurrent_floor(&self, occurrence: &str) -> Option<u32> {
        let position = Position::parse(occurrence)?;
        let state = self.state();
        let lowest_open = state.positions.open.get(&position.family).and_then(|open| {
            open.range((position.iteration, 0)..=position.order())
                .next()
                .copied()
        });
        Some(lowest_open.map_or(position.ordinal, |(_, ordinal)| ordinal))
    }

    /// Whether a position of `occurrence`'s family below it was left
    /// uncertain by a prepare that never answered in this attempt.
    fn uncertain_below(&self, occurrence: &str) -> bool {
        Position::parse(occurrence).is_some_and(|position| {
            self.state()
                .positions
                .uncertain
                .get(&position.family)
                .is_some_and(|&lowest| lowest < position.order())
        })
    }

    /// Ends the open iteration of a cancelled node: no unit is admitted
    /// afterwards. A later submission (a detached task's) is refused closed
    /// with no failure of its own — the node is cancelled — and units in
    /// flight are left for the conclusion to drain.
    pub(crate) fn cancel_iteration(&self) {
        let mut state = self.state();
        state.run_open = false;
        state.admission_cancelled = true;
    }

    /// Checks that this attempt met every effect an earlier attempt recorded
    /// in iterations up to `iteration`; otherwise records an occurrence
    /// mismatch in the node's verdict. What earlier attempts recorded is
    /// read by `read_deadline` at the latest: a ledger that does not answer
    /// by then is a deferring `Unavailable` failure.
    async fn met_every_recorded_effect_through(
        &self,
        iteration: u32,
        read_deadline: tokio::time::Instant,
    ) -> Result<(), EffectExecutionError> {
        let read = tokio::time::timeout_at(read_deadline, self.prior())
            .await
            .unwrap_or_else(|_elapsed| {
                tracing::warn!(
                    execution_id = %self.inner.authority.execution_id,
                    node_key = %self.inner.authority.node_key,
                    iteration,
                    "the node's occurrences were not read within the barrier budget; deferring"
                );
                Err(EffectExecutionError::Ledger(
                    OperationLedgerError::Unavailable,
                ))
            });
        let prior = match read {
            Ok(prior) => prior,
            Err(error) => {
                self.note_failure(error);
                return Err(self.state().failure.unwrap_or(error));
            },
        };
        let next = (iteration.saturating_add(1), 0);
        let skipped = match self.first_unmet_below(prior, Family::Iterated, next) {
            Unmet::None => return Ok(()),
            // Every unit is gone: a position still claimed was given up.
            Unmet::Pending | Unmet::Skipped(_) => self.first_unmet_label(prior),
        };
        tracing::error!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            iteration,
            skipped = ?skipped,
            "a stateful iteration passed an earlier attempt's recorded effect by; stopping"
        );
        self.note_failure(EffectExecutionError::OccurrenceMismatch);
        Err(self
            .state()
            .failure
            .unwrap_or(EffectExecutionError::OccurrenceMismatch))
    }

    /// The first recorded iterated position this attempt has not met.
    fn first_unmet_label(&self, prior: &PriorOccurrences) -> Option<String> {
        let state = self.state();
        prior
            .positions
            .get(&Family::Iterated)?
            .iter()
            .find(|(_, label)| !state.positions.met.contains(label))
            .map(|(_, label)| label.clone())
    }

    /// Fails the barrier of `iteration` with `in_flight` units left after
    /// the drain limit: closes the journal and records the failure in the
    /// node's verdict, which does not wait for those units again.
    fn fail_barrier(&self, iteration: u32, in_flight: usize) -> EffectExecutionError {
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            iteration,
            in_flight,
            "effect units outlived their stateful iteration; closing the effect journal"
        );
        self.inner.barrier_failed.store(true, Ordering::SeqCst);
        self.close();
        let failure = EffectExecutionError::IterationUnitsOutstanding { iteration };
        self.note_failure(failure);
        self.state().failure.unwrap_or(failure)
    }

    fn access(&self) -> LedgerAccess<'_> {
        let authority = &self.inner.authority;
        LedgerAccess {
            ledger: authority.ledger.as_ref(),
            scope: &authority.scope,
            fencing: authority.fencing,
            clock: authority.clock.as_ref(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, JournalState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a failure of the node's verdict: the first one wins, except
    /// that a deferring failure (lease, acknowledgement, availability)
    /// replaces a non-deferring one.
    fn note_failure(&self, error: EffectExecutionError) {
        let mut state = self.state();
        let replace = match state.failure {
            None => true,
            Some(current) => !current.is_deferred() && error.is_deferred(),
        };
        if replace {
            state.failure = Some(error);
        }
    }

    /// Counts a refusal of `step` and returns it.
    fn refused(&self, step: &'static str, refusal: JournalRefusal) -> JournalRefusal {
        let metrics = &self.inner.authority.metrics;
        let labels = metrics
            .interner()
            .label_set(&[("step", step), ("refusal", refusal.as_str())]);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL, &labels)
        {
            counter.inc();
        }
        refusal
    }

    /// The refusal a unit sees for a durable `step` that failed with
    /// `error`, recording the failure in the node's verdict.
    fn refuse(&self, step: &'static str, error: EffectExecutionError) -> JournalRefusal {
        let (refusal, verdict) = classify_failure(error);
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            step,
            refusal = refusal.as_str(),
            code = verdict.code(),
            "effect journal refused a unit step"
        );
        self.note_failure(verdict);
        self.refused(step, refusal)
    }

    /// The slot this journal prepared for `unit`, for `step`.
    fn entry(
        &self,
        step: &'static str,
        unit: &JournalSlot,
    ) -> Result<Arc<tokio::sync::Mutex<LedgerSlot>>, JournalRefusal> {
        if self.is_closed() {
            return Err(self.refused(step, JournalRefusal::Closed));
        }
        let id = EffectSlotId::from_storage_bytes(*unit.id());
        let entry = self.state().slots.get(&id).cloned();
        entry.ok_or_else(|| self.refuse(step, EffectExecutionError::InvalidEvidence))
    }

    /// Derives the slot binding of `intent`: its contract identity, request
    /// fingerprint and provider idempotency key.
    ///
    /// The contract identity binds the action, the destination (resource
    /// key, credential slot identity, configuration fingerprint), the unit
    /// (kind, operation, version, effect class, recorded output) and the
    /// slot policy: any of them changing under a recorded occurrence is an
    /// occurrence mismatch, and nothing is sent. The provider key binds
    /// only what a provider deduplicates on.
    fn derive(&self, intent: &JournalIntent<'_>) -> Result<DerivedBinding, EffectExecutionError> {
        let authority = &self.inner.authority;
        let policy = slot_policy(intent.effect, intent.recovery, intent.max_invocations)?;
        let mut identity = Sha256::new();
        frame(&mut identity, CONTRACT_DOMAIN)?;
        frame(&mut identity, authority.action_key.as_bytes())?;
        frame(
            &mut identity,
            authority.action_version.to_string().as_bytes(),
        )?;
        // The destination: the row, its credentials and its configuration.
        frame(&mut identity, intent.resource_key.as_str().as_bytes())?;
        frame(&mut identity, &slot_identity_bytes(intent.binding)?)?;
        identity.update(intent.config_fingerprint.to_be_bytes());
        frame(&mut identity, intent.kind.as_str().as_bytes())?;
        frame(&mut identity, intent.operation.as_bytes())?;
        identity.update(intent.version.to_be_bytes());
        identity.update([effect_class(intent.effect)?]);
        // How a success is recorded decides what a replay yields.
        identity.update([u8::from(intent.record_output)]);
        identity.update([capability_discriminant(policy.capability())?]);
        identity.update(policy.max_invocations().to_be_bytes());
        identity.update(policy.max_queries().to_be_bytes());
        identity.update(policy.recovery_window_ms().to_be_bytes());
        match policy.stable_window_ms() {
            Some(window) => {
                identity.update([1]);
                identity.update(window.to_be_bytes());
            },
            None => identity.update([0]),
        }
        let identity: [u8; 32] = identity.finalize().into();
        let contract = PreparedEffectContract::new(RequestFingerprint::new(1, identity), policy)
            .map_err(|_| EffectExecutionError::InvalidContract)?;
        let mut request = Sha256::new();
        frame(&mut request, REQUEST_DOMAIN)?;
        request.update(identity);
        frame(&mut request, intent.canonical_request)?;
        let fingerprint = RequestFingerprint::new(1, request.finalize().into());
        let provider_key = provider_idempotency_key(&ProviderKeyParts {
            org_id: &authority.scope.org_id,
            workspace_id: &authority.scope.workspace_id,
            resource_key: intent.resource_key.as_str(),
            operation: intent.operation,
            version: intent.version,
            developer: intent.key_part,
            execution_id: &self.inner.execution,
            node_key: authority.node_key.as_str(),
            occurrence: intent.occurrence,
        })?;
        Ok(DerivedBinding {
            contract,
            fingerprint,
            provider_key,
        })
    }

    /// Waits for every in-flight unit to finish, at most `limit`; `true`
    /// when none is left.
    async fn drain(&self, limit: Duration) -> bool {
        let wait = async {
            loop {
                let notified = self.inner.drained.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.inner.in_flight.load(Ordering::SeqCst) == 0 {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(limit, wait).await.is_ok()
    }

    /// Closes the journal: every later unit step is refused
    /// [`Closed`](JournalRefusal::Closed), including steps of units it
    /// already prepared.
    pub(crate) fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
    }

    /// Ends the journal's node attempt: drains the in-flight units for at
    /// most `drain_limit`, closes the journal, records leaked calls within
    /// what is left of `drain_limit` and returns its verdict, which
    /// overrides the action's result.
    ///
    /// Reads the node's occurrences once, always: no reliable signal tells
    /// a first dispatch from one that follows a crash before the earlier
    /// attempt was recorded.
    ///
    /// # Errors
    ///
    /// - a deferring failure (lease lost, acknowledgement unknown, ledger
    ///   unavailable) — the turn must release its lease without finalizing;
    /// - [`JournalOutcomeUnknown`](EffectExecutionError::JournalOutcomeUnknown)
    ///   when any slot of the node has an unknown outcome — recorded
    ///   unknown, an unexplained call, an unresolved call that may have
    ///   crossed (an ambiguous stable-key call), or a slot a stuck unit kept
    ///   locked past the drain limit — even if the action swallowed the
    ///   unit's error;
    /// - [`OccurrenceMismatch`](EffectExecutionError::OccurrenceMismatch)
    ///   when the node is about to succeed (`node_succeeded`) although an
    ///   earlier attempt recorded an effect — settled, or a call that
    ///   crossed — that this attempt never prepared: the program took
    ///   another path, and its result would ignore an applied mutation;
    /// - any other failure the journal met (an occurrence mismatch, an
    ///   invalid contract or record).
    ///
    /// A failing node that skipped such an effect concludes
    /// [`SkippedRecordedEffect`](Concluded::SkippedRecordedEffect): its own
    /// failure stands — a retry may meet the effect again and replay it —
    /// but no error strategy may recover the node or route past it.
    ///
    /// The final read is bounded by what is left of `drain_limit`, and at
    /// least [`FINAL_READ_FLOOR`]; a read that does not answer by then is a
    /// deferring ledger failure (`Unavailable`).
    pub(crate) async fn conclude_node(
        &self,
        drain_limit: Duration,
        node_succeeded: bool,
    ) -> Result<Concluded, EffectExecutionError> {
        let verdict = self.verdict(drain_limit, node_succeeded).await;
        let label = verdict_label(verdict.as_ref().err());
        let metrics = &self.inner.authority.metrics;
        let labels = metrics.interner().single("code", label);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL, &labels)
        {
            counter.inc();
        }
        verdict
    }

    /// [`conclude_node`](Self::conclude_node) of a node whose action
    /// succeeded.
    #[cfg(test)]
    pub(crate) async fn conclude(&self, drain_limit: Duration) -> Result<(), EffectExecutionError> {
        self.conclude_node(drain_limit, true).await.map(|_| ())
    }

    async fn verdict(
        &self,
        drain_limit: Duration,
        node_succeeded: bool,
    ) -> Result<Concluded, EffectExecutionError> {
        let authority = &self.inner.authority;
        let drain_started = tokio::time::Instant::now();
        // A failed iteration barrier already waited the drain limit out for
        // the units still in flight: the cleanup gets its own floor instead.
        let (drain_limit, cleanup_budget) = if self.inner.barrier_failed.load(Ordering::SeqCst) {
            (Duration::ZERO, FINAL_READ_FLOOR)
        } else {
            (drain_limit, drain_limit)
        };
        if !self.drain(drain_limit).await {
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                in_flight = self.inner.in_flight.load(Ordering::SeqCst),
                "journaled units outlived their action; closing the effect journal"
            );
        }
        self.close();
        // The cleanup shares the drain's limit: a unit stuck in a ledger
        // call keeps its slot locked, and the node must still conclude.
        let cleanup_deadline = drain_started
            .checked_add(cleanup_budget)
            .unwrap_or_else(tokio::time::Instant::now);
        let uninspected = self.record_leaked_calls(cleanup_deadline).await;
        let failure = self.state().failure;
        if let Some(failure) = failure
            && failure.is_deferred()
        {
            return Err(failure);
        }
        // Always read, even when this attempt prepared nothing: a process
        // that died during an earlier dispatch of the node — before the
        // attempt was recorded, so this attempt's generation may still be
        // 1 — can have left a granted call that only the ledger knows of.
        // Bounded by what is left of the drain limit — at least
        // `FINAL_READ_FLOOR`, so a node whose units used it all can still
        // read — and deferred when the ledger does not answer by then: the
        // turn releases its lease without finalizing.
        let read_deadline = cleanup_deadline.max(
            tokio::time::Instant::now()
                .checked_add(FINAL_READ_FLOOR)
                .unwrap_or(cleanup_deadline),
        );
        let slots = tokio::time::timeout_at(
            read_deadline,
            authority.ledger.read_occurrences(
                &authority.scope,
                &self.inner.execution,
                authority.node_key.as_str(),
            ),
        )
        .await
        .map_err(|_elapsed| {
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                "the node's occurrences were not read within the verdict budget; deferring"
            );
            EffectExecutionError::Ledger(OperationLedgerError::Unavailable)
        })??;
        let mut unresolved: Vec<EffectSlotId> = slots
            .iter()
            .map(EffectOccurrenceRecord::record)
            .filter(|record| is_unresolved(record))
            .map(|record| record.operation().slot_id())
            .collect();
        // A slot whose unit still holds it could not be inspected: it may
        // be mid-call, so it is never counted as resolved.
        for slot_id in uninspected {
            if !unresolved.contains(&slot_id) {
                unresolved.push(slot_id);
            }
        }
        if let Some(first) = unresolved.first() {
            let listed: Vec<String> = unresolved.iter().map(ToString::to_string).collect();
            tracing::error!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                slots = ?listed,
                "journaled effect outcome unknown; failing the node"
            );
            return Err(EffectExecutionError::JournalOutcomeUnknown {
                slot_id: *first,
                unresolved: u32::try_from(unresolved.len()).unwrap_or(u32::MAX),
            });
        }
        if let Some(failure) = failure {
            return Err(failure);
        }
        // Every effect an earlier attempt recorded must have been met again:
        // the node's result stands for all of them.
        let prepared = self.state().slots.keys().copied().collect::<HashSet<_>>();
        let skipped: Vec<&str> = slots
            .iter()
            .filter(|slot| {
                !prepared.contains(&slot.record().operation().slot_id())
                    && is_consequential(slot.record())
            })
            .map(EffectOccurrenceRecord::occurrence)
            .collect();
        if skipped.is_empty() {
            return Ok(Concluded::Clean);
        }
        if node_succeeded {
            tracing::error!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                occurrences = ?skipped,
                "an earlier attempt's recorded effect was not met again; failing the node"
            );
            return Err(EffectExecutionError::OccurrenceMismatch);
        }
        tracing::warn!(
            execution_id = %authority.execution_id,
            node_key = %authority.node_key,
            occurrences = ?skipped,
            "a failing node did not meet an earlier attempt's recorded effect again"
        );
        Ok(Concluded::SkippedRecordedEffect)
    }

    /// Records every call a unit was granted and never explained as an
    /// ambiguous crossing: a unit that outlived its action (or whose outcome
    /// could not be recorded) may have reached the provider.
    ///
    /// Bounded by `deadline`: a slot whose unit still holds it past the
    /// deadline (a ledger request that never returns), or whose recording
    /// does not finish by then, is returned uninspected — the verdict counts
    /// it as unresolved.
    async fn record_leaked_calls(&self, deadline: tokio::time::Instant) -> Vec<EffectSlotId> {
        if self
            .state()
            .failure
            .is_some_and(EffectExecutionError::is_deferred)
        {
            return Vec::new();
        }
        let entries: Vec<_> = self
            .state()
            .slots
            .iter()
            .map(|(slot_id, entry)| (*slot_id, Arc::clone(entry)))
            .collect();
        let mut uninspected = Vec::new();
        for (slot_id, entry) in entries {
            // `timeout_at` polls the lock once even past the deadline: a free
            // slot is always inspected.
            let Ok(mut slot) = tokio::time::timeout_at(deadline, entry.lock()).await else {
                uninspected.push(slot_id);
                continue;
            };
            let outstanding = slot.protocol().ok().and_then(|protocol| {
                (protocol.phase() == EffectPhase::InvocationOutstanding)
                    .then(|| protocol.invocation())
                    .flatten()
            });
            let Some(call) = outstanding else {
                continue;
            };
            let recorded = tokio::time::timeout_at(
                deadline,
                slot.advance(
                    self.access(),
                    &OperationCommand::RecordDisposition {
                        invocation: call,
                        disposition: InvocationDisposition::Ambiguous,
                    },
                ),
            )
            .await;
            match recorded {
                Ok(Ok(_)) => {},
                Ok(Err(error)) => {
                    let _ = self.refuse(effect_journal_step::RECORD_LEAKED_CALL, error);
                    uninspected.push(slot_id);
                    return uninspected;
                },
                Err(_elapsed) => uninspected.push(slot_id),
            }
        }
        uninspected
    }

    /// Resolves the phase a unit sees for a prepared `slot`, recording a
    /// crash residue (an outstanding call) as an ambiguous crossing first.
    async fn resolve_phase(
        &self,
        slot: &mut LedgerSlot,
    ) -> Result<SlotPhase, EffectExecutionError> {
        if slot.protocol()?.phase() == EffectPhase::InvocationOutstanding {
            // The call may have reached the provider. An opaque effect's
            // outcome becomes unknown; a stable-key effect may be granted
            // again within its window.
            let call = slot
                .protocol()?
                .invocation()
                .ok_or(EffectExecutionError::InvalidEvidence)?;
            slot.advance(
                self.access(),
                &OperationCommand::RecordDisposition {
                    invocation: call,
                    disposition: InvocationDisposition::Ambiguous,
                },
            )
            .await?;
        }
        slot_phase(slot)
    }

    /// Withholds a granted `call` whose window ended before the grant
    /// reached the journal. The call is never handed out. When an earlier
    /// call of the slot may have crossed, a resend could reach a provider
    /// that no longer deduplicates it: the outcome becomes unknown.
    /// Otherwise nothing ever crossed: the call is recorded not crossed and
    /// refused as unavailable (the ledger answered too slowly).
    async fn expired_grant(&self, slot: &mut LedgerSlot, call: OperationCallId) -> JournalRefusal {
        const STEP: &str = effect_journal_step::GRANT;
        // The withheld call is counted among the crossed ones until it is
        // explained.
        let crossed_before = slot.protocol().map_or(1, |protocol| {
            protocol.crossed_invocations().saturating_sub(1)
        });
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            crossed_before,
            "journaled effect granted with no window left; call withheld"
        );
        if crossed_before > 0 {
            return match slot.mark_unknown(self.access()).await {
                Ok(()) => self.refused(STEP, JournalRefusal::Unknown),
                Err(error) => self.refuse(STEP, error),
            };
        }
        let recorded = slot
            .advance(
                self.access(),
                &OperationCommand::RecordDisposition {
                    invocation: call,
                    disposition: InvocationDisposition::BeforeBoundary,
                },
            )
            .await;
        match recorded {
            Ok(_) => self.refused(STEP, JournalRefusal::Unavailable),
            Err(error) => self.refuse(STEP, error),
        }
    }

    fn count_prepared(&self, phase: &SlotPhase) {
        let label = match phase {
            SlotPhase::Runnable => effect_journal_prepare_phase::RUNNABLE,
            SlotPhase::Replay(_) => effect_journal_prepare_phase::REPLAY,
            _ => effect_journal_prepare_phase::UNKNOWN,
        };
        let metrics = &self.inner.authority.metrics;
        let labels = metrics.interner().single("phase", label);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL, &labels)
        {
            counter.inc();
        }
    }
}

impl fmt::Debug for NodeEffectJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeEffectJournal")
            .field("execution_id", &self.inner.authority.execution_id)
            .field("node_key", &self.inner.authority.node_key)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl EffectJournal for NodeEffectJournal {
    fn next_ordinal(&self) -> u32 {
        let mut state = self.state();
        let ordinal = state.next_ordinal;
        state.next_ordinal = ordinal.saturating_add(1);
        ordinal
    }

    /// `unit/v1/#{ordinal:06}` for a stateless action;
    /// `it{n}/unit/v1/#{ordinal:06}` within stateful iteration `n`.
    ///
    /// The label stays claimed until the unit
    /// [releases](EffectJournal::release_occurrence) it: a fresh slot above
    /// it waits for that unit's prepare to settle before deciding whether
    /// the position was met.
    fn next_occurrence(&self) -> String {
        let mut state = self.state();
        let ordinal = state.next_ordinal;
        state.next_ordinal = ordinal.saturating_add(1);
        let label = occurrence_label(state.iteration, ordinal);
        state.positions.claimed.insert(label.clone());
        if let Some(position) = Position::parse(&label) {
            state
                .positions
                .open
                .entry(position.family)
                .or_default()
                .insert(position.order());
        }
        label
    }

    fn finish_occurrence(&self, occurrence: &str) {
        if let Some(position) = Position::parse(occurrence)
            && let Some(open) = self.state().positions.open.get_mut(&position.family)
        {
            open.remove(&position.order());
        }
    }

    fn release_occurrence(&self, occurrence: &str) {
        self.state().positions.claimed.remove(occurrence);
        self.inner.claims_settled.notify_waiters();
    }

    async fn prepare(&self, intent: &JournalIntent<'_>) -> Result<JournalSlot, JournalRefusal> {
        const STEP: &str = effect_journal_step::PREPARE;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let authority = &self.inner.authority;
        let derived = self
            .derive(intent)
            .map_err(|error| self.refuse(STEP, error))?;
        // Every prepare waits for the one read of what earlier attempts
        // recorded, taken before this attempt writes anything.
        let prior = self
            .prior()
            .await
            .map_err(|error| self.refuse(STEP, error))?;
        if prior.refuses(intent.occurrence) {
            // An earlier attempt reached a later position without recording
            // this one — the effect may be one it recorded further on, and a
            // fresh slot here would send it again — or recorded positions of
            // the other family (the node's action changed kind).
            return Err(self.refuse(STEP, EffectExecutionError::OccurrenceMismatch));
        }
        if prior.reorders_at(intent.occurrence) {
            // A recorded effect that changed nothing yet below one an
            // earlier attempt applied (or may have): running it now would
            // apply it after that later effect.
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                occurrence = intent.occurrence,
                "a recorded effect below an applied later one would run out of order; refused"
            );
            return Err(self.refuse(STEP, EffectExecutionError::OccurrenceMismatch));
        }
        let fresh = !prior.labels.contains(intent.occurrence);
        if fresh && self.uncertain_below(intent.occurrence) {
            // A lower position's prepare never answered: its row may exist,
            // and a recovery would run that effect after this one. Defer:
            // the next attempt reads what was written and replays in order.
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                occurrence = intent.occurrence,
                "a fresh effect above a position whose prepare never answered; deferring"
            );
            return Err(self.refuse(
                STEP,
                EffectExecutionError::Ledger(OperationLedgerError::AcknowledgementUnknown),
            ));
        }
        if fresh && let Some(skipped) = self.skipped_below(prior, intent.occurrence).await {
            // This attempt passed a recorded effect by (another path, or a
            // unit that gave up before reaching the journal): the fresh
            // effect here may be that one, and its slot would send it again
            // under another provider key.
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                occurrence = intent.occurrence,
                skipped = %skipped,
                "a fresh effect above a recorded one this attempt never met; refused"
            );
            return Err(self.refuse(STEP, EffectExecutionError::OccurrenceMismatch));
        }
        if fresh {
            // Only a fresh slot counts against the cap — a recorded one
            // replays however many there are — and the cap is taken before
            // anything durable: a refused prepare writes and sends nothing.
            let cap = self.inner.slot_cap;
            let capped = {
                let mut state = self.state();
                let capped = state.reserved >= cap;
                if !capped {
                    state.reserved += 1;
                }
                capped
            };
            if capped {
                return Err(self.refuse(STEP, EffectExecutionError::JournalSlotCapExceeded { cap }));
            }
        }
        let binding = EffectSlotBinding {
            scope: &authority.scope,
            execution_id: &self.inner.execution,
            node_key: authority.node_key.as_str(),
            occurrence: intent.occurrence,
            attempt_generation: AttemptGeneration::new(authority.attempt_generation),
            fingerprint: derived.fingerprint,
            destination: derived.contract.policy().capability(),
            contract: &derived.contract,
            provider_key: Some(derived.provider_key),
            // Recorded with a fresh slot only; a recorded one keeps its own.
            concurrent_floor: self.concurrent_floor(intent.occurrence),
        };
        // From here until the ledger answers, the row may be written without
        // this unit learning it: a prepare dropped mid-call (the unit
        // cancelled or past its deadline) or one that cannot tell whether
        // it was acknowledged leaves the position uncertain.
        let mut in_flight = PrepareInFlight {
            journal: self,
            occurrence: intent.occurrence,
            answered: false,
        };
        let prepared = LedgerSlot::prepare(self.access(), &binding).await;
        in_flight.answered = match &prepared {
            Ok(_) => true,
            // A definite refusal wrote nothing; a deferring one may have.
            Err(error) => !error.is_deferred(),
        };
        drop(in_flight);
        let slot = prepared.map_err(|error| self.refuse(STEP, error))?;
        // The position is met: the slot recorded here is this unit's.
        self.state()
            .positions
            .met
            .insert(intent.occurrence.to_owned());
        self.inner.claims_settled.notify_waiters();
        // The key the provider receives is the durable one, read back from
        // the ledger — never recomputed for an existing slot.
        let idempotency_key = slot
            .provider_key()
            .and_then(|key| IdempotencyKey::new(key.as_str()).ok())
            .ok_or_else(|| self.refuse(STEP, EffectExecutionError::InvalidEvidence))?;
        let slot_id = slot.slot_id();
        let entry = Arc::new(tokio::sync::Mutex::new(slot));
        {
            let mut state = self.state();
            if state.slots.contains_key(&slot_id) {
                drop(state);
                // One occurrence submitted twice in one node attempt: two
                // units would share one slot's calls.
                return Err(self.refuse(STEP, EffectExecutionError::OccurrenceMismatch));
            }
            state.slots.insert(slot_id, Arc::clone(&entry));
            if state.iteration.is_some() {
                state.iteration_slots.push(slot_id);
            }
        }
        let mut slot = entry.lock().await;
        let phase = self
            .resolve_phase(&mut slot)
            .await
            .map_err(|error| self.refuse(STEP, error))?;
        let revision = slot
            .protocol()
            .map_err(|error| self.refuse(STEP, error))?
            .revision();
        self.count_prepared(&phase);
        tracing::debug!(
            execution_id = %authority.execution_id,
            node_key = %authority.node_key,
            slot_id = %slot_id,
            occurrence = intent.occurrence,
            phase = ?phase,
            "journaled effect prepared"
        );
        Ok(JournalSlot::new(
            *slot_id.as_bytes(),
            idempotency_key,
            revision,
            phase,
        ))
    }

    async fn grant(&self, unit: &JournalSlot) -> Result<CallGrant, JournalRefusal> {
        const STEP: &str = effect_journal_step::GRANT;
        let authority = &self.inner.authority;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let protocol = slot.protocol().map_err(|error| self.refuse(STEP, error))?;
        if protocol.phase() == EffectPhase::OutcomeUnknown {
            return Err(self.refused(STEP, JournalRefusal::Unknown));
        }
        let revision = protocol.revision();
        match slot
            .advance(
                self.access(),
                &OperationCommand::GrantInvocation {
                    expected_revision: revision,
                },
            )
            .await
        {
            Ok(Some(GrantedCall::Invocation {
                call,
                authorized_at_ms,
                request_started,
            })) => {
                if self.is_closed() {
                    // The node concluded while the grant was in flight: the
                    // call is never handed out, so it provably did not cross.
                    let _ = slot
                        .advance(
                            self.access(),
                            &OperationCommand::RecordDisposition {
                                invocation: call,
                                disposition: InvocationDisposition::BeforeBoundary,
                            },
                        )
                        .await;
                    return Err(self.refused(STEP, JournalRefusal::Closed));
                }
                // The ledger vouches for the call only until its window ends
                // (a stable key's deduplication, an opaque effect's recovery
                // window): the unit must finish the call within what is left.
                let budget = match slot.call_timing(
                    authority.clock.as_ref(),
                    CallPurpose::Invocation,
                    authorized_at_ms,
                    request_started,
                ) {
                    Ok((_, budget)) => budget,
                    Err(error) => return Err(self.refuse(STEP, error)),
                };
                if budget.is_zero() {
                    return Err(self.expired_grant(&mut slot, call).await);
                }
                Ok(CallGrant::from_bytes(*call.as_bytes()).with_budget(budget))
            },
            // The ledger refused a fresh call: the budget or the window ran
            // out and the outcome is now unknown.
            Ok(None)
                if slot
                    .protocol()
                    .is_ok_and(|protocol| protocol.phase() == EffectPhase::OutcomeUnknown) =>
            {
                Err(self.refused(STEP, JournalRefusal::Unknown))
            },
            Ok(_) => Err(self.refuse(STEP, EffectExecutionError::InvalidEvidence)),
            Err(error) => Err(self.refuse(STEP, error)),
        }
    }

    async fn explain(
        &self,
        unit: &JournalSlot,
        call: CallGrant,
        crossing: Crossing,
    ) -> Result<(), JournalRefusal> {
        const STEP: &str = effect_journal_step::EXPLAIN;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let disposition = match crossing {
            Crossing::NotCrossed => InvocationDisposition::BeforeBoundary,
            // An unrecognized crossing may have reached the provider.
            _ => InvocationDisposition::Ambiguous,
        };
        slot.advance(
            self.access(),
            &OperationCommand::RecordDisposition {
                invocation: OperationCallId::from_bytes(*call.as_bytes()),
                disposition,
            },
        )
        .await
        .map(|_| ())
        .map_err(|error| self.refuse(STEP, error))
    }

    async fn settle(
        &self,
        unit: &JournalSlot,
        call: CallGrant,
        outcome: CallOutcome<'_>,
    ) -> Result<(), JournalRefusal> {
        const STEP: &str = effect_journal_step::SETTLE;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let evidence = journal_evidence(
            slot.operation_id(),
            OperationCallId::from_bytes(*call.as_bytes()),
            outcome,
        )
        .map_err(|error| self.refuse(STEP, error))?;
        slot.commit_evidence(self.access(), &evidence)
            .await
            .map_err(|error| self.refuse(STEP, error))
    }

    /// A ticket raised under the state lock, like every admission.
    fn track(&self) -> InFlight {
        let _state = self.state();
        self.ticket()
    }

    /// Admits a unit in one transition with the iteration rollover: under
    /// the state lock, refused when the journal closed or — for a stateful
    /// action — when no iteration is open (between an iteration's end and
    /// the next one's begin, a unit belongs to neither). Such a unit is a
    /// detached submission outliving its iteration: the node's verdict
    /// records it.
    fn admit(&self) -> Result<InFlight, JournalRefusal> {
        const STEP: &str = effect_journal_step::SUBMIT;
        let (refusal, failure) = {
            let state = self.state();
            if self.is_closed() || state.admission_cancelled {
                (JournalRefusal::Closed, None)
            } else if self.inner.authority.shape == JournalShape::Iterated && !state.run_open {
                let iteration = state.iteration.unwrap_or(0);
                (
                    JournalRefusal::BetweenRuns,
                    Some(EffectExecutionError::IterationUnitsOutstanding { iteration }),
                )
            } else {
                return Ok(self.ticket());
            }
        };
        if let Some(failure) = failure {
            tracing::warn!(
                execution_id = %self.inner.authority.execution_id,
                node_key = %self.inner.authority.node_key,
                code = failure.code(),
                "an effect unit was submitted between stateful iterations; refused"
            );
            self.note_failure(failure);
        }
        Err(self.refused(STEP, refusal))
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

/// The refusal a unit sees for a durable step that failed with `error`,
/// and the failure it records in the node's verdict.
fn classify_failure(error: EffectExecutionError) -> (JournalRefusal, EffectExecutionError) {
    match error {
        EffectExecutionError::OccurrenceMismatch
        | EffectExecutionError::Ledger(OperationLedgerError::OperationMismatch { .. }) => (
            JournalRefusal::Mismatch,
            EffectExecutionError::OccurrenceMismatch,
        ),
        EffectExecutionError::Ledger(OperationLedgerError::InvalidOccurrence { .. })
        | EffectExecutionError::InvalidContract => (
            JournalRefusal::Mismatch,
            EffectExecutionError::InvalidContract,
        ),
        EffectExecutionError::Ledger(OperationLedgerError::AcknowledgementUnknown) => {
            (JournalRefusal::AcknowledgementUnknown, error)
        },
        EffectExecutionError::Ledger(OperationLedgerError::ExecutionLeaseRejected) => {
            (JournalRefusal::LeaseLost, error)
        },
        EffectExecutionError::JournalSlotCapExceeded { .. } => {
            (JournalRefusal::SlotCapExceeded, error)
        },
        error => (JournalRefusal::Unavailable, error),
    }
}

/// The `code` label of a journal verdict.
fn verdict_label(failure: Option<&EffectExecutionError>) -> &'static str {
    match failure {
        None => effect_journal_verdict::OK,
        Some(error) if error.is_deferred() => effect_journal_verdict::DEFERRED,
        Some(EffectExecutionError::JournalOutcomeUnknown { .. }) => {
            effect_journal_verdict::OUTCOME_UNKNOWN
        },
        Some(EffectExecutionError::OccurrenceMismatch) => {
            effect_journal_verdict::OCCURRENCE_MISMATCH
        },
        Some(EffectExecutionError::InvalidContract) => effect_journal_verdict::INVALID_CONTRACT,
        Some(EffectExecutionError::InvalidEvidence) => effect_journal_verdict::INVALID_EVIDENCE,
        Some(EffectExecutionError::JournalSlotCapExceeded { .. }) => {
            effect_journal_verdict::SLOT_CAP_EXCEEDED
        },
        Some(EffectExecutionError::IterationUnitsOutstanding { .. }) => {
            effect_journal_verdict::ITERATION_BARRIER
        },
        Some(_) => effect_journal_verdict::LEDGER,
    }
}

/// Whether a slot of the node leaves its effect's outcome unknown: a slot
/// whose outcome is not recorded although a call may have reached the
/// provider — recorded unknown, outstanding (a crashed or leaked call), or
/// any later phase after a call that may have crossed, such as an
/// ambiguous stable-key call the unit did not get to resend (or whose
/// error the action swallowed).
fn is_unresolved(record: &OperationRecord) -> bool {
    record.protocol().map_or_else(
        || record.state() == OperationState::OutcomeUnknown,
        |protocol| match protocol.phase() {
            EffectPhase::Resolved => false,
            EffectPhase::OutcomeUnknown | EffectPhase::InvocationOutstanding => true,
            _ => protocol.crossed_invocations() > 0,
        },
    )
}

/// Whether a slot records something that may have changed the provider: a
/// recorded outcome, or a call that crossed. A slot only prepared, or whose
/// calls all stayed before the boundary, changed nothing.
fn is_consequential(record: &OperationRecord) -> bool {
    record.protocol().is_none_or(|protocol| {
        protocol.phase() == EffectPhase::Resolved || protocol.crossed_invocations() > 0
    })
}

/// The phase a unit sees for a prepared `slot` with no outstanding call.
fn slot_phase(slot: &LedgerSlot) -> Result<SlotPhase, EffectExecutionError> {
    let protocol = slot.protocol()?;
    match protocol.phase() {
        EffectPhase::Prepared | EffectPhase::BeforeBoundary | EffectPhase::Ambiguous => {
            Ok(SlotPhase::Runnable)
        },
        EffectPhase::OutcomeUnknown => Ok(SlotPhase::Unknown),
        EffectPhase::Resolved => {
            let evidence = protocol
                .evidence()
                .ok_or(EffectExecutionError::InvalidEvidence)?;
            replay(slot.operation_id(), evidence).map(SlotPhase::Replay)
        },
        _ => Err(EffectExecutionError::InvalidEvidence),
    }
}

/// The durable policy of a journaled effect's slot.
///
/// - `Idempotent` + stable key: at most `max_invocations` calls (clamped to
///   the ledger's `1..=10_000`), the author's key window (at most a year),
///   a recovery window covering it and at least [`OPERATION_DEADLINE_CAP`];
/// - `Write` + opaque: the recovery window is [`OPERATION_DEADLINE_CAP`].
///
/// No queries: journaled effects have no reconciliation query yet.
fn slot_policy(
    effect: Effect,
    recovery: Recovery,
    max_invocations: NonZeroU32,
) -> Result<PreparedEffectPolicy, EffectExecutionError> {
    let max_invocations = max_invocations.get().min(MAX_SLOT_INVOCATIONS);
    let builder = match (effect, recovery) {
        (Effect::Idempotent, Recovery::StableKey { window }) => {
            let window = window.min(MAX_LEDGER_WINDOW);
            PreparedEffectPolicy::builder(DestinationCapability::StableKey)
                .stable_key_window(window)
                .recovery_window(window.max(OPERATION_DEADLINE_CAP).min(MAX_LEDGER_WINDOW))
        },
        (Effect::Write, Recovery::Opaque) => {
            PreparedEffectPolicy::builder(DestinationCapability::Opaque)
                .recovery_window(OPERATION_DEADLINE_CAP)
        },
        _ => return Err(EffectExecutionError::InvalidContract),
    };
    builder
        .maximum_invocations(max_invocations)
        .maximum_queries(0)
        .build()
        .map_err(|_| EffectExecutionError::InvalidContract)
}

/// The byte of a unit's declared effect class in its contract identity.
fn effect_class(effect: Effect) -> Result<u8, EffectExecutionError> {
    match effect {
        Effect::Idempotent => Ok(1),
        Effect::Write => Ok(2),
        _ => Err(EffectExecutionError::InvalidContract),
    }
}

/// The structural bytes of a row's credential slot identity: `0` when
/// unbound; otherwise `1`, the pair count and every framed `(slot,
/// credential)` pair in canonical order. Credential ids, not material: a
/// rotation keeps the identity, re-pointing a slot changes it.
fn slot_identity_bytes(binding: &SlotIdentity) -> Result<Vec<u8>, EffectExecutionError> {
    match binding {
        SlotIdentity::Unbound => Ok(vec![0]),
        SlotIdentity::Structural(pairs) => {
            let count =
                u64::try_from(pairs.len()).map_err(|_| EffectExecutionError::InvalidContract)?;
            let mut bytes = vec![1];
            bytes.extend_from_slice(&count.to_be_bytes());
            for (slot, credential) in pairs.iter() {
                push_frame(&mut bytes, slot.as_bytes())?;
                push_frame(&mut bytes, credential.as_bytes())?;
            }
            Ok(bytes)
        },
        _ => Err(EffectExecutionError::InvalidContract),
    }
}

/// Appends `bytes` to `buffer`, framed by their big-endian `u64` length.
fn push_frame(buffer: &mut Vec<u8>, bytes: &[u8]) -> Result<(), EffectExecutionError> {
    let length = u64::try_from(bytes.len()).map_err(|_| EffectExecutionError::InvalidContract)?;
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(bytes);
    Ok(())
}

/// What the provider idempotency key of one effect is composed of.
pub(super) struct ProviderKeyParts<'a> {
    pub org_id: &'a str,
    pub workspace_id: &'a str,
    pub resource_key: &'a str,
    pub operation: &'a str,
    pub version: u32,
    /// The author's key part, when the operation declared one.
    pub developer: Option<&'a str>,
    pub execution_id: &'a str,
    pub node_key: &'a str,
    pub occurrence: &'a str,
}

/// The provider idempotency key of one journaled effect:
///
/// ```text
/// base64url_nopad(SHA-256(
///     frame("nebula.idempotency-key.v1")
///   ‖ frame(frame(org_id) ‖ frame(workspace_id))
///   ‖ frame(resource_key)
///   ‖ frame(operation)
///   ‖ u32_be(version)
///   ‖ frame(developer part)))
/// ```
///
/// where `frame(x) = u64_be(len(x)) ‖ x` and the developer part is the
/// author's key part, or — without one — `frame(execution_id) ‖
/// frame(node_key) ‖ frame(occurrence)`. A developer key therefore
/// deduplicates across executions and carries no execution id; neither form
/// contains an attempt number, so every retry and resume presents the same
/// key. 43 characters. Separate from the resource runtime's local key
/// (domain `nebula.idempotency.local.v1`) that an unjournaled row presents.
pub(super) fn provider_idempotency_key(
    parts: &ProviderKeyParts<'_>,
) -> Result<ProviderIdempotencyKey, EffectExecutionError> {
    let mut tenant = Vec::new();
    push_frame(&mut tenant, parts.org_id.as_bytes())?;
    push_frame(&mut tenant, parts.workspace_id.as_bytes())?;
    let mut digest = Sha256::new();
    frame(&mut digest, IDEMPOTENCY_KEY_DOMAIN)?;
    frame(&mut digest, &tenant)?;
    frame(&mut digest, parts.resource_key.as_bytes())?;
    frame(&mut digest, parts.operation.as_bytes())?;
    digest.update(parts.version.to_be_bytes());
    if let Some(developer) = parts.developer {
        frame(&mut digest, developer.as_bytes())?;
    } else {
        let mut run = Vec::new();
        push_frame(&mut run, parts.execution_id.as_bytes())?;
        push_frame(&mut run, parts.node_key.as_bytes())?;
        push_frame(&mut run, parts.occurrence.as_bytes())?;
        frame(&mut digest, &run)?;
    }
    let digest: [u8; 32] = digest.finalize().into();
    ProviderIdempotencyKey::new(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
        .map_err(|_| EffectExecutionError::InvalidContract)
}

/// Every closed rejection code a journaled effect may record.
const ERROR_KIND_CODES: [ErrorKindCode; 10] = [
    ErrorKindCode::Transient,
    ErrorKindCode::Permanent,
    ErrorKindCode::Exhausted,
    ErrorKindCode::Backpressure,
    ErrorKindCode::NotFound,
    ErrorKindCode::Cancelled,
    ErrorKindCode::Revoked,
    ErrorKindCode::Ambiguous,
    ErrorKindCode::CredentialUnavailable,
    ErrorKindCode::OutcomeUnknown,
];

/// The frozen evidence of a granted call's `outcome`.
fn journal_evidence(
    operation_id: OperationId,
    call: OperationCallId,
    outcome: CallOutcome<'_>,
) -> Result<FrozenOutcomeEvidence, EffectExecutionError> {
    let operation_id = *operation_id.as_bytes();
    let unavailable = || JournalEvidence::OutputUnavailable { operation_id };
    let (known, recorded) = match outcome {
        CallOutcome::Applied(bytes) => (
            KnownOutcome::Succeeded,
            serde_json::from_slice::<Value>(bytes).map_or_else(
                |_| unavailable(),
                |output| JournalEvidence::Output {
                    operation_id,
                    output,
                },
            ),
        ),
        CallOutcome::AppliedWithoutOutput => (KnownOutcome::Succeeded, unavailable()),
        CallOutcome::Rejected(code) => (
            KnownOutcome::Failed,
            JournalEvidence::Rejected {
                operation_id,
                code: code.as_str().to_owned(),
            },
        ),
        _ => return Err(EffectExecutionError::InvalidEvidence),
    };
    let source = OutcomeEvidenceSource::Invocation(call);
    let payload =
        serde_json::to_vec(&recorded).map_err(|_| EffectExecutionError::InvalidEvidence)?;
    // A known applied effect never becomes retryable because its output is
    // too large to keep: the bounded terminal fact remains durable.
    FrozenOutcomeEvidence::v1_json(source, known, payload).or_else(|_| {
        let payload = serde_json::to_vec(&unavailable())
            .map_err(|_| EffectExecutionError::InvalidEvidence)?;
        FrozenOutcomeEvidence::v1_json(source, known, payload).map_err(Into::into)
    })
}

/// The recorded outcome a resumed unit replays from `evidence`.
fn replay(
    operation_id: OperationId,
    evidence: &FrozenOutcomeEvidence,
) -> Result<RecordedOutcome, EffectExecutionError> {
    evidence
        .validate()
        .map_err(|_| EffectExecutionError::InvalidEvidence)?;
    let recorded = serde_json::from_slice::<JournalEvidence>(evidence.payload());
    let operation_id = *operation_id.as_bytes();
    match (evidence.outcome(), recorded) {
        (
            KnownOutcome::Succeeded,
            Ok(JournalEvidence::Output {
                operation_id: recorded,
                output,
            }),
        ) if recorded == operation_id => serde_json::to_vec(&output)
            .map(RecordedOutcome::Succeeded)
            .map_err(|_| EffectExecutionError::InvalidEvidence),
        (
            KnownOutcome::Succeeded,
            Ok(JournalEvidence::OutputUnavailable {
                operation_id: recorded,
            }),
        ) if recorded == operation_id => Ok(RecordedOutcome::OutputUnavailable),
        (
            KnownOutcome::Failed,
            Ok(JournalEvidence::Rejected {
                operation_id: recorded,
                code,
            }),
        ) if recorded == operation_id => ERROR_KIND_CODES
            .into_iter()
            .find(|known| known.as_str() == code)
            .map(RecordedOutcome::Failed)
            .ok_or(EffectExecutionError::InvalidEvidence),
        // An operator's adjudication records the outcome in its own words:
        // replay only what is known, never the output.
        (KnownOutcome::Succeeded, _)
            if matches!(evidence.source(), OutcomeEvidenceSource::Adjudication(_)) =>
        {
            Ok(RecordedOutcome::OutputUnavailable)
        },
        (KnownOutcome::Failed, _)
            if matches!(evidence.source(), OutcomeEvidenceSource::Adjudication(_)) =>
        {
            Ok(RecordedOutcome::Failed(ErrorKindCode::Permanent))
        },
        _ => Err(EffectExecutionError::InvalidEvidence),
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
