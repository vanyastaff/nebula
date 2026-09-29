//! Execution-owned effects on a managed row, against a deterministic
//! in-memory owner ([`FakeOwner`]) whose phase machine mirrors the operation
//! ledger's rules: an opaque effect is granted again only from prepared or
//! not-crossed, a stable-key one also from ambiguous within its window, an
//! exhausted budget or an ambiguous opaque call makes the outcome unknown.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, Scope};
use nebula_credential::CredentialAvailability;
use nebula_metrics::MetricsRegistry;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::super::{
    Cost, Effect, EffectContract, EffectOperation, EffectRecovery, IdempotencyKeyPart, ManagedRow,
    OccurrenceLabel, OpCx, OpError, Operation, OperationKey, PinSlots, Recorded, SentState,
    SessionClosed, SessionSpec,
    owner::{
        Crossing, ErrorKindCode, OwnerRefusal, OwnerTicket, RecordedOutcome, SlotPhase, UnitCall,
        UnitEffectOwner, UnitIntent, UnitOutcome, UnitSlot,
    },
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
    Outstanding(UnitCall),
    BeforeBoundary,
    Ambiguous,
    Resolved(RecordedOutcome),
    Unknown,
}

#[derive(Debug)]
struct FakeSlot {
    id: [u8; 16],
    key: OperationKey,
    /// Contract id, canonical request and key part: what a resumed effect
    /// must present again.
    fingerprint: (&'static str, Vec<u8>, Option<String>),
    recovery: EffectRecovery,
    max_invocations: u32,
    invocations: u32,
    prepared_at: Instant,
    phase: Phase,
}

/// What the fake saw of one prepare.
#[derive(Debug, Clone)]
struct SeenIntent {
    occurrence: String,
    key_part: Option<String>,
    effect: Effect,
    max_invocations: u32,
    binding: SlotIdentity,
}

#[derive(Default)]
struct FakeState {
    ordinals: HashMap<(ResourceKey, &'static str), u32>,
    slots: HashMap<String, FakeSlot>,
    next_id: u8,
    log: Vec<Step>,
    intents: Vec<SeenIntent>,
    fail_prepare: Option<OwnerRefusal>,
    fail_settle: Option<OwnerRefusal>,
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
        self.state().ordinals.clear();
    }

    fn fail_next_prepare(&self, refusal: OwnerRefusal) {
        self.state().fail_prepare = Some(refusal);
    }

    fn fail_next_settle(&self, refusal: OwnerRefusal) {
        self.state().fail_settle = Some(refusal);
    }

    fn on_next_grant(&self, hook: impl FnOnce() + Send + 'static) {
        self.state().on_grant = Some(Box::new(hook));
    }

    /// Records `outcome` under `occurrence` as a finished earlier run did.
    fn seed(&self, occurrence: &str, fingerprint: (&'static str, &[u8]), phase: Phase) {
        let mut state = self.state();
        state.next_id += 1;
        let id = state.next_id;
        state.slots.insert(
            occurrence.to_owned(),
            FakeSlot {
                id: [id; 16],
                key: OperationKey::new(&format!("key-{id}")).expect("key"),
                fingerprint: (fingerprint.0, fingerprint.1.to_vec(), None),
                recovery: EffectRecovery::Opaque,
                max_invocations: 1,
                invocations: 1,
                prepared_at: Instant::now(),
                phase,
            },
        );
    }

    fn slot_view(slot: &FakeSlot, phase: SlotPhase) -> UnitSlot {
        UnitSlot::new(slot.id, slot.key, 1, phase)
    }

    fn by_id<'s>(state: &'s mut FakeState, id: &[u8; 16]) -> Option<&'s mut FakeSlot> {
        state.slots.values_mut().find(|slot| slot.id == *id)
    }
}

#[async_trait::async_trait]
impl UnitEffectOwner for FakeOwner {
    fn next_ordinal(&self, key: &ResourceKey, contract: EffectContract) -> u32 {
        let mut state = self.state();
        let ordinal = state
            .ordinals
            .entry((key.clone(), contract.id()))
            .or_insert(0);
        let next = *ordinal;
        *ordinal += 1;
        next
    }

