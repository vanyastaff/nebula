//! Execution-owned effects on a managed row, against a deterministic
//! in-memory owner ([`FakeOwner`]) whose phase machine mirrors the operation
//! ledger's rules: an opaque effect is granted again only from prepared or
//! not-crossed, a stable-key one also from ambiguous within its window, an
//! exhausted budget or an ambiguous opaque call makes the outcome unknown.
//!
//! The routing matrix — authority × effect, operations, sessions and
//! streams — and the declaration the runtime derives (occurrence, contract,
//! canonical request, key part, recovery, recorded output) are covered
//! here too.

use std::{
    collections::{BTreeMap, HashMap},
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::Scope;
use nebula_credential::CredentialAvailability;
use nebula_metrics::MetricsRegistry;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::super::{
    Cost, Effect, IdempotencyKey, Operation, OperationCx, OperationError, PinSlots, ResourceHandle,
    SentState, SessionClosed, SessionSpec, StreamOperation, StreamSink,
    declaration::local_idempotency_key,
    journal::{
        CallGrant, CallOutcome, Crossing, EffectJournal, ErrorKindCode, InFlight, JournalIntent,
        JournalRefusal, JournalSlot, RecordedOutcome, Recovery, SlotPhase, UnitKind,
    },
    managed::submit_unit,
    work::Plain,
};
use crate::{
    AcquireOptions, ErrorKind, Manager, PoolConfig, Pooled, Provider, RegistrationSpec,
    ResourceContext, SlotIdentity,
    manager::strict_fixtures::{
        ScriptedObserver, StrictPooled, bind, config, credential_id, seen, strict_manager, tenant,
    },
    rate_limit::{Rate, RowLimit},
    resource::ResourceConfig as _,
    topology::pooled::config::WarmupStrategy,
};

// ── the fake owner ───────────────────────────────────────────────────────

/// One owner step, as the fake logged it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Prepare(String),
    Grant,
    Explain(Crossing),
    Settle(&'static str),
}

/// Where a fake slot stands (the ledger's phases).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Prepared,
    Outstanding(CallGrant),
    BeforeBoundary,
    Ambiguous,
    Resolved(RecordedOutcome),
    Unknown,
}

/// Resource, unit kind, operation, version, canonical request and key part:
/// what a resumed effect must present again (the contract the engine binds).
type Fingerprint = (String, UnitKind, String, u32, Vec<u8>, Option<String>);

#[derive(Debug)]
struct FakeSlot {
    id: [u8; 16],
    key: IdempotencyKey,
    fingerprint: Fingerprint,
    recovery: Recovery,
    max_invocations: u32,
    invocations: u32,
    prepared_at: Instant,
    phase: Phase,
}

/// What the fake saw of one prepare.
#[derive(Debug, Clone)]
struct SeenIntent {
    occurrence: String,
    kind: UnitKind,
    operation: String,
    version: u32,
    effect: Effect,
    recovery: Recovery,
    record_output: bool,
    canonical_request: Vec<u8>,
    key_part: Option<String>,
    max_invocations: u32,
    binding: SlotIdentity,
    config_fingerprint: u64,
}

#[derive(Default)]
struct FakeState {
    /// The budget every grant carries, when set.
    grant_budget: Option<Duration>,
    /// The next position of the owner's one sequence.
    next_ordinal: u32,
    /// The run prefix the owner labels its occurrences with, when set (as
    /// a stateful owner labels an iteration's).
    run_prefix: Option<String>,
    /// Occurrences units released, in order.
    released: Vec<String>,
    slots: HashMap<String, FakeSlot>,
    next_id: u8,
    log: Vec<Step>,
    intents: Vec<SeenIntent>,
    fail_prepare: Option<JournalRefusal>,
    fail_settle: Option<JournalRefusal>,
    fail_grant: Option<JournalRefusal>,
    on_grant: Option<Box<dyn FnOnce() + Send>>,
}

/// A deterministic in-memory owner.
#[derive(Default)]
struct FakeOwner {
    state: Mutex<FakeState>,
    closed: AtomicBool,
    in_flight: Arc<AtomicUsize>,
}

impl std::fmt::Debug for FakeOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("FakeOwner").finish_non_exhaustive()
    }
}

impl FakeOwner {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn log(&self) -> Vec<Step> {
        self.state().log.clone()
    }

    fn intents(&self) -> Vec<SeenIntent> {
        self.state().intents.clone()
    }

    fn phase(&self, occurrence: &str) -> Option<Phase> {
        self.state()
            .slots
            .get(occurrence)
            .map(|slot| slot.phase.clone())
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Restarts the ordinals, as the owner of a resumed execution does: its
    /// effects meet the slots the earlier run recorded.
    fn resume(&self) {
        self.state().next_ordinal = 0;
    }

    /// Starts a new positional run labelled `prefix`: its ordinals restart.
    fn begin_run(&self, prefix: &str) {
        let mut state = self.state();
        state.run_prefix = Some(prefix.to_owned());
        state.next_ordinal = 0;
    }

    fn fail_next_prepare(&self, refusal: JournalRefusal) {
        self.state().fail_prepare = Some(refusal);
    }

    fn fail_next_settle(&self, refusal: JournalRefusal) {
        self.state().fail_settle = Some(refusal);
    }

    fn grant_with_budget(&self, budget: Duration) {
        self.state().grant_budget = Some(budget);
    }

    fn fail_next_grant(&self, refusal: JournalRefusal) {
        self.state().fail_grant = Some(refusal);
    }

    fn on_next_grant(&self, hook: impl FnOnce() + Send + 'static) {
        self.state().on_grant = Some(Box::new(hook));
    }

    /// Records `phase` under `occurrence` as a finished earlier run of
    /// `operation` v1 with `request` on the fixture row did.
    fn seed(&self, occurrence: &str, operation: &str, request: &[u8], phase: Phase) {
        self.seed_unit(
            occurrence,
            (
                StrictPooled::key().to_string(),
                UnitKind::Operation,
                operation.to_owned(),
                1,
                request.to_vec(),
                None,
            ),
            phase,
        );
    }

    /// Records `phase` under `occurrence` for an earlier run's effect of
    /// `fingerprint`.
    fn seed_unit(&self, occurrence: &str, fingerprint: Fingerprint, phase: Phase) {
        let mut state = self.state();
        state.next_id += 1;
        let id = state.next_id;
        state.slots.insert(
            occurrence.to_owned(),
            FakeSlot {
                id: [id; 16],
                key: IdempotencyKey::new(&format!("key-{id}")).expect("key"),
                fingerprint,
                recovery: Recovery::Opaque,
                max_invocations: 1,
                invocations: 1,
                prepared_at: Instant::now(),
                phase,
            },
        );
    }

    fn slot_view(slot: &FakeSlot, phase: SlotPhase) -> JournalSlot {
        JournalSlot::new(slot.id, slot.key, 1, phase)
    }

    fn by_id<'s>(state: &'s mut FakeState, id: &[u8; 16]) -> Option<&'s mut FakeSlot> {
        state.slots.values_mut().find(|slot| slot.id == *id)
    }
}

#[async_trait::async_trait]
impl EffectJournal for FakeOwner {
    fn next_ordinal(&self) -> u32 {
        let mut state = self.state();
        let next = state.next_ordinal;
        state.next_ordinal += 1;
        next
    }

    fn next_occurrence(&self) -> String {
        let ordinal = self.next_ordinal();
        match self.state().run_prefix.clone() {
            Some(prefix) => format!("{prefix}/unit/v1/#{ordinal:06}"),
            None => occurrence(ordinal),
        }
    }

    fn release_occurrence(&self, occurrence: &str) {
        self.state().released.push(occurrence.to_owned());
    }

    async fn prepare(&self, intent: &JournalIntent<'_>) -> Result<JournalSlot, JournalRefusal> {
        if self.is_closed() {
            return Err(JournalRefusal::Closed);
        }
        let mut state = self.state();
        if let Some(refusal) = state.fail_prepare.take() {
            return Err(refusal);
        }
        state.log.push(Step::Prepare(intent.occurrence.to_owned()));
        state.intents.push(SeenIntent {
            occurrence: intent.occurrence.to_owned(),
            kind: intent.kind,
            operation: intent.operation.to_owned(),
            version: intent.version,
            effect: intent.effect,
            recovery: intent.recovery,
            record_output: intent.record_output,
            canonical_request: intent.canonical_request.to_vec(),
            key_part: intent.key_part.map(str::to_owned),
            max_invocations: intent.max_invocations.get(),
            binding: intent.binding.clone(),
            config_fingerprint: intent.config_fingerprint,
        });
        let fingerprint = (
            intent.resource_key.to_string(),
            intent.kind,
            intent.operation.to_owned(),
            intent.version,
            intent.canonical_request.to_vec(),
            intent.key_part.map(str::to_owned),
        );
        let now = Instant::now();
        if let Some(slot) = state.slots.get_mut(intent.occurrence) {
            if slot.fingerprint != fingerprint {
                return Err(JournalRefusal::Mismatch);
            }
            let phase = match (&slot.phase, slot.recovery) {
                (Phase::Resolved(outcome), _) => SlotPhase::Replay(outcome.clone()),
                (Phase::Prepared | Phase::BeforeBoundary, _) => SlotPhase::Runnable,
                (Phase::Ambiguous | Phase::Outstanding(_), Recovery::StableKey { window })
                    if now < slot.prepared_at + window =>
                {
                    slot.phase = Phase::Ambiguous;
                    SlotPhase::Runnable
                },
                _ => {
                    slot.phase = Phase::Unknown;
                    SlotPhase::Unknown
                },
            };
            return Ok(Self::slot_view(slot, phase));
        }
        state.next_id += 1;
        let id = state.next_id;
        let slot = FakeSlot {
            id: [id; 16],
            key: IdempotencyKey::new(&format!("key-{id}")).expect("key"),
            fingerprint,
            recovery: intent.recovery,
            max_invocations: intent.max_invocations.get(),
            invocations: 0,
            prepared_at: now,
            phase: Phase::Prepared,
        };
        let view = Self::slot_view(&slot, SlotPhase::Runnable);
        state.slots.insert(intent.occurrence.to_owned(), slot);
        Ok(view)
    }

