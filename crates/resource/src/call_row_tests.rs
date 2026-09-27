//! The per-unit checkout facade: attempts check out an instance only after
//! their quota and row-gate waits, queue FIFO at the row gate, read their
//! bound credentials once for an idle hit and twice for a create, never hold
//! `Manager.admission` across a read or a create, and settle every refusal
//! unsent. Streaming units on a row keep the same per-attempt checkout.

use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, Scope, ScopeLevel, resource_key};
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialObserveError,
};
use rstest::rstest;
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

use super::super::{
    Cost, Effect, ManagedRow, OpCx, OpError, Operation, PinSlots, SentState, SessionSpec,
    StreamOperation, StreamSink, UNIT_DEADLINE_CAP, UnitScope,
};
use crate::{
    AcquireOptions, CredentialAdmissionProfile, CredentialUnavailableReason, Error, ErrorKind,
    Manager, PoolConfig, Pooled, Provider, RateLimitProfile, RegistrationSpec, ResourceContext,
    ResourceGuard, ShutdownConfig, SlotIdentity,
    manager::{
        CredentialReads,
        strict_fixtures::{
            PinnedEpochs, ScriptedObserver, StrictPooled, bind, config, context, credential_id,
            seen, strict_manager, tenant,
        },
    },
    rate_limit::{Rate, RowLimit},
    resource::{ResourceConfig as _, ResourceMetadataDraft},
    runtime::managed::ManagedResource,
    topology::{
        PoolProvider,
        pooled::{RecycleDecision, config::WarmupStrategy},
    },
};

const REAUTH: CredentialUnavailableReason = CredentialUnavailableReason::ReauthRequired;
const REBINDING: CredentialUnavailableReason = CredentialUnavailableReason::Rebinding;

type Published = Result<CredentialAvailabilityObservation, CredentialObserveError>;

fn available(material: u64, admission: u64) -> Published {
    seen(material, admission, CredentialAvailability::Available)
}

fn reauth(material: u64, admission: u64) -> Published {
    seen(
        material,
        admission,
        CredentialAvailability::Blocked(CredentialBlock::ReauthRequired),
    )
}

fn erased(observer: &Arc<ScriptedObserver>) -> Arc<dyn CredentialAvailabilityObserver> {
    Arc::clone(observer) as Arc<dyn CredentialAvailabilityObserver>
}

fn per_second(requests: u32, burst: u32) -> Rate {
    Rate::per_second(NonZeroU32::new(requests).expect("non-zero"))
        .with_burst(NonZeroU32::new(burst).expect("non-zero"))
        .expect("valid rate")
}

fn pool(max_size: u32) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        idle_timeout: None,
        max_lifetime: None,
        warmup: WarmupStrategy::None,
        maintenance_interval: Duration::from_hours(1),
        ..PoolConfig::default()
    }
}

/// A pooled strict-fixture row of `max_size`, bound at `(1, 1)`.
fn pooled(manager: &Manager, max_size: u32, limit: Option<RowLimit>) -> StrictPooled {
    let resource = StrictPooled::new();
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Pooled::new(pool(max_size), config(1).fingerprint()),
            recovery_gate: None,
            rate_limit: limit,
        })
        .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    resource
}

fn facade<R: Provider + PinSlots>(manager: &Manager) -> ManagedRow<R> {
    manager
        .managed_row_for_identity::<R>(&context(), &tenant())
        .expect("row facade")
}

/// The row's runtime.
fn runtime<R: Provider>(manager: &Manager) -> Arc<ManagedResource<R>> {
    crate::manager::strict_fixtures::row::<R>(manager)
}

/// Leases (checkouts included) the row has out.
fn in_use<R: Provider>(manager: &Manager) -> usize {
    runtime::<R>(manager).in_flight_count()
}

/// Free permits of the row gate.
fn gate_free(manager: &Manager) -> usize {
    runtime::<StrictPooled>(manager)
        .row_gate()
        .expect("a pooled row has a gate")
        .available_permits()
}