    async fn prepare(&self, intent: &UnitIntent<'_>) -> Result<UnitSlot, OwnerRefusal> {
        if self.is_closed() {
            return Err(OwnerRefusal::Closed);
        }
        let mut state = self.state();
        if let Some(refusal) = state.fail_prepare.take() {
            return Err(refusal);
        }
        state.log.push(Step::Prepare(intent.occurrence.to_owned()));
        state.intents.push(SeenIntent {
            occurrence: intent.occurrence.to_owned(),
            key_part: intent.key_part.map(|part| part.as_str().to_owned()),
            effect: intent.effect,
            max_invocations: intent.max_invocations.get(),
            binding: intent.binding.clone(),
        });
        let fingerprint = (
            intent.contract.id(),
            intent.canonical_request.to_vec(),
            intent.key_part.map(|part| part.as_str().to_owned()),
        );
        let now = Instant::now();
        if let Some(slot) = state.slots.get_mut(intent.occurrence) {
            if slot.fingerprint != fingerprint {
                return Err(OwnerRefusal::Mismatch);
            }
            let phase = match (&slot.phase, slot.recovery) {
                (Phase::Resolved(outcome), _) => SlotPhase::Replay(outcome.clone()),
                (Phase::Prepared | Phase::BeforeBoundary, _) => SlotPhase::Runnable,
                (
                    Phase::Ambiguous | Phase::Outstanding(_),
                    EffectRecovery::StableKey { window },
                ) if now < slot.prepared_at + window => {
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
            key: OperationKey::new(&format!("key-{id}")).expect("key"),
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

    async fn grant(&self, slot: &UnitSlot) -> Result<UnitCall, OwnerRefusal> {
        if self.is_closed() {
            return Err(OwnerRefusal::Closed);
        }
        let hook = {
            let mut state = self.state();
            let now = Instant::now();
            let call_id = state.next_id.wrapping_add(100);
            state.next_id += 1;
            let fake = Self::by_id(&mut state, slot.id()).ok_or(OwnerRefusal::Mismatch)?;
            let grantable = match (&fake.phase, fake.recovery) {
                (Phase::Prepared | Phase::BeforeBoundary, _) => true,
                (Phase::Ambiguous, EffectRecovery::StableKey { window }) => {
                    now < fake.prepared_at + window
                },
                _ => false,
            };
            if !grantable || fake.invocations >= fake.max_invocations {
                fake.phase = Phase::Unknown;
                return Err(OwnerRefusal::Unknown);
            }
            fake.invocations += 1;
            let call = UnitCall::from_bytes([call_id; 16]);
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
        slot: &UnitSlot,
        call: UnitCall,
        crossing: Crossing,
    ) -> Result<(), OwnerRefusal> {
        let mut state = self.state();
        let fake = Self::by_id(&mut state, slot.id()).ok_or(OwnerRefusal::Mismatch)?;
        if fake.phase != Phase::Outstanding(call) {
            return Err(OwnerRefusal::Mismatch);
        }
        fake.phase = match (crossing, fake.recovery) {
            (Crossing::NotCrossed, _) => Phase::BeforeBoundary,
            (Crossing::Ambiguous, EffectRecovery::StableKey { .. }) => Phase::Ambiguous,
            _ => Phase::Unknown,
        };
        state.log.push(Step::Explain(crossing));
        Ok(())
    }

    async fn settle(
        &self,
        slot: &UnitSlot,
        call: UnitCall,
        outcome: UnitOutcome<'_>,
    ) -> Result<(), OwnerRefusal> {
        let mut state = self.state();
        if let Some(refusal) = state.fail_settle.take() {
            return Err(refusal);
        }
        let fake = Self::by_id(&mut state, slot.id()).ok_or(OwnerRefusal::Mismatch)?;
        if fake.phase != Phase::Outstanding(call) {
            return Err(OwnerRefusal::Mismatch);
        }
        let (recorded, step) = match outcome {
            UnitOutcome::Applied(bytes) => (RecordedOutcome::Succeeded(bytes.to_vec()), "applied"),
            UnitOutcome::AppliedWithoutOutput => {
                (RecordedOutcome::OutputUnavailable, "applied_without_output")
            },
            UnitOutcome::Rejected(code) => (RecordedOutcome::Failed(code), code.as_str()),
        };
        fake.phase = Phase::Resolved(recorded);
        state.log.push(Step::Settle(step));
        Ok(())
    }

    fn track(&self) -> OwnerTicket {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        let in_flight = Arc::clone(&self.in_flight);
        OwnerTicket::new(move || {
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
    /// Settled `Sent`; the unit yields the value.
    Ok(u64),
    /// Settled `sent`; the unit fails with `kind` when no attempt is left.
    Fail(SentState, ErrorKind),
    /// Dropped unsettled; the unit fails `Transient` when no attempt is
    /// left.
    Unsettled,
}

/// Provider calls made and the operation key each attempt saw.
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
}

const PAY: EffectContract = EffectContract::new("billing.pay", 1);
const REFUND: EffectContract = EffectContract::new("billing.refund", 1);
const WINDOW: Duration = Duration::from_mins(1);

/// A payment: `Idempotent` with a stable key when `IDEM`, a `Write`
/// otherwise; one attempt per reply.
#[derive(Debug, Clone)]
struct Pay<const IDEM: bool> {
    request: &'static str,
    label: Option<&'static str>,
    key_part: Option<&'static str>,
    replies: Vec<Reply>,
    cost: Cost,
    calls: Arc<Calls>,
}

impl<const IDEM: bool> Pay<IDEM> {
    fn new(calls: &Arc<Calls>, replies: Vec<Reply>) -> Self {
        Self {
            request: "pay:42",
            label: None,
            key_part: None,
            replies,
            cost: Cost::FREE,
            calls: Arc::clone(calls),
        }
    }

    fn labeled(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }
}

impl<R: Provider + PinSlots, const IDEM: bool> Operation<R> for Pay<IDEM> {
    type Output = u64;
    const EFFECT: Effect = if IDEM {
        Effect::Idempotent
    } else {
        Effect::Write
    };

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(u32::try_from(self.replies.len()).expect("few")).expect("a reply")
    }

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<u64, OpError> {
        let last = self.replies.len() - 1;
        for (index, reply) in self.replies.into_iter().enumerate() {
            // The key is the same for every attempt; it is read while no
            // attempt borrows the context.
            let key = cx.operation_key().copied();
            let attempt = cx.attempt(self.cost.clone()).await?;
            self.calls.made.fetch_add(1, Ordering::SeqCst);
            self.calls
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(key.map(|key| key.to_string()));
            match reply {
                Reply::Ok(value) => {
                    attempt.settle(SentState::Sent);
                    return Ok(value);
                },
                Reply::Fail(sent, kind) => {
                    attempt.settle(sent);
                    if index == last {
                        return Err(OpError::new(kind, "provider answered"));
                    }
                },
                Reply::Unsettled => {
                    drop(attempt);
                    if index == last {
                        return Err(OpError::new(ErrorKind::Transient, "connection reset"));
                    }
                },
            }
        }
        Err(OpError::new(ErrorKind::Permanent, "no reply scripted"))
    }
}

impl<R: Provider + PinSlots, const IDEM: bool> EffectOperation<R> for Pay<IDEM> {
    const CONTRACT: EffectContract = PAY;
    const RECOVERY: EffectRecovery = if IDEM {
        EffectRecovery::StableKey { window: WINDOW }
    } else {
        EffectRecovery::Opaque
    };

    fn canonical_request(&self) -> Result<Vec<u8>, OpError> {
        Ok(self.request.as_bytes().to_vec())
    }

    fn idempotency_key(&self) -> Option<IdempotencyKeyPart> {
        self.key_part
            .map(|part| IdempotencyKeyPart::new(part).expect("valid part"))
    }

    fn occurrence(&self) -> Option<OccurrenceLabel> {
        self.label
            .map(|label| OccurrenceLabel::new(label).expect("valid label"))
    }
}

/// A `Write` of another contract, recorded as a digest only.
struct Refund(Arc<Calls>);

impl<R: Provider + PinSlots> Operation<R> for Refund {
    type Output = u64;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<u64, OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        self.0.made.fetch_add(1, Ordering::SeqCst);
        attempt.settle(SentState::Sent);
        Ok(7)
    }
}

impl<R: Provider + PinSlots> EffectOperation<R> for Refund {
    const CONTRACT: EffectContract = REFUND;
    const RECOVERY: EffectRecovery = EffectRecovery::Opaque;
    const RECORDED: Recorded = Recorded::DigestOnly;

    fn canonical_request(&self) -> Result<Vec<u8>, OpError> {
        Ok(b"refund".to_vec())
    }
}

/// An operation whose declarations are malformed: `EFFECT` is `effect`,
/// the recovery opaque, the contract `contract`.
struct Misdeclared<const READ: bool, const BAD_CONTRACT: bool>(Arc<Calls>);

impl<R: Provider + PinSlots, const READ: bool, const BAD_CONTRACT: bool> Operation<R>
    for Misdeclared<READ, BAD_CONTRACT>
{
    type Output = ();
    const EFFECT: Effect = if READ {
        Effect::Read
    } else if BAD_CONTRACT {
        Effect::Write
    } else {
        Effect::Idempotent
    };

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        self.0.made.fetch_add(1, Ordering::SeqCst);
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

impl<R: Provider + PinSlots, const READ: bool, const BAD_CONTRACT: bool> EffectOperation<R>
    for Misdeclared<READ, BAD_CONTRACT>
{
    const CONTRACT: EffectContract = if BAD_CONTRACT {
        EffectContract::new("bad contract", 1)
    } else {
        PAY
    };
    const RECOVERY: EffectRecovery = EffectRecovery::Opaque;

    fn canonical_request(&self) -> Result<Vec<u8>, OpError> {
        Ok(b"x".to_vec())
    }
}

/// A read through the plain `submit`.
struct Look(Arc<Calls>, Cost);

impl<R: Provider + PinSlots> Operation<R> for Look {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        let attempt = cx.attempt(self.1).await?;
        self.0.made.fetch_add(1, Ordering::SeqCst);
        attempt.settle(SentState::Sent);
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

    fn owned(&self) -> ManagedRow<StrictPooled> {
        let owner = Arc::clone(&self.owner) as Arc<dyn UnitEffectOwner>;
        *self
            .manager
            .managed_row_any_owned(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
                owner,
            )
            .expect("owned row")
            .downcast::<ManagedRow<StrictPooled>>()
            .expect("typed row")
    }

    fn library(&self) -> ManagedRow<StrictPooled> {
        *self
            .manager
            .managed_row_any(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
            )
            .expect("library row")
            .downcast::<ManagedRow<StrictPooled>>()
            .expect("typed row")
    }

    fn read_only(&self) -> ManagedRow<StrictPooled> {
        *self
            .manager
            .managed_row_any_read_only(
                &StrictPooled::key(),
                &self.ctx(),
                &AcquireOptions::default(),
                &tenant(),
            )
            .expect("read-only row")
            .downcast::<ManagedRow<StrictPooled>>()
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

/// The occurrence label of `label` for `contract` on the fixture row.
fn occurrence(contract: EffectContract, label: &str) -> String {
    format!("unit/v1/{}/{}/{label}", StrictPooled::key(), contract.id())
}

fn assert_unsent(error: &OpError, kind: &ErrorKind) {
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(error.sent(), SentState::NotSent, "{error}");
}

// ── compile gates ────────────────────────────────────────────────────────

#[test]
fn the_owner_is_object_safe_and_an_owned_row_crosses_threads() {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    let _: Option<Arc<dyn UnitEffectOwner>> = None;
    send_sync_clone::<Arc<dyn UnitEffectOwner>>();
    send_sync_clone::<ManagedRow<StrictPooled>>();
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
    let label = occurrence(PAY, "charge");
    fixture.owner.seed(
        &label,
        (PAY.id(), b"pay:42"),
        Phase::Resolved(RecordedOutcome::Succeeded(b"99".to_vec())),
    );
    let row = fixture.owned();

    let replayed = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("charge"))
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
    assert_eq!(fixture.owner.log(), vec![Step::Prepare(label)]);
    // The quota's only permit is still there: a read books it at once.
    let started = Instant::now();
    row.submit(Look(Arc::clone(&calls), Cost::ONE))
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

    fixture.owner.seed(
        &occurrence(PAY, "rejected"),
        (PAY.id(), b"pay:42"),
        Phase::Resolved(RecordedOutcome::Failed(ErrorKindCode::Permanent)),
    );
    let rejected = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("rejected"))
        .await
        .expect_err("the recorded rejection");
    assert_eq!(*rejected.kind(), ErrorKind::Permanent);
    assert_eq!(rejected.sent(), SentState::Sent);
    assert!(!rejected.is_retryable());

    // A retryable code is final once recorded.
    fixture.owner.seed(
        &occurrence(PAY, "throttled"),
        (PAY.id(), b"pay:42"),
        Phase::Resolved(RecordedOutcome::Failed(ErrorKindCode::Transient)),
    );
    let final_error = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("throttled"))
        .await
        .expect_err("recorded");
    assert!(!final_error.is_retryable());

    fixture.owner.seed(
        &occurrence(PAY, "digest"),
        (PAY.id(), b"pay:42"),
        Phase::Resolved(RecordedOutcome::OutputUnavailable),
    );
    let digest = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("digest"))
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
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(5)]).labeled("once"))
        .await
        .expect("paid");
    assert_eq!(paid, 5);
    let again = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(6)]).labeled("once"))
        .await
        .expect("replayed");
    assert_eq!(again, 5, "the first output replays");

    let rejected = row
        .submit_effect(
            Pay::<false>::new(
                &calls,
                vec![Reply::Fail(SentState::Sent, ErrorKind::Permanent)],
            )
            .labeled("declined"),
        )
        .await
        .expect_err("declined");
    assert_eq!(*rejected.kind(), ErrorKind::Permanent);
    let replayed = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(6)]).labeled("declined"))
        .await
        .expect_err("the rejection replays");
    assert_eq!(*replayed.kind(), ErrorKind::Permanent);
    assert_eq!(replayed.sent(), SentState::Sent);

    // A digest-only effect records no output and replays none.
    let refunded = row
        .submit_effect(Refund(Arc::clone(&calls)))
        .await
        .expect("refunded");
    assert_eq!(refunded, 7);

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