    async fn grant(&self, slot: &JournalSlot) -> Result<CallGrant, JournalRefusal> {
        if self.is_closed() {
            return Err(JournalRefusal::Closed);
        }
        let hook = {
            let mut state = self.state();
            if let Some(refusal) = state.fail_grant.take() {
                return Err(refusal);
            }
            let now = Instant::now();
            let call_id = state.next_id.wrapping_add(100);
            state.next_id += 1;
            let budget = state.grant_budget;
            let fake = Self::by_id(&mut state, slot.id()).ok_or(JournalRefusal::Mismatch)?;
            let grantable = match (&fake.phase, fake.recovery) {
                (Phase::Prepared | Phase::BeforeBoundary, _) => true,
                (Phase::Ambiguous, Recovery::StableKey { window }) => {
                    now < fake.prepared_at + window
                },
                _ => false,
            };
            if !grantable || fake.invocations >= fake.max_invocations {
                fake.phase = Phase::Unknown;
                return Err(JournalRefusal::Unknown);
            }
            fake.invocations += 1;
            let call = CallGrant::from_bytes([call_id; 16]);
            let call = match budget {
                Some(budget) => call.with_budget(budget),
                None => call,
            };
            fake.phase = Phase::Outstanding(call);
            state.log.push(Step::Grant);
            (state.on_grant.take(), call)
        };
        let (hook, call) = hook;
        if let Some(hook) = hook {
            hook();
        }
        Ok(call)
    }

    async fn explain(
        &self,
        slot: &JournalSlot,
        call: CallGrant,
        crossing: Crossing,
    ) -> Result<(), JournalRefusal> {
        let mut state = self.state();
        let fake = Self::by_id(&mut state, slot.id()).ok_or(JournalRefusal::Mismatch)?;
        if fake.phase != Phase::Outstanding(call) {
            return Err(JournalRefusal::Mismatch);
        }
        fake.phase = match (crossing, fake.recovery) {
            (Crossing::NotCrossed, _) => Phase::BeforeBoundary,
            (Crossing::Ambiguous, Recovery::StableKey { .. }) => Phase::Ambiguous,
            _ => Phase::Unknown,
        };
        state.log.push(Step::Explain(crossing));
        Ok(())
    }

    async fn settle(
        &self,
        slot: &JournalSlot,
        call: CallGrant,
        outcome: CallOutcome<'_>,
    ) -> Result<(), JournalRefusal> {
        let mut state = self.state();
        if let Some(refusal) = state.fail_settle.take() {
            return Err(refusal);
        }
        let fake = Self::by_id(&mut state, slot.id()).ok_or(JournalRefusal::Mismatch)?;
        if fake.phase != Phase::Outstanding(call) {
            return Err(JournalRefusal::Mismatch);
        }
        let (recorded, step) = match outcome {
            CallOutcome::Applied(bytes) => (RecordedOutcome::Succeeded(bytes.to_vec()), "applied"),
            CallOutcome::AppliedWithoutOutput => {
                (RecordedOutcome::OutputUnavailable, "applied_without_output")
            },
            CallOutcome::Rejected(code) => (RecordedOutcome::Failed(code), code.as_str()),
        };
        fake.phase = Phase::Resolved(recorded);
        state.log.push(Step::Settle(step));
        Ok(())
    }

    fn track(&self) -> InFlight {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        let in_flight = Arc::clone(&self.in_flight);
        InFlight::new(move || {
            in_flight.fetch_sub(1, Ordering::SeqCst);
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

// ── operations ───────────────────────────────────────────────────────────

/// What one attempt of [`Pay`] does.
#[derive(Debug, Clone)]
enum Reply {
    /// Finished as answered; the unit yields the value.
    Ok(u64),
    /// Finished with the classified error; the unit fails with it when no
    /// attempt is left.
    Fail(OperationError),
    /// Dropped unsettled; the unit fails `Transient` when no attempt is
    /// left.
    Unsettled,
}

/// Provider calls made and the idempotency key each attempt saw.
#[derive(Debug, Default)]
struct Calls {
    made: AtomicUsize,
    keys: Mutex<Vec<Option<String>>>,
}

impl Calls {
    fn made(&self) -> usize {
        self.made.load(Ordering::SeqCst)
    }

    fn keys(&self) -> Vec<Option<String>> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn call(&self, key: Option<&IdempotencyKey>) {
        self.made.fetch_add(1, Ordering::SeqCst);
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.map(ToString::to_string));
    }
}

/// A deserialized test operation books nothing.
fn free() -> Cost {
    Cost::FREE
}

const PAY: &str = "billing.pay";
const REFUND: &str = "billing.refund";
const WINDOW: Duration = Duration::from_mins(1);

/// A payment: `Idempotent` with a one-minute key window when `IDEM`, a
/// `Write` otherwise; one attempt per reply. Its canonical request is
/// `{"request":…}`: everything else is not intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pay<const IDEM: bool> {
    request: String,
    #[serde(skip)]
    key_part: Option<&'static str>,
    #[serde(skip)]
    replies: Vec<Reply>,
    #[serde(skip, default = "free")]
    cost: Cost,
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<const IDEM: bool> Pay<IDEM> {
    fn new(calls: &Arc<Calls>, replies: Vec<Reply>) -> Self {
        Self {
            request: "pay:42".to_owned(),
            key_part: None,
            replies,
            cost: Cost::FREE,
            calls: Arc::clone(calls),
        }
    }

    fn keyed(mut self, part: &'static str) -> Self {
        self.key_part = Some(part);
        self
    }
}

impl<R: Provider + PinSlots, const IDEM: bool> Operation<R> for Pay<IDEM> {
    type Output = u64;
    const KEY: &'static str = PAY;
    const EFFECT: Effect = if IDEM {
        Effect::Idempotent
    } else {
        Effect::Write
    };
    const KEY_WINDOW: Duration = WINDOW;

    fn idempotency_key(&self) -> Option<String> {
        self.key_part.map(str::to_owned)
    }

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(u32::try_from(self.replies.len()).expect("few")).expect("a reply")
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        let last = self.replies.len() - 1;
        for (index, reply) in self.replies.into_iter().enumerate() {
            // The key is the same for every attempt; it is read while no
            // attempt borrows the context.
            let key = cx.idempotency_key().copied();
            let attempt = cx.attempt(self.cost.clone()).await?;
            self.calls.call(key.as_ref());
            match reply {
                Reply::Ok(value) => {
                    let result = Ok(value);
                    attempt.finish(&result).await;
                    return result;
                },
                Reply::Fail(error) => {
                    let result: Result<u64, OperationError> = Err(error);
                    attempt.finish(&result).await;
                    if index == last {
                        return result;
                    }
                },
                Reply::Unsettled => {
                    drop(attempt);
                    if index == last {
                        return Err(OperationError::new(
                            ErrorKind::Transient,
                            "connection reset",
                        ));
                    }
                },
            }
        }
        Err(OperationError::new(
            ErrorKind::Permanent,
            "no reply scripted",
        ))
    }
}

/// The canonical request of a [`Pay`] of `request`.
fn pay_request(request: &str) -> Vec<u8> {
    format!(r#"{{"request":"{request}"}}"#).into_bytes()
}

/// A `Write` of another operation, recorded as a digest only.
#[derive(Serialize, Deserialize)]
struct Refund {
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl Refund {
    fn new(calls: &Arc<Calls>) -> Self {
        Self {
            calls: Arc::clone(calls),
        }
    }
}

impl<R: Provider + PinSlots> Operation<R> for Refund {
    type Output = u64;
    const KEY: &'static str = REFUND;
    const RECORD_OUTPUT: bool = false;

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        let calls = self.calls;
        cx.call(Cost::FREE, async move |_, _| {
            calls.call(None);
            Ok(7)
        })
        .await
    }
}

/// A `Write` whose output is `len` bytes of JSON string.
#[derive(Serialize, Deserialize)]
struct Export {
    len: usize,
}

impl<R: Provider + PinSlots> Operation<R> for Export {
    type Output = String;
    const KEY: &'static str = "billing.export";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<String, OperationError> {
        let len = self.len;
        cx.call(Cost::FREE, async move |_, _| Ok("x".repeat(len)))
            .await
    }
}

/// A `Write` of a second version of [`PAY`].
#[derive(Serialize, Deserialize)]
struct PayV2 {
    request: String,
}

impl<R: Provider + PinSlots> Operation<R> for PayV2 {
    type Output = u64;
    const KEY: &'static str = PAY;
    const VERSION: u32 = 2;

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        cx.call(Cost::FREE, async |_, _| Ok(2)).await
    }
}

/// A `Write` whose key breaks the rules; submitted around the build-time
/// assert, it is refused at submit.
#[derive(Serialize, Deserialize)]
struct BadKey {
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<R: Provider + PinSlots> Operation<R> for BadKey {
    type Output = ();
    const KEY: &'static str = "bad key";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let calls = self.calls;
        cx.call(Cost::FREE, async move |_, _| {
            calls.call(None);
            Ok(())
        })
        .await
    }
}

/// A `Write` whose request has no JSON form (a map with non-string keys).
#[derive(Serialize, Deserialize)]
struct Opaque {
    by_pair: BTreeMap<Vec<u8>, u8>,
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<R: Provider + PinSlots> Operation<R> for Opaque {
    type Output = ();
    const KEY: &'static str = "billing.opaque";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let calls = self.calls;
        cx.call(Cost::FREE, async move |_, _| {
            calls.call(None);
            Ok(())
        })
        .await
    }
}

/// An effect made through [`OperationCx::call`]: each attempt answers the
/// next scripted result, within as many attempts as the script has.
#[derive(Serialize, Deserialize)]
struct Called<const IDEM: bool> {
    request: String,
    #[serde(skip)]
    script: Vec<Result<u64, OperationError>>,
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<const IDEM: bool> Called<IDEM> {
    fn new(calls: &Arc<Calls>, script: Vec<Result<u64, OperationError>>) -> Self {
        Self {
            request: "called:1".to_owned(),
            script,
            calls: Arc::clone(calls),
        }
    }
}

impl<R: Provider + PinSlots, const IDEM: bool> Operation<R> for Called<IDEM> {
    type Output = u64;
    const KEY: &'static str = "billing.called";
    const EFFECT: Effect = if IDEM {
        Effect::Idempotent
    } else {
        Effect::Write
    };

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(u32::try_from(self.script.len()).expect("few")).expect("a reply")
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        let mut script = std::collections::VecDeque::from(self.script);
        let calls = self.calls;
        cx.call(Cost::FREE, async move |_, _| {
            calls.call(None);
            script.pop_front().unwrap_or(Ok(0))
        })
        .await
    }
}

/// A `Write` whose call succeeds and whose response the unit then fails to
/// use with `local`: a local failure after the provider applied it.
#[derive(Serialize, Deserialize)]
struct AppliedThenFailed {
    #[serde(skip)]
    local: Option<OperationError>,
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<R: Provider + PinSlots> Operation<R> for AppliedThenFailed {
    type Output = u64;
    const KEY: &'static str = "billing.applied_then_failed";
    const EFFECT: Effect = Effect::Write;

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        let calls = self.calls;
        let receipt = cx
            .call(Cost::FREE, async move |_, _| {
                calls.call(None);
                Ok(7_u64)
            })
            .await?;
        match self.local {
            Some(local) => Err(local),
            None => Ok(receipt),
        }
    }
}

/// An `Idempotent` call the provider never answers.
#[derive(Serialize, Deserialize)]
struct Stall {
    #[serde(skip)]
    calls: Arc<Calls>,
}

impl<R: Provider + PinSlots> Operation<R> for Stall {
    type Output = u64;
    const KEY: &'static str = "billing.stall";
    const EFFECT: Effect = Effect::Idempotent;
    const KEY_WINDOW: Duration = WINDOW;

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
        let calls = self.calls;
        cx.call(Cost::FREE, async move |_, _| {
            calls.call(None);
            std::future::pending::<Result<u64, OperationError>>().await
        })
        .await
    }
}

