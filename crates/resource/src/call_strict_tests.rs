//! Strict per-attempt credential admission through the managed call facade:
//! every attempt on a strict manager reads its bound credentials after
//! every wait, outside `Manager.admission`, and registers under it; the
//! unit's pin is taken at its first grant and a superseded pin refuses
//! later attempts `Rebinding`.

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use nebula_core::ScopeLevel;
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialObserveError,
};
use tokio::{sync::Notify, time::Instant};

use super::super::{Cost, Effect, Managed, OpCx, OpError, Operation, PinSlots, SentState};
use crate::{
    AcquireOptions, CredentialAdmissionProfile, CredentialUnavailableReason, Error, ErrorKind,
    Manager, Pooled, Provider, RegistrationSpec, Resident, ResidentConfig, ResourceEvent,
    ShutdownConfig,
    manager::{
        CredentialReads,
        strict_fixtures::{
            CREDENTIAL_READ_TIMEOUT, PinnedEpochs, ScriptedObserver, StrictPooled, StrictResident,
            UnboundRow, bind, config, context, credential_id, pool_config, register, resident, row,
            seen, strict_manager, tenant,
        },
    },
    rate_limit::{Rate, RowLimit},
    resource::ResourceConfig as _,
};

const REAUTH: CredentialUnavailableReason = CredentialUnavailableReason::ReauthRequired;
const REBINDING: CredentialUnavailableReason = CredentialUnavailableReason::Rebinding;

type Published = Result<CredentialAvailabilityObservation, CredentialObserveError>;

fn available(material: u64, admission: u64) -> Published {
    seen(material, admission, CredentialAvailability::Available)
}

fn refreshing(material: u64) -> Published {
    seen(material, 1, CredentialAvailability::RefreshInFlight)
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

/// A strict manager with a resident row bound at `(1, 1)`.
fn setup(observer: &Arc<ScriptedObserver>) -> (Arc<Manager>, StrictResident) {
    let manager = Arc::new(strict_manager(erased(observer), &Arc::default()));
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    (manager, resource)
}

/// A strict manager with a resident row bound at `(1, 1)` whose rate is one
/// call a second, burst one.
fn setup_limited(observer: &Arc<ScriptedObserver>) -> (Arc<Manager>, StrictResident) {
    let manager = Arc::new(strict_manager(erased(observer), &Arc::default()));
    let resource = StrictResident::new();
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: Some(RowLimit::rate(
                Rate::per_second(NonZeroU32::MIN)
                    .with_burst(NonZeroU32::MIN)
                    .expect("valid rate"),
            )),
        })
        .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    (manager, resource)
}

async fn facade<R: Provider + PinSlots>(manager: &Manager) -> Managed<R> {
    manager
        .acquire_for_identity::<R>(&context(), &AcquireOptions::default(), &tenant())
        .await
        .expect("acquire")
        .into_managed()
}

fn reads(manager: &Manager) -> Arc<CredentialReads> {
    row::<StrictResident>(manager)
        .credential_reads
        .clone()
        .expect("strict row")
}

/// Observer reads the strict row's metrics counted, whatever they answered.
fn reads_total<R: Provider>(manager: &Manager) -> u64 {
    row::<R>(manager)
        .credential_reads
        .as_deref()
        .and_then(CredentialReads::metrics)
        .map_or(0, |metrics| metrics.reads().iter().sum())
}

fn suspended_for<R: Provider>(manager: &Manager) -> Option<CredentialUnavailableReason> {
    row::<R>(manager)
        .admission
        .suspension()
        .and_then(|suspension| suspension.reason_for("db"))
}

fn gate_epoch<R: Provider>(manager: &Manager) -> u64 {
    row::<R>(manager).admission.gate_epoch()
}

fn reason(error: &OpError) -> Option<CredentialUnavailableReason> {
    match error.kind() {
        ErrorKind::CredentialUnavailable { reason } => Some(*reason),
        _ => None,
    }
}

fn in_one(after: Duration) -> std::time::Instant {
    Instant::now().into_std() + after
}

/// Lets every task run until the runtime is idle (paused time only).
async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

// ── operations ───────────────────────────────────────────────────────────

/// One attempt at `cost`, settled `Sent`, on any row.
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

/// `n` attempts at `cost`, each settled `Sent`; yields what each attempt
/// was pinned on.
struct Attempts {
    n: u32,
    cost: Cost,
}

impl Attempts {
    fn free(n: u32) -> Self {
        Self {
            n,
            cost: Cost::FREE,
        }
    }
}