/// Yields until the row has `n` leases out (queued releases settle on the
/// release queue's tasks).
async fn until_in_use<R: Provider>(manager: &Manager, n: usize) {
    for _ in 0..10_000 {
        if in_use::<R>(manager) == n {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(in_use::<R>(manager), n, "leases out");
}

fn reads(manager: &Manager) -> Arc<CredentialReads> {
    runtime::<StrictPooled>(manager)
        .credential_reads
        .clone()
        .expect("strict row")
}

fn reason(error: &OpError) -> Option<CredentialUnavailableReason> {
    match error.kind() {
        ErrorKind::CredentialUnavailable { reason } => Some(*reason),
        _ => None,
    }
}

fn suspended(manager: &Manager) -> Option<CredentialUnavailableReason> {
    runtime::<StrictPooled>(manager)
        .admission
        .suspension()
        .and_then(|suspension| suspension.reason_for("db"))
}

fn in_one(after: Duration) -> std::time::Instant {
    Instant::now().into_std() + after
}

/// Lets every task run until the runtime is idle (paused time only).
async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

// ── operations ───────────────────────────────────────────────────────────

/// One attempt at `cost`, settled `Sent`.
struct Once(Cost);

impl<R: Provider + PinSlots> Operation<R> for Once {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        let attempt = cx.attempt(self.0).await?;
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

/// A granted attempt parked until `release` fires.
struct Held {
    cost: Cost,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

struct Hold {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

fn held(cost: Cost) -> (Held, Hold) {
    let hold = Hold {
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    };
    (
        Held {
            cost,
            entered: Arc::clone(&hold.entered),
            release: Arc::clone(&hold.release),
        },
        hold,
    )
}

impl<R: Provider + PinSlots> Operation<R> for Held {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        let attempt = cx.attempt(self.cost).await?;
        self.entered.notify_one();
        self.release.notified().await;
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

/// Two free attempts; after the first is granted and settled `Sent` it
/// signals `between` and waits for `resume`. Yields each attempt's pin.
struct Paused {
    between: Arc<Notify>,
    resume: Arc<Notify>,
}

impl<R: Provider + PinSlots<Pinned = PinnedEpochs>> Operation<R> for Paused {
    type Output = Vec<PinnedEpochs>;
    const EFFECT: Effect = Effect::Read;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(2).expect("two")
    }

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<Self::Output, OpError> {
        let first = cx.attempt(Cost::FREE).await?;
        let mut pinned = vec![first.slots().clone()];
        first.settle(SentState::Sent);
        self.between.notify_one();
        self.resume.notified().await;
        let second = cx.attempt(Cost::FREE).await?;
        pinned.push(second.slots().clone());
        second.settle(SentState::Sent);
        Ok(pinned)
    }
}

/// One attempt settled `sent`, then the provider's `kind` (if any).
struct Reply<const WRITE: bool> {
    sent: SentState,
    kind: Option<ErrorKind>,
}

impl<R: Provider + PinSlots, const WRITE: bool> Operation<R> for Reply<WRITE> {
    type Output = ();
    const EFFECT: Effect = if WRITE { Effect::Write } else { Effect::Read };

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt.settle(self.sent);
        match self.kind {
            Some(kind) => Err(OpError::new(kind, "provider answered")),
            None => Ok(()),
        }
    }
}

// ── a slot-less pooled row ───────────────────────────────────────────────

#[derive(Clone)]
struct PlainPool;

#[async_trait::async_trait]
impl Provider for PlainPool {
    type Config = crate::manager::strict_fixtures::Config;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("row-plain-pool")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("row-plain-pool"), "")
    }

    async fn create(
        &self,
        _: &crate::manager::strict_fixtures::Config,
        _: &ResourceContext,
    ) -> Result<u64, Error> {
        Ok(0)
    }
}

crate::no_credential_slots!(PlainPool);

impl PoolProvider for PlainPool {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}

// ── compile gates ────────────────────────────────────────────────────────

#[test]
fn the_row_facade_and_its_units_cross_threads() {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    fn send<T: Send + Unpin>() {}
    send_sync_clone::<ManagedRow<StrictPooled>>();
    send::<super::super::Unit<()>>();
}

// ── R1: a unit waiting for quota holds no connection ─────────────────────

#[tokio::test(start_paused = true)]
async fn a_unit_waiting_for_quota_holds_no_connection() {
    let manager = Manager::new();
    pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    let row = facade::<StrictPooled>(&manager);

    // A books the only permit and holds its checkout.
    let (operation, a) = held(Cost::ONE);
    let first = tokio::spawn(row.submit(operation));
    a.entered.notified().await;
    assert_eq!(in_use::<StrictPooled>(&manager), 1);

    // B waits a second for its permit — with nothing checked out.
    let started = Instant::now();
    let second = tokio::spawn(row.submit(Once(Cost::ONE)));
    settle_tasks().await;
    assert_eq!(in_use::<StrictPooled>(&manager), 1, "only A's checkout");

    a.release.notify_one();
    first.await.expect("joined").expect("A settles");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert!(!second.is_finished(), "B is still waiting for quota");
    assert_eq!(in_use::<StrictPooled>(&manager), 0, "B holds no connection");

    second.await.expect("joined").expect("B granted");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_lease_facade_holds_its_connection_while_its_unit_waits_for_quota() {
    let manager = Manager::new();
    pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    // The acquire books the only permit before the facade latches.
    let lease = manager
        .acquire_for_identity::<StrictPooled>(&context(), &AcquireOptions::default(), &tenant())
        .await
        .expect("acquire")
        .into_managed();

    let unit = tokio::spawn(lease.submit(Once(Cost::ONE)));
    settle_tasks().await;
    assert!(!unit.is_finished(), "waiting for quota");
    assert_eq!(
        in_use::<StrictPooled>(&manager),
        1,
        "the lease keeps the connection checked out meanwhile"
    );
    unit.await.expect("joined").expect("granted");
}

// ── R2: the gate queues FIFO, no backpressure ────────────────────────────

#[tokio::test(start_paused = true)]
async fn row_attempts_queue_fifo_at_the_gate_without_backpressure() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    let (operation, a) = held(Cost::FREE);
    let first = tokio::spawn(row.submit(operation));
    a.entered.notified().await;
    let (operation, b) = held(Cost::FREE);
    let second = tokio::spawn(row.submit(operation));
    settle_tasks().await;
    let (operation, c) = held(Cost::FREE);
    let third = tokio::spawn(row.submit(operation));
    settle_tasks().await;
    assert_eq!(gate_free(&manager), 0);

    a.release.notify_one();
    first.await.expect("joined").expect("A");
    b.entered.notified().await;
    settle_tasks().await;
    assert!(
        futures::poll!(Box::pin(c.entered.notified())).is_pending(),
        "C queued behind B"
    );
    b.release.notify_one();
    second
        .await
        .expect("joined")
        .expect("B waited, never refused");
    c.entered.notified().await;
    c.release.notify_one();
    third
        .await
        .expect("joined")
        .expect("C waited, never refused");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
}

// ── R3: reads per checkout ───────────────────────────────────────────────

#[tokio::test]
async fn reads_once_for_an_idle_hit_twice_for_a_create_and_never_for_a_slot_less_row() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 2, None);
    let row = facade::<StrictPooled>(&manager);

    row.submit(Once(Cost::FREE)).await.expect("create");
    assert_eq!(resource.probe.creates(), 1);
    assert_eq!(observer.calls(), 2, "R1, then R2 after the create");
    until_in_use::<StrictPooled>(&manager, 0).await;

    row.submit(Once(Cost::ONE)).await.expect("idle hit");
    assert_eq!(resource.probe.creates(), 1);
    assert_eq!(observer.calls(), 3, "R1 serves an idle hit");

    // A slot-less row reads nothing, even through a failing observer.
    let failing = ScriptedObserver::answering(Err(CredentialObserveError::Unavailable));
    let manager = strict_manager(erased(&failing), &Arc::default());
    manager
        .register(RegistrationSpec {
            resource: PlainPool,
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Pooled::new(pool(1), config(1).fingerprint()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let plain = facade::<PlainPool>(&manager);
    plain.submit(Once(Cost::FREE)).await.expect("create");
    plain.submit(Once(Cost::ONE)).await.expect("idle hit");
    assert_eq!(failing.calls(), 0);
}

// ── R4: a block committed during the gate wait ───────────────────────────

#[tokio::test(start_paused = true)]
async fn a_block_committed_during_the_gate_wait_refuses_without_a_create() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    let (operation, a) = held(Cost::FREE);
    let first = tokio::spawn(row.submit(operation));
    a.entered.notified().await;
    let second = tokio::spawn(row.submit(Once(Cost::FREE)));
    settle_tasks().await;
    let calls = observer.calls();

    observer.answer(reauth(1, 2));
    a.release.notify_one();
    first
        .await
        .expect("joined")
        .expect("A was granted before the block");

    let error = second.await.expect("joined").expect_err("blocked");
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(observer.calls(), calls + 1, "B's R1 only");
    assert_eq!(resource.probe.creates(), 1, "nothing created for B");
    assert_eq!(suspended(&manager), Some(REAUTH));
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
}

// ── R5: a block committed while the create is parked ─────────────────────

#[tokio::test]
async fn a_block_committed_during_the_create_is_refused_at_r2_and_the_instance_pooled() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    resource.probe.park_next_create();
    let entered = resource.probe.create_entered.notified();
    let unit = tokio::spawn(row.submit(Once(Cost::FREE)));
    entered.await;
    assert_eq!(observer.calls(), 1, "R1 before the create");
    observer.answer(reauth(1, 2));
    resource.probe.release_create.notify_one();

    let error = unit.await.expect("joined").expect_err("R2 saw the block");
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(observer.calls(), 2);
    assert_eq!(suspended(&manager), Some(REAUTH));
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(
        runtime::<StrictPooled>(&manager).store.len().await,
        1,
        "the refused checkout went back to the pool"
    );
}

// ── R6: the admission lock is never held across R1, R2 or the create ─────

#[tokio::test]
async fn the_admission_lock_is_not_held_across_the_reads_or_the_create() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    let link = reads(&manager).link().clone();

    resource.probe.park_next_create();
    let entered = resource.probe.create_entered.notified();
    let unit = tokio::spawn(row.submit(Once(Cost::FREE)));

    observer.until_calls(1).await;
    drop(link.lock()); // R1 in flight: the lock is free.
    observer.release(1);
    entered.await;
    drop(link.lock()); // The create in flight: the lock is free.
    resource.probe.release_create.notify_one();
    observer.until_calls(2).await;
    drop(link.lock()); // R2 in flight: the lock is free.
    observer.release(1);

    unit.await.expect("joined").expect("granted");
}

