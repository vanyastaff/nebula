//! The per-unit checkout facade: attempts check out an instance only after
//! their quota and row-gate waits, queue FIFO at the row gate, read their
//! bound credentials once for an idle hit and twice for a create, never hold
//! `Manager.admission` across a read or a create, and settle every refusal
//! unsent.

use std::{
    num::NonZeroU32,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key};
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialObserveError,
};
use rstest::rstest;
use tokio::{sync::Notify, time::Instant};

use super::super::{Cost, Effect, ManagedRow, OpCx, OpError, Operation, PinSlots, SentState};
use crate::{
    AcquireOptions, CredentialAdmissionProfile, CredentialUnavailableReason, Error, ErrorKind,
    Manager, PoolConfig, Pooled, Provider, RateLimitProfile, RegistrationSpec, ResourceContext,
    ResourceGuard, ShutdownConfig,
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