impl<R: Provider + PinSlots<Pinned = PinnedEpochs>> Operation<R> for Attempts {
    type Output = Vec<PinnedEpochs>;
    const EFFECT: Effect = Effect::Read;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(self.n).expect("at least one attempt")
    }

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<Self::Output, OpError> {
        let mut pinned = Vec::new();
        for _ in 0..self.n {
            let attempt = cx.attempt(self.cost.clone()).await?;
            pinned.push(attempt.slots().clone());
            attempt.settle(SentState::Sent);
        }
        Ok(pinned)
    }
}

/// Two free attempts; after the first is granted and settled `Sent` it
/// signals `between` and waits for `resume`.
struct Paused<const WRITE: bool> {
    between: Arc<Notify>,
    resume: Arc<Notify>,
}

struct Pause {
    between: Arc<Notify>,
    resume: Arc<Notify>,
}

fn paused<const WRITE: bool>() -> (Paused<WRITE>, Pause) {
    let pause = Pause {
        between: Arc::new(Notify::new()),
        resume: Arc::new(Notify::new()),
    };
    (
        Paused {
            between: Arc::clone(&pause.between),
            resume: Arc::clone(&pause.resume),
        },
        pause,
    )
}

impl<R: Provider + PinSlots<Pinned = PinnedEpochs>, const WRITE: bool> Operation<R>
    for Paused<WRITE>
{
    type Output = Vec<PinnedEpochs>;
    const EFFECT: Effect = if WRITE { Effect::Write } else { Effect::Read };

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

// ── zero reads ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_slot_less_row_on_a_strict_manager_reads_nothing_per_attempt() {
    let observer = ScriptedObserver::answering(Err(CredentialObserveError::Unavailable));
    let manager = strict_manager(erased(&observer), &Arc::default());
    register(
        &manager,
        UnboundRow(Arc::default()),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register");
    // A strict row whose slot was never bound reads nothing either.
    resident(&manager);

    let unbound = facade::<UnboundRow>(&manager).await;
    unbound.submit(Once(Cost::ONE)).await.expect("slot-less");
    unbound.submit(Once(Cost::FREE)).await.expect("slot-less");
    let never_bound = facade::<StrictResident>(&manager).await;
    never_bound
        .submit(Attempts::free(2))
        .await
        .expect("unbound slot");
    assert_eq!(observer.calls(), 0);
}

#[tokio::test]
async fn an_interim_manager_reads_nothing_per_attempt() {
    let manager = Manager::new();
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);

    let managed = facade::<StrictResident>(&manager).await;
    let pinned = managed.submit(Attempts::free(2)).await.expect("granted");
    assert_eq!(pinned, vec![vec![("db", Some(1))]; 2]);
    let row = row::<StrictResident>(&manager);
    assert!(row.credential_reads.is_none(), "nothing to read through");
    assert_eq!(
        row.credential_admission_profile(),
        CredentialAdmissionProfile::InterimRowGate
    );
}

// ── one read per attempt ─────────────────────────────────────────────────

#[tokio::test]
async fn every_attempt_reads_once() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    assert_eq!(observer.calls(), 1, "the acquire read");
    assert_eq!(reads_total::<StrictResident>(&manager), 1);

    let pinned = managed
        .submit(Attempts {
            n: 3,
            cost: Cost::ONE,
        })
        .await
        .expect("granted");
    assert_eq!(pinned, vec![vec![("db", Some(1))]; 3]);
    assert_eq!(observer.calls(), 1 + 3);
    assert_eq!(reads_total::<StrictResident>(&manager), 1 + 3);
}

#[tokio::test(start_paused = true)]
async fn the_read_runs_after_the_quota_wait() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, _resource) = setup_limited(&observer);
    // The acquire books the only permit; the attempt's slot is a second
    // later.
    let managed = facade::<StrictResident>(&manager).await;
    let started = Instant::now();
    let unit = tokio::spawn(managed.submit(Attempts {
        n: 1,
        cost: Cost::ONE,
    }));

    tokio::time::sleep_until(started + Duration::from_millis(999)).await;
    assert_eq!(observer.calls(), 1, "no read while the attempt waits");
    unit.await.expect("joined").expect("granted");
    assert_eq!(observer.calls(), 2);
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_reauthentication_during_the_quota_wait_refuses_and_suspends() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, _resource) = setup_limited(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let mut events = manager.subscribe_events();
    let unit = tokio::spawn(managed.submit(Once(Cost::ONE)));
    settle_tasks().await;

    observer.answer(reauth(1, 2));
    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(suspended_for::<StrictResident>(&manager), Some(REAUTH));
    assert!(managed.is_closing(), "the suspension closes the lease");
    let mut suspended = 0;
    while let Some(event) = events.try_recv() {
        if matches!(
            event,
            ResourceEvent::CredentialSuspended { reason, .. } if reason == REAUTH
        ) {
            suspended += 1;
        }
    }
    assert_eq!(suspended, 1);

    // The next unit is refused by the closed lease before it reads.
    let calls = observer.calls();
    let error = managed
        .submit(Once(Cost::FREE))
        .await
        .expect_err("closed lease");
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(observer.calls(), calls, "zero reads");
}