// ── unknown outcomes ─────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn an_unsettled_write_makes_the_outcome_unknown_for_every_later_unit() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    let label = occurrence(PAY, "charge");

    let error = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Unsettled]).labeled("charge"))
        .await
        .expect_err("the connection reset");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(error.is_outcome_unknown());
    assert_eq!(fixture.owner.phase(&label), Some(Phase::Unknown));

    let blocked = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("charge"))
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
            Step::Prepare(label.clone()),
            Step::Grant,
            Step::Explain(Crossing::Ambiguous),
            Step::Prepare(label),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_write_retried_after_a_sent_attempt_is_refused_outcome_unknown() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let error = row
        .submit_effect(Pay::<false>::new(
            &calls,
            vec![
                Reply::Fail(SentState::Sent, ErrorKind::Transient),
                Reply::Ok(1),
            ],
        ))
        .await
        .expect_err("the owner refuses the retry");
    assert_eq!(*error.kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(error.sent(), SentState::Sent);
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
        .submit_effect(Pay::<false>::new(
            &calls,
            vec![
                Reply::Fail(SentState::NotSent, ErrorKind::Transient),
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
async fn settle_without_acknowledgement_after_the_apply_is_outcome_unknown() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture
        .owner
        .fail_next_settle(OwnerRefusal::AcknowledgementUnknown);

    let error = row
        .submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(1)]))
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
        .submit_effect(Pay::<true>::new(
            &calls,
            vec![Reply::Unsettled, Reply::Ok(8)],
        ))
        .await
        .expect("the retry lands");
    assert_eq!(paid, 8);
    let keys = calls.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys[0].is_some(), "an owned unit has an operation key");
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
        .submit_effect(Pay::<true>::new(&calls, vec![Reply::Unsettled]).labeled("exhausted"))
        .await
        .expect_err("ambiguous");
    assert_eq!(exhausted.sent(), SentState::MaybeSent);
    assert_eq!(
        fixture.owner.phase(&occurrence(PAY, "exhausted")),
        Some(Phase::Ambiguous)
    );
    let refused = row
        .submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(1)]).labeled("exhausted"))
        .await
        .expect_err("no invocation left");
    assert_eq!(*refused.kind(), ErrorKind::OutcomeUnknown);
}