// ── R7: a suspension during the create ───────────────────────────────────

#[tokio::test]
async fn a_suspension_during_the_create_is_refused_at_hand_out() {
    let manager = Manager::new();
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    resource.probe.park_next_create();
    let entered = resource.probe.create_entered.notified();
    let unit = tokio::spawn(row.submit(Once(Cost::FREE)));
    entered.await;
    manager
        .suspend_credential_row(
            &StrictPooled::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            CredentialUnavailableReason::OperationBlocked,
            None,
        )
        .expect("suspend");
    resource.probe.release_create.notify_one();

    let error = unit.await.expect("joined").expect_err("hand-out refused");
    assert_eq!(
        reason(&error),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(resource.probe.creates(), 1);
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(runtime::<StrictPooled>(&manager).store.len().await, 1);
}

// ── R8: cancel before the grant, at every wait ───────────────────────────

async fn assert_cancelled(unit: super::super::Unit<()>) {
    unit.cancel();
    let error = unit.await.expect_err("cancelled");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_during_the_quota_or_gate_wait_is_cancelled_and_releases_everything() {
    // During the quota wait.
    let manager = Manager::new();
    pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    let row = facade::<StrictPooled>(&manager);
    row.submit(Once(Cost::ONE)).await.expect("takes the permit");
    let mut unit = row.submit(Once(Cost::ONE));
    assert!(futures::poll!(&mut unit).is_pending());
    settle_tasks().await;
    assert_cancelled(unit).await;
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);

    // During the gate wait.
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    let (operation, a) = held(Cost::FREE);
    let holder = tokio::spawn(row.submit(operation));
    a.entered.notified().await;
    let mut unit = row.submit(Once(Cost::FREE));
    assert!(futures::poll!(&mut unit).is_pending());
    settle_tasks().await;
    assert_cancelled(unit).await;
    a.release.notify_one();
    holder.await.expect("joined").expect("holder");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1, "the cancelled waiter took nothing");
}