// ── the read runs outside the lock and races the lease ───────────────────

#[tokio::test]
async fn the_admission_lock_is_not_held_across_an_attempt_read() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let unit = tokio::spawn(managed.submit(Once(Cost::FREE)));
    observer.until_calls(2).await;
    // Completes while the attempt's read is gated.
    manager
        .suspend_credential_row(
            &StrictResident::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            CredentialUnavailableReason::OperationBlocked,
            None,
        )
        .expect("suspend");
    observer.release(1);

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(
        reason(&error),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert_eq!(error.sent(), SentState::NotSent);
}

#[tokio::test]
async fn a_taint_during_an_attempt_read_refuses_as_revoked() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let unit = tokio::spawn(managed.submit(Once(Cost::FREE)));
    observer.until_calls(2).await;
    let _tainted = manager
        .taint_slot_for_identity(&StrictResident::key(), ScopeLevel::Global, "db", &tenant())
        .expect("taint");
    observer.release(1);

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(*error.kind(), ErrorKind::Revoked);
    assert_eq!(error.sent(), SentState::NotSent);
}

#[tokio::test]
async fn a_lease_closing_during_an_attempt_read_refuses_as_cancelled() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let reads = reads(&manager);

    let unit = tokio::spawn(managed.submit(Once(Cost::FREE)));
    observer.until_calls(2).await;
    manager.remove(&StrictResident::key()).expect("remove");

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(reads.lane_count(), 0, "the read was dropped");
}

#[tokio::test]
async fn a_cancel_during_the_first_read_ends_it_and_a_cancel_after_the_grant_is_ignored() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let mut unit = managed.submit(Attempts::free(1));
    assert!(futures::poll!(&mut unit).is_pending());
    observer.until_calls(2).await;
    unit.cancel();
    let error = unit.await.expect_err("cancelled");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(reads(&manager).lane_count(), 0, "the read was dropped");

    // After the first grant a cancel is ignored: the second read runs.
    let (operation, pause) = paused::<false>();
    let mut unit = managed.submit(operation);
    assert!(futures::poll!(&mut unit).is_pending());
    observer.release(1);
    pause.between.notified().await;
    unit.cancel();
    pause.resume.notify_one();
    observer.release(1);
    let pinned = unit.await.expect("granted twice");
    assert_eq!(pinned.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_read_that_never_answers_refuses_at_its_own_bound() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let started = Instant::now();
    let error = managed
        .submit(Attempts::free(1))
        .with_deadline(in_one(Duration::from_secs(5)))
        .await
        .expect_err("unanswered");
    assert_eq!(
        reason(&error),
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(started.elapsed(), CREDENTIAL_READ_TIMEOUT);
}

// ── material ─────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_refresh_in_flight_is_joined_then_admits_or_rebinds() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    observer.then([refreshing(1), refreshing(1)]);
    managed
        .submit(Attempts::free(1))
        .await
        .expect("the refresh left the material usable");
    assert_eq!(observer.calls(), 1 + 3, "two joined re-reads");

    observer.then([refreshing(1)]);
    observer.answer(available(2, 2));
    let error = managed
        .submit(Attempts::free(1))
        .await
        .expect_err("new material");
    assert_eq!(reason(&error), Some(REBINDING));
    assert_eq!(error.sent(), SentState::NotSent);
}

#[tokio::test]
async fn newer_observed_material_refuses_without_touching_the_gate() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let epoch = gate_epoch::<StrictResident>(&manager);

    observer.answer(available(2, 2));
    let error = managed
        .submit(Attempts::free(1))
        .await
        .expect_err("not installed yet");
    assert_eq!(reason(&error), Some(REBINDING));
    assert_eq!(suspended_for::<StrictResident>(&manager), None);
    assert_eq!(gate_epoch::<StrictResident>(&manager), epoch);
    assert!(!managed.is_closing());
}