#[tokio::test(start_paused = true)]
async fn the_developer_key_part_reaches_the_intent() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let mut keyed = Pay::<true>::new(&calls, vec![Reply::Ok(1)]);
    keyed.key_part = Some("order-123");
    row.submit_effect(keyed).await.expect("keyed");
    row.submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect("unkeyed");

    let intents = fixture.owner.intents();
    assert_eq!(intents[0].key_part.as_deref(), Some("order-123"));
    assert_eq!(intents[1].key_part, None);
    assert_eq!(intents[0].effect, Effect::Idempotent);
    assert_eq!(intents[0].max_invocations, 1);
    assert_eq!(intents[0].binding, tenant());
}

// ── labels ───────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn ordinals_are_zero_padded_per_contract_and_author_labels_verbatim() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    // Ordinals are taken at submit, in program order.
    let first = row.submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(1)]));
    let second = row.submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(2)]));
    let refund = row.submit_effect(Refund(Arc::clone(&calls)));
    let labeled = row.submit_effect(Pay::<true>::new(&calls, vec![Reply::Ok(3)]).labeled("charge"));
    second.await.expect("second");
    first.await.expect("first");
    refund.await.expect("refund");
    labeled.await.expect("labeled");

    let mut seen: Vec<_> = fixture
        .owner
        .intents()
        .into_iter()
        .map(|intent| intent.occurrence)
        .collect();
    seen.sort();
    let mut expected = vec![
        occurrence(PAY, "#000000"),
        occurrence(PAY, "#000001"),
        occurrence(REFUND, "#000000"),
        occurrence(PAY, "charge"),
    ];
    expected.sort();
    assert_eq!(seen, expected);
    assert!(OccurrenceLabel::new("with space").is_err());
    assert!(OccurrenceLabel::new("x".repeat(129)).is_err());
}