#[tokio::test]
async fn a_cancel_during_the_read_or_the_create_is_cancelled_and_releases_everything() {
    // During R1.
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    let mut unit = row.submit(Once(Cost::FREE));
    assert!(futures::poll!(&mut unit).is_pending());
    observer.until_calls(1).await;
    assert_cancelled(unit).await;
    assert_eq!(reads(&manager).lane_count(), 0, "the read was dropped");
    assert_eq!(gate_free(&manager), 1);
    assert_eq!(resource.probe.creates(), 0);

    // During the create.
    let manager = Manager::new();
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    resource.probe.park_next_create();
    let entered = resource.probe.create_entered.notified();
    let mut unit = row.submit(Once(Cost::FREE));
    assert!(futures::poll!(&mut unit).is_pending());
    entered.await;
    assert_cancelled(unit).await;
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
    assert_eq!(resource.probe.creates(), 0, "the create never completed");
}

// ── R9: shutdown ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_ignores_an_idle_facade_and_ends_quota_waits() {
    let manager = Arc::new(Manager::new());
    pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    let row = facade::<StrictPooled>(&manager);
    row.submit(Once(Cost::ONE)).await.expect("takes the permit");
    let waiting = tokio::spawn(row.submit(Once(Cost::ONE)));
    settle_tasks().await;

    let _report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("an idle facade holds nothing to drain");
    let error = waiting.await.expect("joined").expect_err("shut down");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    let error = row
        .submit(Once(Cost::FREE))
        .await
        .expect_err("a shut-down row admits nothing");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
}

// ── R10: the deadline during the gate wait ───────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_deadline_during_the_gate_wait_is_backpressure() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    let (operation, a) = held(Cost::FREE);
    let holder = tokio::spawn(row.submit(operation));
    a.entered.notified().await;

    let started = Instant::now();
    let error = row
        .submit(Once(Cost::FREE))
        .with_deadline(in_one(Duration::from_secs(1)))
        .await
        .expect_err("the gate stayed full");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    a.release.notify_one();
    holder.await.expect("joined").expect("holder");
}

// ── R11: the retry-safety table over both hosts ──────────────────────────

#[derive(Debug, Clone, Copy)]
enum Host {
    Lease,
    Row,
}

async fn reply<const WRITE: bool>(host: Host, sent: SentState, kind: Option<ErrorKind>) -> OpError {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let operation = Reply::<WRITE> { sent, kind };
    let unit = match host {
        Host::Lease => manager
            .acquire_for_identity::<StrictPooled>(&context(), &AcquireOptions::default(), &tenant())
            .await
            .expect("acquire")
            .into_managed()
            .submit(operation),
        Host::Row => facade::<StrictPooled>(&manager).submit(operation),
    };
    unit.await.expect_err("the provider refused")
}

#[rstest]
#[case::write_sent_transient(
    true,
    SentState::Sent,
    ErrorKind::Transient,
    false,
    ErrorKind::OutcomeUnknown
)]
#[case::read_sent_transient(
    false,
    SentState::Sent,
    ErrorKind::Transient,
    true,
    ErrorKind::Transient
)]
#[case::write_not_sent(
    true,
    SentState::NotSent,
    ErrorKind::Transient,
    true,
    ErrorKind::Transient
)]
#[case::write_sent_throttled(
    true,
    SentState::Sent,
    ErrorKind::Exhausted { retry_after: None },
    true,
    ErrorKind::Exhausted { retry_after: None }
)]
#[case::read_maybe_sent(
    false,
    SentState::MaybeSent,
    ErrorKind::Transient,
    true,
    ErrorKind::Transient
)]
#[case::write_maybe_sent(
    true,
    SentState::MaybeSent,
    ErrorKind::Transient,
    false,
    ErrorKind::OutcomeUnknown
)]
#[case::permanent(
    true,
    SentState::Sent,
    ErrorKind::Permanent,
    false,
    ErrorKind::Permanent
)]
#[tokio::test]
async fn the_retry_safety_table_holds_for_lease_and_row_units(
    #[values(Host::Lease, Host::Row)] host: Host,
    #[case] write: bool,
    #[case] sent: SentState,
    #[case] kind: ErrorKind,
    #[case] retryable: bool,
    #[case] as_error: ErrorKind,
) {
    let error = if write {
        reply::<true>(host, sent, Some(kind)).await
    } else {
        reply::<false>(host, sent, Some(kind)).await
    };
    assert_eq!(error.sent(), sent, "{host:?}");
    assert_eq!(error.is_retryable(), retryable, "{host:?}");
    assert_eq!(*Error::from(error).kind(), as_error, "{host:?}");
}