/// A read.
#[derive(Serialize, Deserialize)]
struct Look {
    #[serde(skip)]
    calls: Arc<Calls>,
    #[serde(skip, default = "free")]
    cost: Cost,
    #[serde(skip)]
    key_part: Option<&'static str>,
}

impl Look {
    fn new(calls: &Arc<Calls>, cost: Cost) -> Self {
        Self {
            calls: Arc::clone(calls),
            cost,
            key_part: None,
        }
    }
}

impl<R: Provider + PinSlots> Operation<R> for Look {
    type Output = ();
    const KEY: &'static str = "billing.look";
    const EFFECT: Effect = Effect::Read;

    fn idempotency_key(&self) -> Option<String> {
        self.key_part.map(str::to_owned)
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let key = cx.idempotency_key().copied();
        let calls = self.calls;
        cx.call(self.cost, async move |_, _| {
            calls.call(key.as_ref());
            Ok(())
        })
        .await
    }
}

/// A stream of three items: a `Write` when `WRITE`, a read otherwise; its
/// attempt records the idempotency key it sees.
struct Ticks<const WRITE: bool>(Arc<Calls>, Option<&'static str>);

impl<R: Provider + PinSlots, const WRITE: bool> StreamOperation<R> for Ticks<WRITE> {
    type Item = u8;
    type Output = ();
    const KEY: &'static str = "billing.ticks";
    const EFFECT: Effect = if WRITE { Effect::Write } else { Effect::Read };

    fn idempotency_key(&self) -> Option<String> {
        self.1.map(str::to_owned)
    }

    async fn run(
        self,
        cx: &mut OperationCx<'_, R>,
        mut sink: StreamSink<u8>,
    ) -> Result<(), OperationError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        self.0.call(attempt.idempotency_key());
        attempt.finish(&Ok::<(), OperationError>(())).await;
        for tick in 0..3 {
            sink.send(tick).await?;
        }
        Ok(())
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────

struct Fixture {
    manager: Manager,
    resource: StrictPooled,
    observer: Arc<ScriptedObserver>,
    owner: Arc<FakeOwner>,
    parent: CancellationToken,
}

impl Fixture {
    /// A strict manager with a pooled credential-bound row, optionally
    /// rate limited.
    fn new(limit: Option<RowLimit>) -> Self {
        let observer = ScriptedObserver::answering(seen(1, 1, CredentialAvailability::Available));
        let metrics = Arc::new(MetricsRegistry::new());
        let manager = strict_manager(
            Arc::clone(&observer) as Arc<dyn nebula_credential::CredentialAvailabilityObserver>,
            &metrics,
        );
        let resource = StrictPooled::new();
        let pool = PoolConfig {
            min_size: 0,
            max_size: 2,
            idle_timeout: None,
            max_lifetime: None,
            warmup: WarmupStrategy::None,
            maintenance_interval: Duration::from_hours(1),
            ..PoolConfig::default()
        };
        manager
            .register(RegistrationSpec {
                resource: resource.clone(),
                config: config(1),
                scope: nebula_core::ScopeLevel::Global,
                slot_identity: tenant(),
                topology: Pooled::new(pool, config(1).fingerprint()),
                recovery_gate: None,
                rate_limit: limit,
            })
            .expect("register");
        bind(&resource.db, credential_id(), 1, 1);
        Self {
            manager,
            resource,
            observer,
            owner: FakeOwner::new(),
            parent: CancellationToken::new(),
        }
    }

    fn ctx(&self) -> ResourceContext {
        ResourceContext::minimal(Scope::default(), self.parent.clone())
    }

    fn owned(&self) -> ResourceHandle<StrictPooled> {
        let owner = Arc::clone(&self.owner) as Arc<dyn EffectJournal>;
        *self
            .manager
            .handle_any_journaled(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
                owner,
            )
            .expect("owned row")
            .downcast::<ResourceHandle<StrictPooled>>()
            .expect("typed row")
    }

    fn library(&self) -> ResourceHandle<StrictPooled> {
        *self
            .manager
            .handle_any(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
            )
            .expect("library row")
            .downcast::<ResourceHandle<StrictPooled>>()
            .expect("typed row")
    }

    fn read_only(&self) -> ResourceHandle<StrictPooled> {
        *self
            .manager
            .handle_any_read_only(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
            )
            .expect("read-only row")
            .downcast::<ResourceHandle<StrictPooled>>()
            .expect("typed row")
    }

    /// Attempts the facade granted and refused.
    fn attempts(&self) -> (u64, u64) {
        let snapshot = self.manager.metrics().expect("metrics").snapshot();
        (
            snapshot.call_attempts.granted,
            snapshot.call_attempts.refused,
        )
    }
}

/// The occurrence label of the owner's `ordinal`th effect unit, whatever
/// its resource, kind or operation.
fn occurrence(ordinal: u32) -> String {
    format!("unit/v1/#{ordinal:06}")
}

/// The occurrence label of the `ordinal`th effect unit, such as a [`Pay`].
fn pay(ordinal: u32) -> String {
    occurrence(ordinal)
}

fn assert_unsent(error: &OperationError, kind: &ErrorKind) {
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(error.sent(), SentState::NotSent, "{error}");
}

// ── compile gates ────────────────────────────────────────────────────────

#[test]
fn the_owner_is_object_safe_and_an_owned_row_crosses_threads() {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    let _: Option<Arc<dyn EffectJournal>> = None;
    send_sync_clone::<Arc<dyn EffectJournal>>();
    send_sync_clone::<ResourceHandle<StrictPooled>>();
}

// ── the derived declaration ──────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn the_intent_carries_the_derived_declaration() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    row.submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]).keyed("order-123"))
        .await
        .expect("keyed");
    row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(1), Reply::Ok(2)]))
        .await
        .expect("unkeyed");
    row.submit(Refund::new(&calls)).await.expect("refund");

    let intents = fixture.owner.intents();
    let keyed = &intents[0];
    assert_eq!(keyed.occurrence, pay(0));
    assert_eq!(keyed.kind, UnitKind::Operation);
    assert_eq!(keyed.operation, PAY);
    assert_eq!(keyed.version, 1);
    assert_eq!(keyed.effect, Effect::Idempotent);
    assert_eq!(keyed.recovery, Recovery::StableKey { window: WINDOW });
    assert!(keyed.record_output);
    assert_eq!(keyed.canonical_request, pay_request("pay:42"));
    assert_eq!(keyed.key_part.as_deref(), Some("order-123"));
    assert_eq!(keyed.max_invocations, 1);
    assert_eq!(keyed.binding, tenant());
    assert_eq!(keyed.config_fingerprint, config(1).fingerprint());

    let unkeyed = &intents[1];
    assert_eq!(unkeyed.occurrence, pay(1));
    assert_eq!(unkeyed.effect, Effect::Write);
    assert_eq!(unkeyed.recovery, Recovery::Opaque);
    assert_eq!(unkeyed.key_part, None);
    assert_eq!(unkeyed.max_invocations, 2);

    let refund = &intents[2];
    assert_eq!(
        refund.occurrence,
        pay(2),
        "the third operation on the row, whatever its key"
    );
    assert_eq!(refund.operation, REFUND);
    assert!(!refund.record_output, "RECORD_OUTPUT = false");
    assert_eq!(refund.canonical_request, b"{}", "every field skipped");
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Settle("applied_without_output"))
    );
}