// ── refusals ─────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn prepare_refusals_map_to_unsent_errors_without_a_call() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    for refusal in [
        OwnerRefusal::AcknowledgementUnknown,
        OwnerRefusal::Unavailable,
        OwnerRefusal::LeaseLost,
    ] {
        fixture.owner.fail_next_prepare(refusal);
        let error = row
            .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
            .await
            .expect_err("refused");
        assert_unsent(&error, &ErrorKind::Backpressure);
        assert!(error.is_retryable());
    }
    fixture.owner.fail_next_prepare(OwnerRefusal::Closed);
    let closed = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("closed");
    assert_unsent(&closed, &ErrorKind::Cancelled);

    assert_eq!(calls.made(), 0);
    assert_eq!(fixture.attempts(), (0, 0));
    assert_eq!(fixture.resource.probe.creates(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_different_effect_under_a_label_is_a_permanent_mismatch() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();
    fixture.owner.seed(
        &occurrence(PAY, "charge"),
        (PAY.id(), b"pay:41"),
        Phase::Prepared,
    );

    let error = row
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]).labeled("charge"))
        .await
        .expect_err("another request under the label");
    assert_unsent(&error, &ErrorKind::Permanent);
    assert_eq!(error.detail(), "effect occurrence mismatch");
    assert_eq!(calls.made(), 0);
}