// ── R12: a multi-attempt row unit ────────────────────────────────────────

#[tokio::test]
async fn a_multi_attempt_row_unit_checks_out_per_attempt_and_pins_once() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    let between = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let unit = tokio::spawn(row.submit(Paused {
        between: Arc::clone(&between),
        resume: Arc::clone(&resume),
    }));
    between.notified().await;
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1, "nothing held between attempts");
    resume.notify_one();
    let pinned = unit.await.expect("joined").expect("granted twice");
    assert_eq!(pinned, vec![vec![("db", Some(1))]; 2]);
    assert_eq!(resource.pins.load(Ordering::SeqCst), 1, "pinned once");
    assert_eq!(resource.probe.creates(), 1, "the second attempt hit idle");

    // A rotation between the attempts supersedes the pin.
    let unit = tokio::spawn(row.submit(Paused {
        between: Arc::clone(&between),
        resume: Arc::clone(&resume),
    }));
    between.notified().await;
    bind(&resource.db, credential_id(), 2, 2);
    observer.answer(available(2, 2));
    resume.notify_one();
    let error = unit.await.expect("joined").expect_err("superseded pin");
    assert_eq!(reason(&error), Some(REBINDING));
    assert_eq!(error.sent(), SentState::Sent, "the first attempt was sent");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
}

// ── R13: the profile latch ───────────────────────────────────────────────

#[tokio::test]
async fn a_row_facade_latches_per_attempt() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    pooled(&manager, 1, None);
    let managed = runtime::<StrictPooled>(&manager);
    assert_eq!(
        managed.credential_admission_profile(),
        CredentialAdmissionProfile::StrictPerAcquire
    );

    let _row = facade::<StrictPooled>(&manager);
    assert_eq!(
        managed.credential_admission_profile(),
        CredentialAdmissionProfile::StrictPerAttempt
    );
    assert_eq!(managed.rate_limiter.profile(), RateLimitProfile::PerAttempt);
}

// ── R14: a pool saturated by plain leases ────────────────────────────────

#[tokio::test]
async fn a_pool_saturated_by_a_plain_lease_refuses_backpressure_unsent() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);
    let lease: ResourceGuard<StrictPooled> = manager
        .acquire_for_identity::<StrictPooled>(&context(), &AcquireOptions::default(), &tenant())
        .await
        .expect("acquire");

    let error = row
        .submit(Once(Cost::ONE))
        .await
        .expect_err("the pool is full");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(gate_free(&manager), 1, "the gate permit came back");
    assert_eq!(in_use::<StrictPooled>(&manager), 1, "only the plain lease");
    drop(lease);
}

// ── streaming units on a row ─────────────────────────────────────────────

/// One attempt at `cost`; sends `items` values while its checkout is held,
/// then settles `Sent`. Counts its end, however it ends.
struct RowFeed {
    cost: Cost,
    items: u64,
    ended: Arc<AtomicUsize>,
}

/// Counts the operation's end, including a drop.
struct Ended(Arc<AtomicUsize>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl<R: Provider + PinSlots> StreamOperation<R> for RowFeed {
    type Item = u64;
    type Output = u64;
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>, mut sink: StreamSink<u64>) -> Result<u64, OpError> {
        let _ended = Ended(Arc::clone(&self.ended));
        let attempt = cx.attempt(self.cost).await?;
        for value in 0..self.items {
            sink.send(value).await?;
        }
        attempt.settle(SentState::Sent);
        Ok(self.items)
    }
}

fn row_feed(cost: Cost, items: u64) -> (RowFeed, Arc<AtomicUsize>) {
    let ended = Arc::new(AtomicUsize::new(0));
    (
        RowFeed {
            cost,
            items,
            ended: Arc::clone(&ended),
        },
        ended,
    )
}

fn capacity(items: usize) -> NonZeroUsize {
    NonZeroUsize::new(items).expect("non-zero")
}

#[tokio::test(start_paused = true)]
async fn a_row_stream_delivers_its_items_in_order_and_releases_its_checkout() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    let (operation, ended) = row_feed(Cost::FREE, 5);
    let mut stream = row.submit_streaming(operation, capacity(2));
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item.expect("no error"));
    }
    assert_eq!(items, [0, 1, 2, 3, 4]);
    assert_eq!(stream.finish().await.expect("output"), 5);
    assert_eq!(ended.load(Ordering::SeqCst), 1);
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
}