/// An owner that only hands out ordinals: every durable step is refused.
#[derive(Debug, Default)]
struct OrdinalOnly(Mutex<u32>);

#[async_trait::async_trait]
impl EffectJournal for OrdinalOnly {
    fn next_ordinal(&self) -> u32 {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let ordinal = *next;
        *next += 1;
        ordinal
    }

    async fn prepare(&self, _: &JournalIntent<'_>) -> Result<JournalSlot, JournalRefusal> {
        Err(JournalRefusal::Closed)
    }

    async fn grant(&self, _: &JournalSlot) -> Result<CallGrant, JournalRefusal> {
        Err(JournalRefusal::Closed)
    }

    async fn explain(
        &self,
        _: &JournalSlot,
        _: CallGrant,
        _: Crossing,
    ) -> Result<(), JournalRefusal> {
        Err(JournalRefusal::Closed)
    }

    async fn settle(
        &self,
        _: &JournalSlot,
        _: CallGrant,
        _: CallOutcome<'_>,
    ) -> Result<(), JournalRefusal> {
        Err(JournalRefusal::Closed)
    }

    fn track(&self) -> InFlight {
        InFlight::new(|| {})
    }

    fn is_closed(&self) -> bool {
        true
    }
}

#[test]
fn the_default_occurrence_is_the_flat_positional_label() {
    let owner = OrdinalOnly::default();
    assert_eq!(owner.next_occurrence(), "unit/v1/#000000");
    assert_eq!(owner.next_occurrence(), "unit/v1/#000001");
    assert_eq!(owner.next_ordinal(), 2, "one sequence underneath");
}

#[tokio::test(start_paused = true)]
async fn an_owner_labels_each_run_and_a_slot_cap_refusal_sends_nothing() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    fixture.owner.begin_run("it0");
    row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect("first run");
    fixture.owner.begin_run("it1");
    row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(2)]))
        .await
        .expect("second run");
    let occurrences: Vec<String> = fixture
        .owner
        .intents()
        .into_iter()
        .map(|intent| intent.occurrence)
        .collect();
    assert_eq!(
        occurrences,
        ["it0/unit/v1/#000000", "it1/unit/v1/#000000"],
        "the owner's label, its ordinal restarted per run"
    );

    fixture
        .owner
        .fail_next_prepare(JournalRefusal::SlotCapExceeded);
    let refused = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(3)]))
        .await
        .expect_err("over the owner's cap");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(
        refused.detail(),
        "effect journal slot cap reached; unit refused"
    );
    assert_eq!(calls.made(), 2, "nothing sent past the cap");
}

#[tokio::test(start_paused = true)]
async fn every_position_handed_out_is_released_even_when_the_unit_gives_up() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    // Polled only past its deadline: the unit takes a position and gives up
    // before reaching the owner.
    let late = row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]));
    tokio::time::advance(crate::call::OPERATION_DEADLINE_CAP + Duration::from_secs(1)).await;
    let gave_up = late.await.expect_err("past its deadline");
    assert_unsent(&gave_up, &ErrorKind::Backpressure);
    assert!(
        fixture.owner.intents().is_empty(),
        "never reached the owner"
    );
    assert_eq!(fixture.owner.state().released, [pay(0)]);

    row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(2)]))
        .await
        .expect("prepared and run");
    assert_eq!(
        fixture.owner.state().released,
        [pay(0), pay(1)],
        "released once its prepare returned"
    );
    assert_eq!(calls.made(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_grant_budget_bounds_the_unit_and_a_spent_one_sends_nothing() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    // Nearly expired: the call is cut at the grant's budget, long before
    // the unit's own deadline.
    fixture.owner.grant_with_budget(Duration::from_secs(2));
    let started = Instant::now();
    let cut = row
        .submit(Stall {
            calls: Arc::clone(&calls),
        })
        .await
        .expect_err("cut at the budget");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_secs(3),
        "{elapsed:?}"
    );
    assert_eq!(cut.sent(), SentState::MaybeSent, "{cut}");
    assert_eq!(calls.made(), 1);
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Explain(Crossing::Ambiguous))
    );

    // Spent: the grant is explained not crossed and no call is made.
    fixture.owner.grant_with_budget(Duration::ZERO);
    let refused = row
        .submit(Called::<true>::new(&calls, vec![Ok(1)]))
        .await
        .expect_err("no budget left");
    assert_unsent(&refused, &ErrorKind::Backpressure);
    assert_eq!(calls.made(), 1, "no second call");
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Explain(Crossing::NotCrossed))
    );
}

// Real time: the registration blocks on the admission lock a thread holds.
#[tokio::test]
async fn a_grant_that_expires_during_registration_sends_nothing() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture.owner.grant_with_budget(Duration::from_millis(50));
    // While the owner grants, another thread takes the admission lock the
    // strict registration needs and holds it past the grant's budget.
    let lock = fixture.manager.admission_lock_for_tests();
    let (held, holding) = std::sync::mpsc::channel();
    let holder = Arc::new(Mutex::new(None));
    let holder_slot = Arc::clone(&holder);
    fixture.owner.on_next_grant(move || {
        let thread = std::thread::spawn(move || {
            let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            held.send(()).expect("signal");
            std::thread::sleep(Duration::from_millis(300));
        });
        holding.recv().expect("the lock is held");
        *holder_slot.lock().expect("holder") = Some(thread);
    });

    let refused = row
        .submit(Called::<false>::new(&calls, vec![Ok(1)]))
        .await
        .expect_err("the grant expired while the attempt registered");
    assert_unsent(&refused, &ErrorKind::Backpressure);
    assert_eq!(
        refused.detail(),
        "effect grant expired during registration; attempt refused"
    );
    assert_eq!(calls.made(), 0, "no provider call");
    assert_eq!(
        fixture.owner.log(),
        vec![
            Step::Prepare(pay(0)),
            Step::Grant,
            Step::Explain(Crossing::NotCrossed)
        ]
    );
    let thread = holder.lock().expect("holder").take();
    thread.expect("the holder ran").join().expect("joined");
}

#[tokio::test(start_paused = true)]
async fn a_reload_after_submit_refuses_the_grant_and_sends_nothing() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let unit = row.submit(Called::<true>::new(&calls, vec![Ok(1)]));
    assert_eq!(
        fixture
            .manager
            .reload_config::<StrictPooled>(config(2), &nebula_core::ScopeLevel::Global)
            .expect("reloaded"),
        crate::reload::ReloadOutcome::SwappedImmediately
    );
    let refused = unit.await.expect_err("the row points elsewhere now");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(calls.made(), 0);
    assert_eq!(
        fixture.owner.log(),
        vec![Step::Prepare(pay(0))],
        "prepared against the old configuration, never granted"
    );
    assert_eq!(
        fixture.owner.intents()[0].config_fingerprint,
        config(1).fingerprint()
    );

    // A unit submitted after the reload binds the new configuration.
    row.submit(Called::<true>::new(&calls, vec![Ok(2)]))
        .await
        .expect("runs");
    assert_eq!(
        fixture.owner.intents()[1].config_fingerprint,
        config(2).fingerprint()
    );
    assert_eq!(calls.made(), 1);
}

#[tokio::test(start_paused = true)]
async fn ordinals_are_one_sequence_in_prepare_order() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    // Ordinals are taken when a unit starts preparing and are positional:
    // operations, versions and sessions share the owner's one sequence.
    let first = row.submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]));
    let second = row.submit(Pay::<false>::new(&calls, vec![Reply::Ok(2)]));
    let refund = row.submit(Refund::new(&calls));
    let v2 = row.submit(PayV2 {
        request: "pay:42".to_owned(),
    });
    // A session takes the next position too.
    let session = row.session(
        SessionSpec::write(PAY, &"pay:42").cost(Cost::FREE),
        |tx, _cx| {
            Box::pin(async move {
                tx.pending += 1;
                Ok(0_u64)
            })
        },
    );
    second.await.expect("second");
    first.await.expect("first");
    refund.await.expect("refund");
    v2.await.expect("v2");
    session.await.expect("session");

    let mut seen: Vec<_> = fixture
        .owner
        .intents()
        .into_iter()
        .map(|intent| (intent.occurrence, intent.operation, intent.version))
        .collect();
    seen.sort();
    let mut expected = vec![
        (pay(0), PAY.to_owned(), 1),
        (pay(1), PAY.to_owned(), 1),
        (pay(2), REFUND.to_owned(), 1),
        (pay(3), PAY.to_owned(), 2),
        (occurrence(4), PAY.to_owned(), 1),
    ];
    expected.sort();
    assert_eq!(seen, expected);
}

/// A `Pay` of `request`.
fn pay_of(calls: &Arc<Calls>, request: &str) -> Pay<false> {
    Pay {
        request: request.to_owned(),
        ..Pay::new(calls, vec![Reply::Ok(1)])
    }
}