#[tokio::test(start_paused = true)]
async fn misdeclared_effects_and_reads_are_refused_at_submit() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let mismatched = row
        .submit_effect(Misdeclared::<false, false>(Arc::clone(&calls)))
        .await
        .expect_err("idempotent needs a stable key");
    assert_unsent(&mismatched, &ErrorKind::Permanent);
    let read = row
        .submit_effect(Misdeclared::<true, false>(Arc::clone(&calls)))
        .await
        .expect_err("a read is not an owned effect");
    assert_unsent(&read, &ErrorKind::Permanent);
    let contract = row
        .submit_effect(Misdeclared::<false, true>(Arc::clone(&calls)))
        .await
        .expect_err("a malformed contract");
    assert_unsent(&contract, &ErrorKind::Permanent);

    assert_eq!(calls.made(), 0);
    assert!(fixture.owner.log().is_empty(), "nothing reached the owner");
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
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
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
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
    row.submit(Look(Arc::clone(&calls), Cost::ONE))
        .await
        .expect("read");

    let mut pay = Pay::<false>::new(&calls, vec![Reply::Ok(1)]);
    pay.cost = Cost::ONE;
    let mut unit = row.submit_effect(pay);
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
            .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
            .await
            .expect_err("the owner closed");
        assert_unsent(&error, &ErrorKind::Cancelled);
    }
    assert_eq!(calls.made(), 0);
    assert!(fixture.owner.log().is_empty());
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn plain_submit_refuses_effects_and_runs_reads_without_the_owner() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let row = fixture.owned();

    let error = row
        .submit(Pay::<false>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("effects go through submit_effect");
    assert_unsent(&error, &ErrorKind::Permanent);
    assert_eq!(
        error.detail(),
        "execution-owned effects go through submit_effect"
    );
    let idempotent = row
        .submit(Pay::<true>::new(&calls, vec![Reply::Ok(1)]))
        .await
        .expect_err("idempotent too");
    assert_unsent(&idempotent, &ErrorKind::Permanent);
    let session = row
        .session(SessionSpec::new(Cost::FREE), |_tx, _cx| {
            Box::pin(async { Ok(()) })
        })
        .await
        .expect_err("a write session too");
    assert_unsent(&session, &ErrorKind::Permanent);

    row.submit(Look(Arc::clone(&calls), Cost::FREE))
        .await
        .expect("a read runs");
    assert_eq!(calls.made(), 1);
    assert!(fixture.owner.log().is_empty());
    assert_eq!(fixture.owner.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_library_row_runs_submit_effect_as_submit_and_a_read_only_row_refuses_it() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());

    let paid = fixture
        .library()
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect("a library effect");
    assert_eq!(paid, 4);
    assert_eq!(calls.keys(), vec![None], "no owner, no operation key");

    let refused = fixture
        .read_only()
        .submit_effect(Pay::<false>::new(&calls, vec![Reply::Ok(4)]))
        .await
        .expect_err("no effect authority");
    assert_unsent(&refused, &ErrorKind::Permanent);
    assert_eq!(
        refused.detail(),
        "managed row effect requires execution-owner authority"
    );
    assert_eq!(calls.made(), 1);
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

const SESSION: EffectContract = EffectContract::new("billing.session", 1);

/// A `Write` session effect of `request` whose body does `body`; `calls`
/// counts the bodies run and the operation key each saw.
fn session(
    row: &ManagedRow<StrictPooled>,
    request: &'static str,
    body: Body,
    calls: &Arc<Calls>,
) -> super::super::Unit<u64> {
    let calls = Arc::clone(calls);
    row.session_effect(
        SessionSpec::new(Cost::FREE),
        SESSION,
        EffectRecovery::Opaque,
        request.as_bytes().to_vec(),
        None,
        move |tx, cx| {
            calls.made.fetch_add(1, Ordering::SeqCst);
            calls
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(cx.operation_key().map(ToString::to_string));
            Box::pin(async move {
                tx.pending += 1;
                match body {
                    Body::Ok(value) => Ok(value),
                    Body::Fail => Err(OpError::new(ErrorKind::Permanent, "constraint")),
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
        .close_next_with(SessionClosed::Unknown(OpError::new(
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
async fn a_library_session_effect_runs_as_a_session_without_a_key() {
    let fixture = Fixture::new(None);
    let calls = Arc::new(Calls::default());
    let value = session(&fixture.library(), "library", Body::Ok(3), &calls)
        .await
        .expect("committed");
    assert_eq!(value, 3);
    assert_eq!(calls.keys(), vec![None]);
    assert!(fixture.owner.log().is_empty());
}