#[tokio::test(start_paused = true)]
async fn dropping_a_row_stream_mid_stream_releases_the_checkout_and_the_gate() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let row = facade::<StrictPooled>(&manager);

    let (operation, ended) = row_feed(Cost::FREE, 100);
    let mut stream = row.submit_streaming(operation, capacity(1));
    assert_eq!(stream.next().await.expect("an item").expect("no error"), 0);
    settle_tasks().await;
    assert_eq!(
        in_use::<StrictPooled>(&manager),
        1,
        "backpressure holds the attempt's checkout"
    );
    assert_eq!(gate_free(&manager), 0);
    assert_eq!(ended.load(Ordering::SeqCst), 0);

    drop(stream);
    settle_tasks().await;
    assert_eq!(ended.load(Ordering::SeqCst), 1, "the operation ended");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1, "the gate permit came back");

    // The row serves the next unit.
    row.submit(Once(Cost::FREE)).await.expect("next unit");
}

#[tokio::test(start_paused = true)]
async fn a_row_stream_waits_for_quota_with_nothing_checked_out() {
    let manager = Manager::new();
    pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    let row = facade::<StrictPooled>(&manager);

    // A books the only permit and holds its checkout.
    let (operation, a) = held(Cost::ONE);
    let first = tokio::spawn(row.submit(operation));
    a.entered.notified().await;

    let started = Instant::now();
    let (operation, _ended) = row_feed(Cost::ONE, 1);
    let stream = row.submit_streaming(operation, capacity(1));
    let reader = tokio::spawn(async move {
        let mut stream = stream;
        let first = stream.next().await;
        (first, stream.finish().await)
    });
    settle_tasks().await;
    assert_eq!(in_use::<StrictPooled>(&manager), 1, "only A's checkout");

    a.release.notify_one();
    first.await.expect("joined").expect("A settles");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert!(
        !reader.is_finished(),
        "the stream is still waiting for quota"
    );
    assert_eq!(
        in_use::<StrictPooled>(&manager),
        0,
        "the stream holds no connection while it waits"
    );

    let (item, output) = reader.await.expect("joined");
    assert_eq!(item.expect("an item").expect("no error"), 0);
    assert_eq!(output.expect("output"), 1);
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1);
}

// ── the unit scope: parent cancellation and deadline ─────────────────────

/// A context whose cancellation token is `token`.
fn context_of(token: &CancellationToken) -> ResourceContext {
    ResourceContext::minimal(Scope::default(), token.clone())
}

/// A row facade whose units inherit `token`, built the typed way.
fn linked(manager: &Manager, token: &CancellationToken) -> ManagedRow<StrictPooled> {
    manager
        .managed_row_for_identity::<StrictPooled>(&context_of(token), &tenant())
        .expect("row facade")
}

/// A row facade whose units inherit only `deadline`.
fn bounded(manager: &Manager, deadline: std::time::Instant) -> ManagedRow<StrictPooled> {
    facade::<StrictPooled>(manager).with_unit_scope(UnitScope {
        cancel: None,
        deadline: Some(deadline),
        ..UnitScope::default()
    })
}

/// Yields the unit's deadline without asking for an attempt.
struct Deadline;

impl<R: Provider + PinSlots> Operation<R> for Deadline {
    type Output = std::time::Instant;
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<std::time::Instant, OpError> {
        Ok(cx.deadline())
    }
}

/// Records that it ran, then asks for one free attempt.
struct Tracked(Arc<AtomicBool>);

impl<R: Provider + PinSlots> Operation<R> for Tracked {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<(), OpError> {
        self.0.store(true, Ordering::SeqCst);
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

fn assert_cancelled_unsent(error: &OpError) {
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
}

#[tokio::test(start_paused = true)]
async fn a_parent_cancel_during_the_quota_wait_refuses_unsent_without_a_checkout() {
    let manager = Manager::new();
    let resource = pooled(&manager, 1, Some(RowLimit::rate(per_second(1, 1))));
    facade::<StrictPooled>(&manager)
        .submit(Once(Cost::ONE))
        .await
        .expect("takes the permit");
    until_in_use::<StrictPooled>(&manager, 0).await;
    let creates = resource.probe.creates();

    let token = CancellationToken::new();
    let waiting = tokio::spawn(linked(&manager, &token).submit(Once(Cost::ONE)));
    settle_tasks().await;
    assert!(!waiting.is_finished(), "waiting for quota");
    token.cancel();

    let error = waiting.await.expect("joined").expect_err("cancelled");
    assert_cancelled_unsent(&error);
    assert_eq!(resource.probe.creates(), creates, "nothing created");
    assert_eq!(in_use::<StrictPooled>(&manager), 0, "nothing checked out");
    assert_eq!(gate_free(&manager), 1, "the gate was never taken");
}

#[tokio::test(start_paused = true)]
async fn a_parent_cancel_during_the_gate_wait_refuses_unsent() {
    let manager = Manager::new();
    let resource = pooled(&manager, 1, None);
    let (operation, a) = held(Cost::FREE);
    let holder = tokio::spawn(facade::<StrictPooled>(&manager).submit(operation));
    a.entered.notified().await;

    let token = CancellationToken::new();
    let waiting = tokio::spawn(linked(&manager, &token).submit(Once(Cost::FREE)));
    settle_tasks().await;
    assert!(!waiting.is_finished(), "queued at the gate");
    token.cancel();
    let error = waiting.await.expect("joined").expect_err("cancelled");
    assert_cancelled_unsent(&error);

    a.release.notify_one();
    holder
        .await
        .expect("joined")
        .expect("the holder is unaffected");
    until_in_use::<StrictPooled>(&manager, 0).await;
    assert_eq!(gate_free(&manager), 1, "the cancelled waiter took nothing");
    assert_eq!(resource.probe.creates(), 1, "only the holder's instance");
}

#[tokio::test]
async fn a_parent_cancel_during_the_strict_read_refuses_unsent() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager, 1, None);
    let token = CancellationToken::new();
    let unit = tokio::spawn(linked(&manager, &token).submit(Once(Cost::FREE)));
    observer.until_calls(1).await;
    token.cancel();