#[tokio::test(start_paused = true)]
async fn a_first_attempt_runs_on_material_installed_during_its_wait() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, resource) = setup_limited(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let unit = tokio::spawn(managed.submit(Attempts {
        n: 1,
        cost: Cost::ONE,
    }));
    settle_tasks().await;

    bind(&resource.db, credential_id(), 2, 2);
    observer.answer(available(2, 2));
    let pinned = unit.await.expect("joined").expect("granted");
    assert_eq!(
        pinned,
        vec![vec![("db", Some(2))]],
        "pinned after the read that validated it"
    );
}

#[tokio::test]
async fn a_later_attempt_whose_pin_was_superseded_is_refused_rebinding() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let (operation, pause) = paused::<false>();
    let unit = tokio::spawn(managed.submit(operation));
    pause.between.notified().await;
    bind(&resource.db, credential_id(), 2, 2);
    observer.answer(available(2, 2));
    pause.resume.notify_one();

    let error = unit.await.expect("joined").expect_err("superseded pin");
    assert_eq!(reason(&error), Some(REBINDING));
    assert_eq!(error.sent(), SentState::Sent, "the first attempt was sent");
    assert!(error.is_retryable(), "a read unit is retried");
    assert!(!managed.is_closing(), "a rebinding closes nothing");

    // The next unit pins the new material.
    let pinned = managed.submit(Attempts::free(1)).await.expect("granted");
    assert_eq!(pinned, vec![vec![("db", Some(2))]]);
}

#[tokio::test]
async fn a_write_whose_later_attempt_rebinds_has_an_unknown_outcome() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let (manager, resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let mut events = manager.subscribe_events();

    let (operation, pause) = paused::<true>();
    let unit = tokio::spawn(managed.submit(operation));
    pause.between.notified().await;
    bind(&resource.db, credential_id(), 2, 2);
    observer.answer(available(2, 2));
    pause.resume.notify_one();

    let error = unit.await.expect("joined").expect_err("superseded pin");
    assert_eq!(reason(&error), Some(REBINDING));
    assert!(!error.is_retryable());
    assert_eq!(*Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    let mut unknown = 0;
    while let Some(event) = events.try_recv() {
        if matches!(event, ResourceEvent::UnitOutcomeUnknown { .. }) {
            unknown += 1;
        }
    }
    assert_eq!(unknown, 1);
}

// ── coalescing, shutdown, pooled leases ──────────────────────────────────

#[tokio::test]
async fn attempts_of_concurrent_units_join_the_next_read_only() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;
    let reads = reads(&manager);

    let first = tokio::spawn(managed.submit(Once(Cost::FREE)));
    observer.until_calls(2).await;
    // A block commits after the first read started: the later units must
    // not take that read's answer.
    observer.answer(reauth(1, 2));
    let rest: Vec<_> = (0..7)
        .map(|_| tokio::spawn(managed.submit(Once(Cost::FREE))))
        .collect();
    while reads.users(credential_id()) < 8 {
        tokio::task::yield_now().await;
    }

    observer.release(1);
    first.await.expect("joined").expect("read before the block");
    observer.release(1);
    for unit in rest {
        let error = unit
            .await
            .expect("joined")
            .expect_err("read after the block");
        assert_eq!(reason(&error), Some(REAUTH));
    }
    assert_eq!(observer.calls(), 1 + 2, "eight attempts, two reads");
    assert_eq!(suspended_for::<StrictResident>(&manager), Some(REAUTH));
}

#[tokio::test]
async fn a_shutdown_during_an_attempt_read_refuses_as_cancelled() {
    let observer = ScriptedObserver::gated(available(1, 1));
    observer.release(1);
    let (manager, _resource) = setup(&observer);
    let managed = facade::<StrictResident>(&manager).await;

    let unit = tokio::spawn(managed.submit(Once(Cost::FREE)));
    observer.until_calls(2).await;
    let shutdown = {
        let manager = Arc::clone(&manager);
        tokio::spawn(async move { manager.graceful_shutdown(ShutdownConfig::default()).await })
    };

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    drop(managed);
    let _report = shutdown.await.expect("joined");
}

#[tokio::test]
async fn a_pooled_lease_reads_per_attempt_and_a_block_closes_it() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = register(
        &manager,
        StrictPooled::new(),
        Pooled::new(pool_config(), config(1).fingerprint()),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    let managed = facade::<StrictPooled>(&manager).await;

    managed.submit(Attempts::free(2)).await.expect("granted");
    assert_eq!(observer.calls(), 1 + 2);

    observer.answer(reauth(1, 2));
    let error = managed
        .submit(Attempts::free(1))
        .await
        .expect_err("blocked");
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(observer.calls(), 1 + 3);
    assert!(managed.is_closing());
    assert_eq!(suspended_for::<StrictPooled>(&manager), Some(REAUTH));
}