#[tokio::test(start_paused = true)]
async fn a_submission_dropped_before_its_first_poll_takes_no_position() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    // An earlier run settled `B` at the first position — the run in which
    // a branch built another submission and dropped it unpolled.
    fixture.owner.seed(
        &pay(0),
        PAY,
        &pay_request("pay:B"),
        Phase::Resolved(RecordedOutcome::Succeeded(b"7".to_vec())),
    );
    fixture.owner.resume();

    drop(row.submit(pay_of(&calls, "pay:dropped")));
    let replayed = row
        .submit(pay_of(&calls, "pay:B"))
        .await
        .expect("replayed at the first position");

    assert_eq!(replayed, 7, "the recorded output");
    assert_eq!(calls.made(), 0, "no second provider call");
    assert_eq!(
        fixture.owner.log(),
        vec![Step::Prepare(pay(0))],
        "the dropped submission never reached the owner"
    );
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn units_prepared_in_another_order_fail_safe() {
    // An earlier run settled `A` at #0 and `B` at #1; a resumed run polls
    // `B` first. Positions follow the prepare order, so `B` meets `A`'s
    // slot: a mismatch with nothing sent, never a fresh effect.
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    for (ordinal, request) in [(0, "pay:A"), (1, "pay:B")] {
        fixture.owner.seed(
            &pay(ordinal),
            PAY,
            &pay_request(request),
            Phase::Resolved(RecordedOutcome::Succeeded(b"1".to_vec())),
        );
    }
    fixture.owner.resume();
    let a = row.submit(pay_of(&calls, "pay:A"));
    let b = row.submit(pay_of(&calls, "pay:B"));
    let b = b.await.expect_err("B prepared at A's position");
    let a = a.await.expect_err("A prepared at B's position");
    for error in [&a, &b] {
        assert_unsent(error, &ErrorKind::Permanent);
        assert_eq!(error.detail(), "effect occurrence mismatch");
    }
    assert_eq!(calls.made(), 0, "nothing sent");

    // Identical intents are interchangeable: either order replays both.
    let fixture = Fixture::new(None);
    let row = fixture.owned();
    for ordinal in [0, 1] {
        fixture.owner.seed(
            &pay(ordinal),
            PAY,
            &pay_request("pay:same"),
            Phase::Resolved(RecordedOutcome::Succeeded(
                format!("{}", ordinal + 10).into_bytes(),
            )),
        );
    }
    fixture.owner.resume();
    let first = row.submit(pay_of(&calls, "pay:same"));
    let second = row.submit(pay_of(&calls, "pay:same"));
    assert_eq!(second.await.expect("replayed"), 10);
    assert_eq!(first.await.expect("replayed"), 11);
    assert_eq!(calls.made(), 0, "nothing sent");
}

#[tokio::test(start_paused = true)]
async fn effects_of_other_kinds_or_resources_reordered_fail_safe() {
    // An earlier run settled an operation first; the resumed path opens a
    // session first. One node-wide sequence: the session meets the
    // operation's slot — a mismatch, its body never runs.
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture.owner.seed(
        &occurrence(0),
        PAY,
        &pay_request("pay:A"),
        Phase::Resolved(RecordedOutcome::Succeeded(b"1".to_vec())),
    );
    fixture.owner.resume();
    let opened = Arc::new(AtomicBool::new(false));
    let body_opened = Arc::clone(&opened);
    let refused = row
        .session(
            SessionSpec::write(PAY, &"pay:A").cost(Cost::FREE),
            move |tx, _cx| {
                body_opened.store(true, Ordering::SeqCst);
                Box::pin(async move {
                    tx.pending += 1;
                    Ok(0_u64)
                })
            },
        )
        .await
        .expect_err("a session where an operation was recorded");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(refused.detail(), "effect occurrence mismatch");
    assert!(!opened.load(Ordering::SeqCst), "no session opened");

    // An earlier run's first effect was on another resource; the resumed
    // path reaches this row first: a mismatch, nothing sent.
    let fixture = Fixture::new(None);
    let row = fixture.owned();
    fixture.owner.seed_unit(
        &occurrence(0),
        (
            "billing.other".to_owned(),
            UnitKind::Operation,
            PAY.to_owned(),
            1,
            pay_request("pay:A"),
            None,
        ),
        Phase::Resolved(RecordedOutcome::Succeeded(b"1".to_vec())),
    );
    fixture.owner.resume();
    let refused = row
        .submit(pay_of(&calls, "pay:A"))
        .await
        .expect_err("an effect recorded for another resource");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(refused.detail(), "effect occurrence mismatch");

    // The same order replays.
    let fixture = Fixture::new(None);
    let row = fixture.owned();
    fixture.owner.seed(
        &occurrence(0),
        PAY,
        &pay_request("pay:A"),
        Phase::Resolved(RecordedOutcome::Succeeded(b"1".to_vec())),
    );
    fixture.owner.resume();
    assert_eq!(
        row.submit(pay_of(&calls, "pay:A")).await.expect("replayed"),
        1
    );
    assert_eq!(calls.made(), 0, "nothing sent");
}