    let error = unit.await.expect("joined").expect_err("cancelled");
    assert_cancelled_unsent(&error);
    assert_eq!(reads(&manager).lane_count(), 0, "the read was dropped");
    assert_eq!(gate_free(&manager), 1);
    assert_eq!(resource.probe.creates(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_parent_cancel_after_the_grant_is_ignored() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let token = CancellationToken::new();
    let row = linked(&manager, &token);

    // A granted attempt runs on and settles.
    let (operation, a) = held(Cost::FREE);
    let unit = tokio::spawn(row.submit(operation));
    a.entered.notified().await;
    token.cancel();
    a.release.notify_one();
    unit.await.expect("joined").expect("granted, not cancelled");

    // A later attempt of a unit granted before the cancel is granted too.
    let token = CancellationToken::new();
    let row = linked(&manager, &token);
    let between = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let unit = tokio::spawn(row.submit(Paused {
        between: Arc::clone(&between),
        resume: Arc::clone(&resume),
    }));
    between.notified().await;
    token.cancel();
    resume.notify_one();
    let pinned = unit.await.expect("joined").expect("both attempts granted");
    assert_eq!(pinned.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_unit_whose_parent_was_cancelled_before_its_first_poll_never_runs() {
    let manager = Manager::new();
    let resource = pooled(&manager, 1, None);
    let token = CancellationToken::new();
    let row = linked(&manager, &token);
    token.cancel();

    let ran = Arc::new(AtomicBool::new(false));
    let error = row
        .submit(Tracked(Arc::clone(&ran)))
        .await
        .expect_err("cancelled");
    assert_cancelled_unsent(&error);
    assert!(!ran.load(Ordering::SeqCst), "the operation never started");
    assert_eq!(resource.probe.creates(), 0);
    assert_eq!(in_use::<StrictPooled>(&manager), 0);
    assert_eq!(gate_free(&manager), 1);

    // A session is refused the same way.
    let error = row
        .session(SessionSpec::new(Cost::FREE), |_tx, _cx| {
            Box::pin(async { Ok(()) })
        })
        .await
        .expect_err("cancelled");
    assert_cancelled_unsent(&error);
    assert_eq!(resource.probe.opens(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_scope_deadline_bounds_the_unit_and_with_deadline_cannot_extend_it() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let scope_deadline = in_one(Duration::from_secs(10));

    // Without a scope: the cap.
    let capped = facade::<StrictPooled>(&manager)
        .submit(Deadline)
        .await
        .expect("deadline");
    assert_eq!(capped, in_one(UNIT_DEADLINE_CAP));

    // The scope's deadline is shorter than the cap; a later one cannot
    // extend it, an earlier one shortens it.
    let row = bounded(&manager, scope_deadline);
    assert_eq!(
        row.submit(Deadline).await.expect("deadline"),
        scope_deadline
    );
    let later = row
        .submit(Deadline)
        .with_deadline(in_one(Duration::from_mins(1)))
        .await
        .expect("deadline");
    assert_eq!(later, scope_deadline);
    let earlier = in_one(Duration::from_secs(3));
    let shorter = row
        .submit(Deadline)
        .with_deadline(earlier)
        .await
        .expect("deadline");
    assert_eq!(shorter, earlier);

    // A unit queued at a full gate is refused at the scope's deadline.
    let (operation, a) = held(Cost::FREE);
    let holder = tokio::spawn(facade::<StrictPooled>(&manager).submit(operation));
    a.entered.notified().await;
    let started = Instant::now();
    let error = row
        .submit(Once(Cost::FREE))
        .with_deadline(in_one(Duration::from_mins(1)))
        .await
        .expect_err("the gate stayed full");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    a.release.notify_one();
    holder.await.expect("joined").expect("holder");
}

// ── the erased row facade ────────────────────────────────────────────────

/// The erased facade of `key` for `(ctx, identity)`.
fn erased_row(
    manager: &Manager,
    key: &ResourceKey,
    ctx: &ResourceContext,
    options: &AcquireOptions,
    identity: &SlotIdentity,
) -> Result<Box<dyn std::any::Any + Send + Sync>, Error> {
    manager.managed_row_any(key, ctx, options, identity)
}

/// The erased facade of the strict-fixture row, downcast.
fn typed_row(
    manager: &Manager,
    ctx: &ResourceContext,
    options: &AcquireOptions,
) -> ManagedRow<StrictPooled> {
    let erased = erased_row(manager, &StrictPooled::key(), ctx, options, &tenant())
        .expect("erased row facade");
    *erased
        .downcast::<ManagedRow<StrictPooled>>()
        .expect("a ManagedRow<StrictPooled>")
}

/// A pooled strict-fixture row of one instance for `identity`.
fn pooled_for(manager: &Manager, identity: SlotIdentity) -> StrictPooled {
    let resource = StrictPooled::new();
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: identity,
            topology: Pooled::new(pool(1), config(1).fingerprint()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    resource
}

fn identity(tenant: &str) -> SlotIdentity {
    SlotIdentity::from_bindings([("db", tenant)])
}

#[test]
fn the_erased_facade_is_send_sync_and_static() {
    fn erasable<T: Send + Sync + 'static>() {}
    erasable::<ManagedRow<StrictPooled>>();
}

#[tokio::test]
async fn the_erased_facade_downcasts_to_its_row_and_nothing_else() {
    let manager = Manager::new();
    let resource = pooled(&manager, 1, None);
    let options = AcquireOptions::default();

    let row = typed_row(&manager, &context(), &options);
    assert_eq!(row.resource_key(), &StrictPooled::key());
    row.submit(Once(Cost::FREE)).await.expect("granted");
    assert_eq!(resource.probe.creates(), 1);

    let erased = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &tenant(),
    )
    .expect("erased row facade");
    assert!(erased.downcast::<ManagedRow<PlainPool>>().is_err());
    let erased = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &tenant(),
    )
    .expect("erased row facade");
    assert!(
        erased.downcast::<ResourceGuard<StrictPooled>>().is_err(),
        "a row facade is no lease"
    );
}

#[tokio::test]
async fn the_erased_facade_refuses_a_missing_or_ambiguous_row() {
    let manager = Arc::new(Manager::new());
    let options = AcquireOptions::default();
    let error = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &tenant(),
    )
    .expect_err("nothing registered");
    assert_eq!(*error.kind(), ErrorKind::NotFound);

    pooled_for(&manager, identity("tenant-a"));
    pooled_for(&manager, identity("tenant-b"));
    let error = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &SlotIdentity::Unbound,
    )
    .expect_err("two credential rows, no identity");
    // Fails closed exactly as the erased acquire does: an unbound identity
    // never aliases one tenant's row.
    assert_eq!(*error.kind(), ErrorKind::NotFound);
    let acquired = Manager::acquire_any(
        Arc::clone(&manager),
        &StrictPooled::key(),
        &context(),
        &options,
        &SlotIdentity::Unbound,
    )
    .await
    .err()
    .map(|error| error.kind().clone());
    assert_eq!(acquired, Some(ErrorKind::NotFound));
    let error = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &identity("tenant-c"),
    )
    .expect_err("no such tenant");
    assert_eq!(*error.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn a_pinned_identity_reaches_its_own_row() {
    let manager = Manager::new();
    let a = pooled_for(&manager, identity("tenant-a"));
    let b = pooled_for(&manager, identity("tenant-b"));
    let row = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &AcquireOptions::default(),
        &identity("tenant-b"),
    )
    .expect("tenant b's row")
    .downcast::<ManagedRow<StrictPooled>>()
    .expect("typed");
    row.submit(Once(Cost::FREE)).await.expect("granted");
    assert_eq!(b.probe.creates(), 1, "tenant b's instance");
    assert_eq!(a.probe.creates(), 0, "never tenant a's");
}

#[tokio::test]
async fn the_erased_facade_refuses_a_tainted_row_and_a_shutting_down_manager() {
    let manager = Arc::new(Manager::new());
    pooled(&manager, 1, None);
    let options = AcquireOptions::default();
    runtime::<StrictPooled>(&manager).taint();
    let error = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &tenant(),
    )
    .expect_err("tainted");
    assert_eq!(*error.kind(), ErrorKind::Revoked);

    let manager = Arc::new(Manager::new());
    pooled(&manager, 1, None);
    let _report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("nothing to drain");
    let error = erased_row(
        &manager,
        &StrictPooled::key(),
        &context(),
        &options,
        &tenant(),
    )
    .expect_err("shut down");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
}

#[tokio::test]
async fn a_pool_saturated_by_leases_still_yields_a_facade_whose_attempt_is_backpressure() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    pooled(&manager, 1, None);
    let managed = runtime::<StrictPooled>(&manager);
    let lease: ResourceGuard<StrictPooled> = manager
        .acquire_for_identity::<StrictPooled>(&context(), &AcquireOptions::default(), &tenant())
        .await
        .expect("acquire");

    let row = typed_row(&manager, &context(), &AcquireOptions::default());
    assert_eq!(managed.rate_limiter.profile(), RateLimitProfile::PerAttempt);
    assert_eq!(
        managed.credential_admission_profile(),
        CredentialAdmissionProfile::StrictPerAttempt
    );
    let error = row
        .submit(Once(Cost::ONE))
        .await
        .expect_err("the pool is full");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    drop(lease);
}

#[tokio::test(start_paused = true)]
async fn the_options_deadline_and_the_context_cancel_reach_the_units() {
    let manager = Manager::new();
    pooled(&manager, 1, None);
    let deadline = in_one(Duration::from_secs(30));
    let token = CancellationToken::new();
    let row = typed_row(
        &manager,
        &context_of(&token),
        &AcquireOptions::default().with_deadline(deadline),
    );
    assert_eq!(row.submit(Deadline).await.expect("deadline"), deadline);

    token.cancel();
    let error = row.submit(Once(Cost::FREE)).await.expect_err("cancelled");
    assert_cancelled_unsent(&error);
}