#[tokio::test(start_paused = true)]
async fn a_changed_operation_at_a_recorded_position_is_a_mismatch() {
    // A settled `Pay` v1 at the row's first position; a redeploy (with no
    // action-version bump) then submits another operation — or the same key
    // at another version — first. The position is the occurrence, the
    // operation its contract: a mismatch, nothing sent, never a fresh
    // effect.
    for drifted in ["version", "operation"] {
        let fixture = Fixture::new(None);
        let calls = Arc::new(Calls::default());
        let row = fixture.owned();
        fixture.owner.seed(
            &pay(0),
            PAY,
            &pay_request("pay:42"),
            Phase::Resolved(RecordedOutcome::Succeeded(b"99".to_vec())),
        );
        let error = if drifted == "version" {
            row.submit(PayV2 {
                request: "pay:42".to_owned(),
            })
            .await
            .expect_err("another version under the recorded occurrence")
        } else {
            row.submit(Refund::new(&calls))
                .await
                .expect_err("another operation under the recorded occurrence")
        };
        assert_unsent(&error, &ErrorKind::Permanent);
        assert_eq!(error.detail(), "effect occurrence mismatch", "{drifted}");
        assert_eq!(calls.made(), 0, "{drifted}");
        assert_eq!(
            fixture.owner.log(),
            vec![Step::Prepare(pay(0))],
            "{drifted}: prepared under the recorded occurrence, never granted"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn an_invalid_key_part_or_request_is_refused_at_submit() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let part = row
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]).keyed("has space"))
        .await
        .expect_err("not visible ASCII");
    assert_unsent(&part, &ErrorKind::Permanent);
    assert_eq!(
        part.detail(),
        "idempotency key part must be 1..=256 bytes of visible ASCII"
    );

    let opaque = || Opaque {
        by_pair: BTreeMap::from([(vec![1], 1)]),
        calls: Arc::clone(&calls),
    };
    let request = row.submit(opaque()).await.expect_err("no JSON form");
    assert_unsent(&request, &ErrorKind::Permanent);
    assert_eq!(
        request.detail(),
        "operation request does not serialize to JSON"
    );
    assert_eq!(calls.made(), 0);
    assert!(fixture.owner.log().is_empty(), "nothing reached the owner");
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);

    // Unjournaled, the request is never canonicalized.
    fixture
        .library()
        .submit(opaque())
        .await
        .expect("a library row runs it");
    assert_eq!(calls.made(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_invalid_operation_key_is_refused_at_submit() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    for row in [fixture.owned(), fixture.library()] {
        // `submit` would fail the build; the runtime refuses the same
        // declaration when it gets past it.
        let error = submit_unit(
            row.unit_host(),
            row.unit_scope(),
            Plain(BadKey {
                calls: Arc::clone(&calls),
            }),
        )
        .await
        .expect_err("refused");
        assert_unsent(&error, &ErrorKind::Permanent);
        assert!(error.detail().starts_with("operation key must be"));
    }
    assert_eq!(calls.made(), 0);
    assert!(fixture.owner.log().is_empty());
}

// ── replay ───────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_recorded_success_replays_without_quota_checkout_read_or_call() {
    let fixture = Fixture::new(Some(RowLimit::rate(
        Rate::per_second(NonZeroU32::MIN)
            .with_burst(NonZeroU32::MIN)
            .expect("rate"),
    )));
    let calls = Arc::new(Calls::default());
    fixture.owner.seed(
        &pay(0),
        PAY,
        &pay_request("pay:42"),
        Phase::Resolved(RecordedOutcome::Succeeded(b"99".to_vec())),
    );
    let row = fixture.owned();

    let replayed = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect("replayed");

    assert_eq!(replayed, 99, "the recorded output, not a new call");
    assert_eq!(calls.made(), 0, "no provider call");
    assert_eq!(
        fixture.attempts(),
        (0, 0),
        "no attempt, so no quota booking"
    );
    assert_eq!(fixture.resource.probe.creates(), 0, "no checkout");
    assert_eq!(fixture.observer.calls(), 0, "no credential read");
    assert_eq!(fixture.owner.log(), vec![Step::Prepare(pay(0))]);
    // The quota's only permit is still there: a read books it at once.
    let started = Instant::now();
    row.submit(Look::new(&calls, Cost::ONE))
        .await
        .expect("read");
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn recorded_rejections_and_digests_replay_without_a_call() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    let request = pay_request("pay:42");

    fixture.owner.seed(
        &pay(0),
        PAY,
        &request,
        Phase::Resolved(RecordedOutcome::Failed(ErrorKindCode::Permanent)),
    );
    let rejected = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("the recorded rejection");
    assert_eq!(*rejected.kind(), ErrorKind::Permanent);
    assert_eq!(rejected.sent(), SentState::Sent);
    assert!(!rejected.is_retryable());

    // A retryable code is final once recorded.
    fixture.owner.seed(
        &pay(1),
        PAY,
        &request,
        Phase::Resolved(RecordedOutcome::Failed(ErrorKindCode::Transient)),
    );
    let final_error = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("recorded");
    assert!(!final_error.is_retryable());

    fixture.owner.seed(
        &pay(2),
        PAY,
        &request,
        Phase::Resolved(RecordedOutcome::OutputUnavailable),
    );
    let digest = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("recorded without output");
    assert_eq!(*digest.kind(), ErrorKind::Permanent);
    assert_eq!(digest.detail(), "effect recorded without output");
    assert_eq!(digest.sent(), SentState::Sent);

    assert_eq!(calls.made(), 0);
    assert_eq!(fixture.attempts(), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn a_success_and_a_rejection_are_recorded_and_replayed() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let paid = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(5)]))
        .await
        .expect("paid");
    assert_eq!(paid, 5);
    let rejected = row
        .submit(Pay::<false>::new(
            &calls,
            vec![Reply::Fail(OperationError::rejected("declined"))],
        ))
        .await
        .expect_err("declined");
    assert_eq!(*rejected.kind(), ErrorKind::Permanent);
    // A digest-only effect records no output and replays none.
    let refunded = row.submit(Refund::new(&calls)).await.expect("refunded");
    assert_eq!(refunded, 7);

    // The resumed execution meets every recorded slot in program order.
    fixture.owner.resume();
    let again = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(6)]))
        .await
        .expect("replayed");
    assert_eq!(again, 5, "the first output replays");
    let replayed = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(6)]))
        .await
        .expect_err("the rejection replays");
    assert_eq!(*replayed.kind(), ErrorKind::Permanent);
    assert_eq!(replayed.sent(), SentState::Sent);
    let digest = row
        .submit(Refund::new(&calls))
        .await
        .expect_err("recorded without output");
    assert_eq!(digest.detail(), "effect recorded without output");

    assert_eq!(calls.made(), 3, "one call per first run, none per replay");
    let settled: Vec<_> = fixture
        .owner
        .log()
        .into_iter()
        .filter(|step| matches!(step, Step::Settle(_)))
        .collect();
    assert_eq!(
        settled,
        vec![
            Step::Settle("applied"),
            Step::Settle("permanent"),
            Step::Settle("applied_without_output"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn an_output_over_the_recording_cap_is_recorded_digest_only() {
    let fixture = Fixture::new(None);
    let row = fixture.owned();

    // 1 MiB of characters is 1 MiB + 2 bytes of JSON.
    let exported = row
        .submit(Export { len: 1024 * 1024 })
        .await
        .expect("the unit still yields its output");
    assert_eq!(exported.len(), 1024 * 1024);
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Settle("applied_without_output"))
    );
    let fits = row
        .submit(Export { len: 16 })
        .await
        .expect("a small output");
    assert_eq!(fits.len(), 16);
    assert_eq!(fixture.owner.log().last(), Some(&Step::Settle("applied")));

    fixture.owner.resume();
    let replay = row
        .submit(Export { len: 1024 * 1024 })
        .await
        .expect_err("recorded without output");
    assert_eq!(*replay.kind(), ErrorKind::Permanent);
    assert_eq!(replay.detail(), "effect recorded without output");
}

// ── unknown outcomes ─────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn an_unsettled_write_makes_the_outcome_unknown_for_every_later_unit() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let error = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Unsettled]))
        .await
        .expect_err("the connection reset");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(error.is_outcome_unknown());
    assert_eq!(fixture.owner.phase(&pay(0)), Some(Phase::Unknown));

    fixture.owner.resume();
    let blocked = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("the outcome is unknown");
    assert_eq!(*blocked.kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(blocked.sent(), SentState::MaybeSent);
    assert_eq!(calls.made(), 1, "the second unit made no call");
    assert_eq!(
        crate::Error::from(blocked).kind(),
        &ErrorKind::OutcomeUnknown
    );
    assert_eq!(
        fixture.owner.log(),
        vec![
            Step::Prepare(pay(0)),
            Step::Grant,
            Step::Explain(Crossing::Ambiguous),
            Step::Prepare(pay(0)),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_write_retried_after_an_interrupted_attempt_is_refused_outcome_unknown() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let error = row
        .submit(Pay::<false>::new(
            &calls,
            vec![
                Reply::Fail(OperationError::interrupted("connection reset")),
                Reply::Ok(1),
            ],
        ))
        .await
        .expect_err("the owner refuses the retry");
    assert_eq!(*error.kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(calls.made(), 1);
    assert_eq!(
        fixture.owner.log()[1..],
        [Step::Grant, Step::Explain(Crossing::Ambiguous)]
    );
}

#[tokio::test(start_paused = true)]
async fn an_unsent_write_attempt_is_not_crossed_and_granted_again() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let paid = row
        .submit(Pay::<false>::new(
            &calls,
            vec![
                Reply::Fail(OperationError::unreachable("no connection")),
                Reply::Ok(3),
            ],
        ))
        .await
        .expect("the unsent attempt did not cross");
    assert_eq!(paid, 3);
    assert_eq!(
        fixture.owner.log()[1..],
        [
            Step::Grant,
            Step::Explain(Crossing::NotCrossed),
            Step::Grant,
            Step::Settle("applied"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_call_explains_each_attempt_from_its_classification() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let paid = row
        .submit(Called::<true>::new(
            &calls,
            vec![
                Err(OperationError::throttled(None)),
                Err(OperationError::unreachable("no connection")),
                Err(OperationError::interrupted("connection reset")),
                Ok(9),
            ],
        ))
        .await
        .expect("the fourth attempt applied");
    assert_eq!(paid, 9);
    assert_eq!(calls.made(), 4);
    assert_eq!(
        fixture.owner.log()[1..],
        [
            Step::Grant,
            Step::Explain(Crossing::NotCrossed),
            Step::Grant,
            Step::Explain(Crossing::NotCrossed),
            Step::Grant,
            Step::Explain(Crossing::Ambiguous),
            Step::Grant,
            Step::Settle("applied"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn an_owner_refusing_a_retry_ends_the_call_with_its_refusal() {
    // The stable-key window expired during the throttle's pause, the owner
    // closed, the slot no longer matches: the refusal is authoritative, never
    // masked by the retryable throttle the retry followed.
    let cases = [
        (JournalRefusal::Unknown, ErrorKind::OutcomeUnknown),
        (JournalRefusal::Closed, ErrorKind::Cancelled),
        (JournalRefusal::Mismatch, ErrorKind::Permanent),
        (JournalRefusal::Unavailable, ErrorKind::Backpressure),
    ];
    for (refusal, kind) in cases {
        let fixture = Fixture::new(None);
        let calls = Arc::new(Calls::default());
        let owner = Arc::clone(&fixture.owner);
        fixture
            .owner
            .on_next_grant(move || owner.fail_next_grant(refusal));
        let error = fixture
            .owned()
            .submit(Called::<true>::new(
                &calls,
                vec![Err(OperationError::throttled(None)), Ok(9)],
            ))
            .await
            .expect_err("the owner refused the retry");
        assert_eq!(*error.kind(), kind, "{refusal:?}: {error}");
        assert_ne!(error.detail(), "provider throttled the call", "{refusal:?}");
        assert_eq!(calls.made(), 1, "{refusal:?}");
        assert_eq!(
            fixture.owner.log()[1..],
            [Step::Grant, Step::Explain(Crossing::NotCrossed)],
            "{refusal:?}"
        );
    }
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let owner = Arc::clone(&fixture.owner);
    fixture
        .owner
        .on_next_grant(move || owner.fail_next_grant(JournalRefusal::Unknown));
    let error = fixture
        .owned()
        .submit(Called::<true>::new(
            &calls,
            vec![Err(OperationError::throttled(None)), Ok(9)],
        ))
        .await
        .expect_err("unknown");
    assert!(
        !error.is_retryable(),
        "a resubmission could apply the effect twice: {error}"
    );

    // A local refusal of the retry — the throttle's pause lands past the
    // unit deadline — still ends the call with the attempt it would have
    // retried.
    let fixture = Fixture::new(Some(RowLimit::rate(
        Rate::per_second(NonZeroU32::MIN)
            .with_burst(NonZeroU32::MIN)
            .expect("rate"),
    )));
    let calls = Arc::new(Calls::default());
    let error = fixture
        .owned()
        .submit(Called::<true>::new(
            &calls,
            vec![
                Err(OperationError::throttled(Some(Duration::from_mins(1)))),
                Ok(9),
            ],
        ))
        .with_deadline(Instant::now().into_std() + Duration::from_secs(2))
        .await
        .expect_err("throttled");
    assert_eq!(error.detail(), "provider throttled the call");
    assert_eq!(calls.made(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_owner_refusing_a_retry_after_a_throttle_settles_nothing_crossed() {
    // The throttle applied nothing and the refused retry was never granted:
    // a `Write` settles not sent, its owner's refusal retryable as it is,
    // never turned into an unknown outcome.
    for refusal in [
        JournalRefusal::Unavailable,
        JournalRefusal::AcknowledgementUnknown,
        JournalRefusal::LeaseLost,
    ] {
        let fixture = Fixture::new(None);
        let calls = Arc::new(Calls::default());
        let owner = Arc::clone(&fixture.owner);
        fixture
            .owner
            .on_next_grant(move || owner.fail_next_grant(refusal));
        let error = fixture
            .owned()
            .submit(Called::<false>::new(
                &calls,
                vec![Err(OperationError::throttled(None)), Ok(9)],
            ))
            .await
            .expect_err("the owner refused the retry");
        assert_eq!(*error.kind(), ErrorKind::Backpressure, "{refusal:?}");
        assert_eq!(error.sent(), SentState::NotSent, "{refusal:?}");
        assert!(error.is_retryable(), "{refusal:?}: {error}");
        assert_eq!(
            *crate::Error::from(error).kind(),
            ErrorKind::Backpressure,
            "{refusal:?}"
        );
        assert_eq!(calls.made(), 1, "{refusal:?}");
    }

    // The owner's own unknown outcome stays unknown.
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let owner = Arc::clone(&fixture.owner);
    fixture
        .owner
        .on_next_grant(move || owner.fail_next_grant(JournalRefusal::Unknown));
    let error = fixture
        .owned()
        .submit(Called::<false>::new(
            &calls,
            vec![Err(OperationError::throttled(None)), Ok(9)],
        ))
        .await
        .expect_err("unknown");
    assert_eq!(*error.kind(), ErrorKind::OutcomeUnknown);
    assert!(!error.is_retryable(), "{error}");

    // The same for an idempotent effect.
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let owner = Arc::clone(&fixture.owner);
    fixture
        .owner
        .on_next_grant(move || owner.fail_next_grant(JournalRefusal::Unavailable));
    let error = fixture
        .owned()
        .submit(Called::<true>::new(
            &calls,
            vec![Err(OperationError::throttled(None)), Ok(9)],
        ))
        .await
        .expect_err("the owner refused the retry");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(error.is_retryable(), "{error}");

    // An earlier attempt that may have crossed still counts: interrupted,
    // then throttled, then the owner refuses the third attempt.
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let first = Arc::clone(&fixture.owner);
    fixture.owner.on_next_grant(move || {
        let second = Arc::clone(&first);
        first.on_next_grant(move || second.fail_next_grant(JournalRefusal::Unavailable));
    });
    let error = fixture
        .owned()
        .submit(Called::<true>::new(
            &calls,
            vec![
                Err(OperationError::interrupted("connection lost")),
                Err(OperationError::throttled(None)),
                Ok(9),
            ],
        ))
        .await
        .expect_err("the owner refused the third attempt");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(calls.made(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_local_failure_after_an_applied_call_never_records_a_rejection() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    // The response does not decode: non-retryable, after the provider
    // applied the call. Recorded applied (without output), not rejected.
    let local = row
        .submit(AppliedThenFailed {
            local: Some(OperationError::new(
                ErrorKind::Permanent,
                "response did not decode",
            )),
            calls: Arc::clone(&calls),
        })
        .await
        .expect_err("the local failure");
    assert_eq!(local.detail(), "response did not decode");
    assert_eq!(calls.made(), 1);
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Settle("applied_without_output"))
    );

    // A resume replays the applied effect without sending it again.
    fixture.owner.resume();
    let replayed = row
        .submit(AppliedThenFailed {
            local: None,
            calls: Arc::clone(&calls),
        })
        .await
        .expect_err("recorded without output");
    assert_eq!(replayed.detail(), "effect recorded without output");
    assert_eq!(replayed.sent(), SentState::Sent);
    assert_eq!(calls.made(), 1, "never sent again");

    // A retryable local failure stays an ambiguous crossing.
    let fixture = Fixture::new(None);
    let row = fixture.owned();
    row.submit(AppliedThenFailed {
        local: Some(OperationError::new(ErrorKind::Transient, "decoder busy")),
        calls: Arc::clone(&calls),
    })
    .await
    .expect_err("the local failure");
    assert_eq!(
        fixture.owner.log().last(),
        Some(&Step::Explain(Crossing::Ambiguous))
    );
}

#[tokio::test(start_paused = true)]
async fn the_last_call_is_recorded_from_its_classification() {
    // (reply, recorded, sent)
    let cases = [
        (
            OperationError::rejected_as(ErrorKind::NotFound, "no such order"),
            Step::Settle("not_found"),
            SentState::Sent,
        ),
        (
            OperationError::rejected("declined"),
            Step::Settle("permanent"),
            SentState::Sent,
        ),
        (
            OperationError::throttled(None),
            Step::Explain(Crossing::NotCrossed),
            SentState::Sent,
        ),
        (
            OperationError::unreachable("no connection"),
            Step::Explain(Crossing::NotCrossed),
            SentState::NotSent,
        ),
        (
            OperationError::interrupted("connection reset"),
            Step::Explain(Crossing::Ambiguous),
            SentState::MaybeSent,
        ),
        (
            // Unclassified: the call may have crossed, whatever its kind.
            OperationError::new(ErrorKind::Permanent, "client error"),
            Step::Explain(Crossing::Ambiguous),
            SentState::MaybeSent,
        ),
    ];
    for (reply, recorded, sent) in cases {
        let fixture = Fixture::new(None);
        let calls = Arc::new(Calls::default());
        let error = fixture
            .owned()
            .submit(Called::<false>::new(&calls, vec![Err(reply.clone())]))
            .await
            .expect_err("the call failed");
        assert_eq!(
            fixture.owner.log()[1..],
            [Step::Grant, recorded],
            "{reply:?}"
        );
        assert_eq!(error.sent(), sent, "{reply:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_interrupted_write_call_is_ambiguous_and_never_sent_again() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let error = row
        .submit(Called::<false>::new(
            &calls,
            vec![Err(OperationError::interrupted("connection reset")), Ok(1)],
        ))
        .await
        .expect_err("not retried");
    assert_eq!(calls.made(), 1);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(*crate::Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(
        fixture.owner.log()[1..],
        [Step::Grant, Step::Explain(Crossing::Ambiguous)]
    );
}

#[tokio::test(start_paused = true)]
async fn settle_without_acknowledgement_after_the_apply_is_outcome_unknown() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture
        .owner
        .fail_next_settle(JournalRefusal::AcknowledgementUnknown);

    let error = row
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("applied, but not recorded");
    assert_eq!(*error.kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(error.sent(), SentState::Sent);
    assert!(!error.is_retryable());
    assert_eq!(calls.made(), 1);
}

// ── stable keys ──────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn an_ambiguous_idempotent_attempt_is_granted_again_with_the_same_key() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let paid = row
        .submit(Pay::<true>::new(
            &calls,
            vec![Reply::Unsettled, Reply::Ok(8)],
        ))
        .await
        .expect("the retry lands");
    assert_eq!(paid, 8);
    let keys = calls.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys[0].is_some(), "an owned unit has an idempotency key");
    assert_eq!(keys[0], keys[1], "every attempt presents the same key");
    assert_eq!(
        fixture.owner.log()[1..],
        [
            Step::Grant,
            Step::Explain(Crossing::Ambiguous),
            Step::Grant,
            Step::Settle("applied"),
        ]
    );

    // An exhausted budget makes the outcome unknown.
    let exhausted = row
        .submit(Pay::<true>::new(&calls, vec![Reply::Unsettled]))
        .await
        .expect_err("ambiguous");
    assert_eq!(exhausted.sent(), SentState::MaybeSent);
    assert_eq!(fixture.owner.phase(&pay(1)), Some(Phase::Ambiguous));
    fixture.owner.resume();
    row.submit(Pay::<true>::new(&calls, vec![Reply::Ok(8)]))
        .await
        .expect("the first unit replays");
    let refused = row
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("no invocation left");
    assert_eq!(*refused.kind(), ErrorKind::OutcomeUnknown);
}

// ── refusals ─────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn prepare_refusals_map_to_unsent_errors_without_a_call() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    for refusal in [
        JournalRefusal::AcknowledgementUnknown,
        JournalRefusal::Unavailable,
        JournalRefusal::LeaseLost,
    ] {
        fixture.owner.fail_next_prepare(refusal);
        let error = row
            .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
            .await
            .expect_err("refused");
        assert_unsent(&error, &ErrorKind::Backpressure);
        assert!(error.is_retryable());
    }
    fixture.owner.fail_next_prepare(JournalRefusal::Closed);
    let closed = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("closed");
    assert_unsent(&closed, &ErrorKind::Cancelled);

    assert_eq!(calls.made(), 0);
    assert_eq!(fixture.attempts(), (0, 0));
    assert_eq!(fixture.resource.probe.creates(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_different_request_under_an_occurrence_is_a_permanent_mismatch() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture
        .owner
        .seed(&pay(0), PAY, &pay_request("pay:41"), Phase::Prepared);

    let error = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("another request under the occurrence");
    assert_unsent(&error, &ErrorKind::Permanent);
    assert_eq!(error.detail(), "effect occurrence mismatch");
    assert_eq!(calls.made(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_registration_refused_after_the_grant_is_explained_not_crossed() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    // The parent cancel lands while the owner grants: the unit's own grant
    // then refuses it, after the owner's.
    let parent = fixture.parent.clone();
    fixture.owner.on_next_grant(move || parent.cancel());

    let error = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("cancelled at registration");
    assert_unsent(&error, &ErrorKind::Cancelled);
    assert_eq!(calls.made(), 0);
    assert_eq!(
        fixture.owner.log()[1..],
        [Step::Grant, Step::Explain(Crossing::NotCrossed)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_cancel_before_the_first_grant_leaves_only_the_prepare() {
    let fixture = Fixture::new(Some(RowLimit::rate(
        Rate::per_second(NonZeroU32::MIN)
            .with_burst(NonZeroU32::MIN)
            .expect("rate"),
    )));
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    // A read takes the only permit.
    row.submit(Look::new(&calls, Cost::ONE))
        .await
        .expect("read");

    let mut pay = Pay::<false>::new(&calls, vec![Reply::Ok(1)]);
    pay.cost = Cost::ONE;
    let mut unit = row.submit(pay);
    assert!(futures::poll!(&mut unit).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    unit.cancel();
    let error = unit.await.expect_err("cancelled in the quota wait");
    assert_unsent(&error, &ErrorKind::Cancelled);
    assert_eq!(fixture.owner.log().len(), 1);
    assert!(matches!(fixture.owner.log()[0], Step::Prepare(_)));
}

#[tokio::test(start_paused = true)]
async fn a_closed_owner_refuses_new_units_and_smuggled_clones() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    let clone = row.clone();
    fixture.owner.close();

    for row in [row, clone] {
        let error = row
            .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
            .await
            .expect_err("the owner closed");
        assert_unsent(&error, &ErrorKind::Cancelled);
    }
    assert_eq!(calls.made(), 0);
    assert!(fixture.owner.log().is_empty());
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

// ── routing ──────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_read_on_a_journaled_row_is_never_prepared_and_keys_locally() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    row.submit(Look::new(&calls, Cost::FREE))
        .await
        .expect("a read runs");
    let mut keyed = Look::new(&calls, Cost::FREE);
    keyed.key_part = Some("look-1");
    row.submit(keyed).await.expect("a keyed read runs");

    assert_eq!(calls.made(), 2);
    let local = local_idempotency_key(&StrictPooled::key(), "billing.look", 1, "look-1")
        .expect("local key");
    assert_eq!(calls.keys(), vec![None, Some(local.to_string())]);
    assert!(fixture.owner.log().is_empty(), "never prepared");
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_library_row_runs_effects_plain_and_a_read_only_row_refuses_them() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let library = fixture.library();

    let paid = library
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect("a library effect");
    assert_eq!(paid, 4);
    library
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(4)]).keyed("order-1"))
        .await
        .expect("a keyed library effect");
    let local = local_idempotency_key(&StrictPooled::key(), PAY, 1, "order-1").expect("local key");
    assert_eq!(
        calls.keys(),
        vec![None, Some(local.to_string())],
        "no owner: no key, or the local one"
    );

    let read_only = fixture.read_only();
    let refused = read_only
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect_err("no effect authority");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(
        refused.detail(),
        "managed row effect requires execution-owner authority"
    );
    let idempotent = read_only
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect_err("idempotent too");
    assert_unsent(&idempotent, &ErrorKind::Permanent);
    read_only
        .submit(Look::new(&calls, Cost::FREE))
        .await
        .expect("a read runs");
    assert_eq!(calls.made(), 3);
    assert!(fixture.owner.log().is_empty());

    let explained = *fixture
        .manager
        .handle_any_read_only_because(
            &StrictPooled::key(),
            &fixture.ctx(),
            &AcquireOptions::default(),
            &tenant(),
            "journaled effects need execution stores",
        )
        .expect("read-only row")
        .downcast::<ResourceHandle<StrictPooled>>()
        .expect("typed row");
    let refused = explained
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect_err("no effect authority, with its reason");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(refused.detail(), "journaled effects need execution stores");
    explained
        .submit(Look::new(&calls, Cost::FREE))
        .await
        .expect("a read runs");
    assert_eq!(calls.made(), 4);
    assert!(fixture.owner.log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn streamed_effects_are_refused_on_a_journaled_row() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let capacity = NonZeroUsize::MIN;

    let refused = fixture
        .owned()
        .submit_streaming(Ticks::<true>(Arc::clone(&calls), None), capacity)
        .finish()
        .await
        .expect_err("not journaled in v1");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(
        refused.detail(),
        "streaming effects are not journaled in v1"
    );
    let read_only = fixture
        .read_only()
        .submit_streaming(Ticks::<true>(Arc::clone(&calls), None), capacity)
        .finish()
        .await
        .expect_err("no effect authority");
    assert_unsent(&read_only, &ErrorKind::Permanent);
    assert_eq!(calls.made(), 0);

    fixture
        .owned()
        .submit_streaming(Ticks::<false>(Arc::clone(&calls), None), capacity)
        .finish()
        .await
        .expect("a streamed read runs");
    fixture
        .library()
        .submit_streaming(Ticks::<true>(Arc::clone(&calls), None), capacity)
        .finish()
        .await
        .expect("a library stream runs");
    assert_eq!(calls.made(), 2);
    assert!(fixture.owner.log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_keyed_stream_presents_a_local_key_to_its_attempt() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    fixture
        .library()
        .submit_streaming(
            Ticks::<true>(Arc::clone(&calls), Some("tick-1")),
            NonZeroUsize::MIN,
        )
        .finish()
        .await
        .expect("a keyed library stream");
    let local = local_idempotency_key(&StrictPooled::key(), "billing.ticks", 1, "tick-1")
        .expect("local key");
    assert_eq!(calls.keys(), vec![Some(local.to_string())]);
}

// ── sessions ─────────────────────────────────────────────────────────────

/// What a session body does.
#[derive(Clone, Copy)]
enum Body {
    Ok(u64),
    Fail,
    Hang,
    Panic,
}

const SESSION: &str = "billing.session";

/// A `Write` session of `request` whose body does `body`; `calls` counts
/// the bodies run and the idempotency key each saw.
fn session(
    row: &ResourceHandle<StrictPooled>,
    request: &'static str,
    body: Body,
    calls: &Arc<Calls>,
) -> super::super::Submission<u64> {
    let calls = Arc::clone(calls);
    row.session(
        SessionSpec::write(SESSION, request).cost(Cost::FREE),
        move |tx, cx| {
            calls.call(cx.idempotency_key());
            Box::pin(async move {
                tx.pending += 1;
                match body {
                    Body::Ok(value) => Ok(value),
                    Body::Fail => Err(OperationError::new(ErrorKind::Permanent, "constraint")),
                    Body::Hang => {
                        tokio::time::sleep(Duration::from_hours(1)).await;
                        Ok(0)
                    },
                    Body::Panic => panic!("the body panicked"),
                }
            })
        },
    )
}

#[tokio::test(start_paused = true)]
async fn session_outcomes_are_recorded_by_how_the_session_closed() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    let last = |fixture: &Fixture| fixture.owner.log().last().cloned();

    // Committed: applied, with the output, which a resumed run replays
    // without opening a session.
    let committed = session(&row, "committed", Body::Ok(11), &calls).await;
    assert_eq!(committed.expect("committed"), 11);
    assert_eq!(last(&fixture), Some(Step::Settle("applied")));
    let intent = &fixture.owner.intents()[0];
    assert_eq!(intent.occurrence, occurrence(0));
    assert_eq!(intent.operation, SESSION);
    assert_eq!(intent.kind, UnitKind::Session);
    assert_eq!(intent.canonical_request, br#""committed""#);
    assert_eq!(intent.recovery, Recovery::Opaque);
    fixture.owner.resume();
    let replayed = session(&row, "committed", Body::Ok(12), &calls).await;
    assert_eq!(replayed.expect("replayed"), 11);
    assert_eq!(fixture.resource.probe.opens(), 1);
    assert_eq!(calls.made(), 1);
    assert!(calls.keys()[0].is_some(), "an owned session has a key");

    // Rolled back: not crossed.
    let failed = session(&row, "failed", Body::Fail, &calls)
        .await
        .expect_err("the body failed");
    assert_eq!(failed.sent(), SentState::NotSent);
    assert_eq!(last(&fixture), Some(Step::Explain(Crossing::NotCrossed)));

    // Unknown, deadline, panic: ambiguous.
    fixture
        .resource
        .probe
        .close_next_with(SessionClosed::Unknown(OperationError::new(
            ErrorKind::Transient,
            "connection dropped",
        )));
    let unknown = session(&row, "unknown", Body::Ok(1), &calls)
        .await
        .expect_err("unknown commit");
    assert_eq!(unknown.sent(), SentState::MaybeSent);
    assert_eq!(last(&fixture), Some(Step::Explain(Crossing::Ambiguous)));

    let deadline = session(&row, "deadline", Body::Hang, &calls)
        .with_deadline(Instant::now().into_std() + Duration::from_secs(2))
        .await
        .expect_err("the deadline cut the session off");
    assert_eq!(deadline.sent(), SentState::MaybeSent);
    assert_eq!(last(&fixture), Some(Step::Explain(Crossing::Ambiguous)));

    let panicked = session(&row, "panic", Body::Panic, &calls)
        .await
        .expect_err("the body panicked");
    assert_eq!(panicked.sent(), SentState::MaybeSent);
    assert_eq!(last(&fixture), Some(Step::Explain(Crossing::Ambiguous)));
}

#[tokio::test(start_paused = true)]
async fn sessions_route_by_their_spec() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    let run = |spec: SessionSpec| {
        let calls = Arc::clone(&calls);
        row.session(spec.cost(Cost::FREE), move |tx, cx| {
            calls.call(cx.idempotency_key());
            Box::pin(async move {
                tx.pending += 1;
                Ok(1_u64)
            })
        })
    };

    // A read session is never prepared.
    run(SessionSpec::read("billing.balance"))
        .await
        .expect("a read session");
    assert!(fixture.owner.log().is_empty());

    // An idempotent session recovers by its key window and key part.
    run(SessionSpec::idempotent("billing.transfer", &("a", "b", 3))
        .idempotency_key("transfer-1")
        .key_window(Duration::from_secs(30))
        .version(3))
    .await
    .expect("an idempotent session");
    let intent = &fixture.owner.intents()[0];
    assert_eq!(intent.occurrence, occurrence(0));
    assert_eq!(intent.operation, "billing.transfer");
    assert_eq!(intent.version, 3);
    assert_eq!(
        intent.recovery,
        Recovery::StableKey {
            window: Duration::from_secs(30)
        }
    );
    assert_eq!(intent.key_part.as_deref(), Some("transfer-1"));
    assert_eq!(intent.canonical_request, br#"["a","b",3]"#);

    // Defects are refused at submit, on every row.
    for (spec, detail) in [
        (
            SessionSpec::read("bad name"),
            "session name must be 1..=64 bytes of [A-Za-z0-9_.-], starting and ending alphanumeric",
        ),
        (
            SessionSpec::write("billing.opaque", &BTreeMap::from([(vec![1_u8], 1_u8)])),
            "operation request does not serialize to JSON",
        ),
        (
            SessionSpec::write("billing.versioned", &1).version(0),
            "operation version must be at least 1",
        ),
    ] {
        let error = run(spec).await.expect_err("a defect");
        assert_unsent(&error, &ErrorKind::Permanent);
        assert_eq!(error.detail(), detail);
    }
    assert_eq!(calls.made(), 2);
    assert_eq!(fixture.owner.intents().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_library_session_runs_without_an_owner() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let value = session(&fixture.library(), "library", Body::Ok(3), &calls)
        .await
        .expect("committed");
    assert_eq!(value, 3);
    assert_eq!(calls.keys(), vec![None]);
    let refused = session(&fixture.read_only(), "read-only", Body::Ok(3), &calls)
        .await
        .expect_err("no effect authority");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert!(fixture.owner.log().is_empty());
}
