//! Strict per-acquire credential admission: every acquire reads its bound
//! credentials first, outside the admission lock, and applies what it saw
//! under it (invariant I7).

use std::{sync::Arc, time::Duration};

use nebula_core::ScopeLevel;
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialObserveError, CredentialOperationKind, REFRESH_JOIN_WAIT,
};

use super::{SlotOutcome, decide};
use crate::{
    AcquireOptions, CredentialUnavailableReason, Error, ErrorKind, Pooled, Provider,
    RegistrationSpec, Resident, ResidentConfig, ResourceEvent, ResourceGuard,
    manager::{
        CredentialObservedAt, CredentialReopenOutcome, Manager,
        credential_reads::{
            CREDENTIAL_READ_TIMEOUT, CredentialAdmissionMetrics, Published, ReadFailure,
            tests::{ScriptedObserver, seen},
        },
        strict_fixtures::{
            StrictBounded, StrictPooled, StrictResident, StrictTwoSlot, UnboundRow, bind,
            cache_credential_id, config, context, credential_id, guard, pool_config, register,
            resident, row, strict_manager, tenant,
        },
    },
    recovery::{GateState, RecoveryGate, RecoveryGateConfig},
    resource::ResourceConfig as _,
    runtime::admission::UseMark,
};

const REAUTH: CredentialUnavailableReason = CredentialUnavailableReason::ReauthRequired;
const BLOCKED: CredentialUnavailableReason = CredentialUnavailableReason::OperationBlocked;

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

/// A revoke in flight: no use revision, as the real observer reports it.
fn revoking(material: u64) -> Published {
    Ok(CredentialAvailabilityObservation::new(
        material,
        1,
        CredentialAvailability::Blocked(CredentialBlock::OperationInFlight {
            operation: CredentialOperationKind::Revoke,
        }),
    ))
}

fn erased(observer: &Arc<ScriptedObserver>) -> Arc<dyn CredentialAvailabilityObserver> {
    Arc::clone(observer) as Arc<dyn CredentialAvailabilityObserver>
}

/// A strict manager with a resident row bound at `(1, 1)`.
fn setup(answer: Published) -> (Arc<ScriptedObserver>, Manager, StrictResident) {
    let observer = ScriptedObserver::answering(answer);
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    (observer, manager, resource)
}

fn metrics(manager: &Manager) -> &CredentialAdmissionMetrics {
    manager
        .credential_reads
        .as_deref()
        .and_then(super::CredentialReads::metrics)
        .expect("strict manager with metrics")
}

async fn acquire<R: Provider>(manager: &Manager) -> Result<ResourceGuard<R>, Error> {
    manager
        .acquire_for_identity::<R>(&context(), &AcquireOptions::default(), &tenant())
        .await
}

/// The error of an operation that must have been refused.
trait Refused {
    fn refused(self) -> Error;
}

impl<T> Refused for Result<T, Error> {
    fn refused(self) -> Error {
        match self {
            Ok(_) => panic!("expected a refusal"),
            Err(error) => error,
        }
    }
}

fn reason(error: &Error) -> Option<CredentialUnavailableReason> {
    match error.kind() {
        ErrorKind::CredentialUnavailable { reason } => Some(*reason),
        _ => None,
    }
}

async fn refused<R: Provider>(manager: &Manager) -> Option<CredentialUnavailableReason> {
    reason(&acquire::<R>(manager).await.refused())
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

fn current_seq<R: Provider>(manager: &Manager) -> Option<u64> {
    row::<R>(manager)
        .admission
        .current()
        .map(|generation| generation.seq())
}

fn spawn_acquire<R: Provider>(
    manager: &Arc<Manager>,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    let manager = Arc::clone(manager);
    tokio::spawn(async move { acquire::<R>(&manager).await.map(drop) })
}

// ---------------------------------------------------------------------------
// Decision table (pure).
// ---------------------------------------------------------------------------

fn observed(answer: Published) -> SlotOutcome {
    SlotOutcome::Observed(answer.expect("an observation"))
}

#[test]
fn the_decision_table_maps_each_read_to_one_refusal() {
    let installed = |_: &str| Some(UseMark::new(2, 5));
    let cases: [(SlotOutcome, Option<CredentialUnavailableReason>); 10] = [
        (observed(available(2, 5)), None),
        (observed(available(2, 9)), None),
        (
            observed(available(3, 1)),
            Some(CredentialUnavailableReason::Rebinding),
        ),
        (
            observed(available(1, 9)),
            Some(CredentialUnavailableReason::CheckUnavailable),
        ),
        (
            observed(refreshing(2)),
            Some(CredentialUnavailableReason::RefreshInFlight),
        ),
        (
            observed(refreshing(3)),
            Some(CredentialUnavailableReason::Rebinding),
        ),
        (observed(reauth(2, 6)), Some(REAUTH)),
        (observed(revoking(2)), Some(BLOCKED)),
        (
            SlotOutcome::Failed(ReadFailure::Observe(
                CredentialObserveError::WrongCredentialKey,
            )),
            Some(CredentialUnavailableReason::Absent),
        ),
        (
            SlotOutcome::Failed(ReadFailure::TimedOut),
            Some(CredentialUnavailableReason::CheckUnavailable),
        ),
    ];
    for (outcome, expected) in cases {
        let verdict = decide(&[("db", outcome)], installed);
        assert_eq!(verdict.deny, expected, "{outcome:?}");
        assert!(!verdict.cancelled);
    }
    // Blocked reads carry what to suspend: a reauthentication with its use
    // revision (witnessed), an operation without.
    let verdict = decide(&[("db", observed(reauth(2, 6)))], installed);
    assert_eq!(
        verdict.suspend,
        vec![(
            "db",
            REAUTH,
            CredentialObservedAt::new(2).with_admission_epoch(6)
        )]
    );
    let verdict = decide(&[("db", observed(revoking(2)))], installed);
    assert_eq!(
        verdict.suspend,
        vec![("db", BLOCKED, CredentialObservedAt::new(2))]
    );
    // A slot no longer observable, or a shutdown.
    assert_eq!(
        decide(&[("db", observed(available(2, 5)))], |_| None).deny,
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert!(
        decide(
            &[("db", SlotOutcome::Failed(ReadFailure::Cancelled))],
            installed
        )
        .cancelled
    );
}

#[test]
fn several_refusing_slots_report_the_highest_priority_reason() {
    let installed = |_: &str| Some(UseMark::new(1, 1));
    let order = [
        (
            SlotOutcome::Failed(ReadFailure::Observe(CredentialObserveError::Absent)),
            CredentialUnavailableReason::Absent,
        ),
        (observed(reauth(1, 2)), REAUTH),
        (observed(revoking(1)), BLOCKED),
        (
            observed(available(2, 1)),
            CredentialUnavailableReason::Rebinding,
        ),
        (
            observed(refreshing(1)),
            CredentialUnavailableReason::RefreshInFlight,
        ),
        (
            SlotOutcome::Unobservable,
            CredentialUnavailableReason::CheckUnavailable,
        ),
    ];
    for (index, (_, expected)) in order.iter().enumerate() {
        // The refusal beside every lower-priority one, listed last.
        let slots: Vec<_> = order[index..]
            .iter()
            .rev()
            .map(|(outcome, _)| ("db", *outcome))
            .collect();
        assert_eq!(decide(&slots, installed).deny, Some(*expected));
    }
    // Every blocked slot is suspended, whatever wins.
    let verdict = decide(
        &[
            ("db", observed(reauth(1, 2))),
            ("cache", observed(revoking(1))),
        ],
        installed,
    );
    assert_eq!(verdict.suspend.len(), 2);
}

// ---------------------------------------------------------------------------
// Acquire.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_row_without_bound_credentials_reads_nothing_even_with_a_failing_observer() {
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

    drop(acquire::<UnboundRow>(&manager).await.expect("slot-less"));
    drop(
        acquire::<StrictResident>(&manager)
            .await
            .expect("unbound slot"),
    );
    assert_eq!(observer.calls(), 0);
}

#[tokio::test]
async fn an_available_credential_admits_after_one_read_per_slot() {
    let (observer, manager, _resource) = setup(available(1, 1));
    let epoch = gate_epoch::<StrictResident>(&manager);
    let seq = current_seq::<StrictResident>(&manager);

    let guard = acquire::<StrictResident>(&manager).await.expect("admitted");
    assert_eq!(observer.calls(), 1);
    assert!(!guard.is_closing());
    assert_eq!(
        gate_epoch::<StrictResident>(&manager),
        epoch,
        "no gate change"
    );
    assert_eq!(current_seq::<StrictResident>(&manager), seq);
    assert_eq!(metrics(&manager).reads(), [1, 0, 0, 0, 0, 0]);
}

#[tokio::test]
async fn two_bound_slots_are_read_concurrently() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = Arc::new(strict_manager(erased(&observer), &Arc::default()));
    let resource = register(
        &manager,
        StrictTwoSlot::new(),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    bind(&resource.cache, cache_credential_id(), 1, 1);

    let pending = spawn_acquire::<StrictTwoSlot>(&manager);
    // Both reads are in flight at once.
    observer.until_calls(2).await;
    observer.release(2);
    pending.await.expect("joined").expect("admitted");
    assert_eq!(observer.calls(), 2);
}

#[tokio::test]
async fn a_denying_slot_beside_an_absent_one_is_suspended_and_absent_is_reported() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = register(
        &manager,
        StrictTwoSlot::new(),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    bind(&resource.cache, cache_credential_id(), 1, 1);
    observer.answer_for(credential_id(), reauth(1, 2));
    observer.answer_for(cache_credential_id(), Err(CredentialObserveError::Absent));

    assert_eq!(
        refused::<StrictTwoSlot>(&manager).await,
        Some(CredentialUnavailableReason::Absent)
    );
    assert_eq!(suspended_for::<StrictTwoSlot>(&manager), Some(REAUTH));
}

#[tokio::test]
async fn a_reauthentication_read_suspends_the_row_and_closes_earlier_leases() {
    let (observer, manager, _resource) = setup(available(1, 1));
    let earlier = acquire::<StrictResident>(&manager).await.expect("admitted");
    let mut events = manager.subscribe_events();
    let epoch = gate_epoch::<StrictResident>(&manager);

    // No activation, no fan-out: only the acquire observes the flag.
    observer.answer(reauth(1, 2));
    assert_eq!(refused::<StrictResident>(&manager).await, Some(REAUTH));

    assert_eq!(suspended_for::<StrictResident>(&manager), Some(REAUTH));
    assert!(earlier.is_closing(), "the admitted lease observes closing");
    assert_eq!(gate_epoch::<StrictResident>(&manager), epoch + 1);
    let mut suspended = 0;
    while let Some(event) = events.try_recv() {
        if matches!(event, ResourceEvent::CredentialSuspended { reason, .. } if reason == REAUTH) {
            suspended += 1;
        }
    }
    assert_eq!(suspended, 1);
    assert_eq!(metrics(&manager).denied(REAUTH), 1);

    // The same use revision is a lagging read: still refused.
    observer.answer(available(1, 2));
    assert_eq!(refused::<StrictResident>(&manager).await, Some(REAUTH));
    // Reauthentication advanced the revision: the acquire reopens the row.
    let mut events = manager.subscribe_events();
    observer.answer(available(1, 3));
    let reopened = acquire::<StrictResident>(&manager).await.expect("reopened");
    assert!(!reopened.is_closing());
    assert_eq!(suspended_for::<StrictResident>(&manager), None);
    assert!(
        std::iter::from_fn(|| events.try_recv())
            .any(|event| matches!(event, ResourceEvent::CredentialReopened { .. }))
    );
}

#[tokio::test]
async fn a_strict_refusal_never_touches_the_recovery_gate() {
    let observer = ScriptedObserver::answering(reauth(1, 2));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));
    let resource = StrictResident::new();
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: Some(Arc::clone(&gate)),
            rate_limit: None,
        })
        .expect("register");
    bind(&resource.db, credential_id(), 1, 1);

    assert_eq!(refused::<StrictResident>(&manager).await, Some(REAUTH));
    assert!(matches!(gate.state(), GateState::Idle));
}

#[tokio::test]
async fn an_operation_block_read_suspends_and_a_usable_read_at_the_admitted_revision_reopens() {
    let (observer, manager, _resource) = setup(available(1, 1));
    let earlier = acquire::<StrictResident>(&manager).await.expect("admitted");
    let before = current_seq::<StrictResident>(&manager).expect("current");

    observer.answer(revoking(1));
    assert_eq!(refused::<StrictResident>(&manager).await, Some(BLOCKED));
    assert_eq!(suspended_for::<StrictResident>(&manager), Some(BLOCKED));
    assert!(earlier.is_closing());

    // The revoke claim lapsed; the backend bumped the revision when the
    // block began, and a denial read without one clears at the admitted
    // revision.
    observer.answer(available(1, 1));
    let reopened = acquire::<StrictResident>(&manager).await.expect("reopened");
    let after = current_seq::<StrictResident>(&manager).expect("current");
    assert!(after > before, "a fresh generation");
    assert!(!reopened.is_closing());
    assert!(earlier.is_closing(), "the earlier lease stays closed");
}

#[tokio::test]
async fn a_missed_denial_interval_readmits_without_closing_earlier_leases() {
    let (observer, manager, _resource) = setup(available(1, 1));
    let earlier = acquire::<StrictResident>(&manager).await.expect("admitted");
    let before = current_seq::<StrictResident>(&manager).expect("current");
    let epoch = gate_epoch::<StrictResident>(&manager);

    // Use closed and reopened between two acquires: the revision advanced.
    observer.answer(available(1, 3));
    let readmitted = acquire::<StrictResident>(&manager)
        .await
        .expect("readmitted");
    let after = current_seq::<StrictResident>(&manager).expect("current");
    assert!(after > before, "new work runs under a fresh generation");
    assert!(!earlier.is_closing(), "readmission never closes");
    assert!(!readmitted.is_closing());

    // The next acquire at the same revision changes nothing.
    drop(acquire::<StrictResident>(&manager).await.expect("admitted"));
    assert_eq!(current_seq::<StrictResident>(&manager), Some(after));
    assert_eq!(gate_epoch::<StrictResident>(&manager), epoch);
}

#[tokio::test]
async fn newer_material_refuses_until_it_is_installed() {
    let (observer, manager, resource) = setup(available(2, 2));
    assert_eq!(
        refused::<StrictResident>(&manager).await,
        Some(CredentialUnavailableReason::Rebinding)
    );
    assert_eq!(
        resource.probe.creates(),
        0,
        "nothing ran on the old material"
    );
    assert_eq!(suspended_for::<StrictResident>(&manager), None);

    let _installed = manager
        .install_and_refresh_slot_for_identity(
            &StrictResident::key(),
            ScopeLevel::Global,
            "db",
            &tenant(),
            guard(2, 2),
        )
        .await
        .expect("install");
    drop(acquire::<StrictResident>(&manager).await.expect("admitted"));
    assert_eq!(observer.calls(), 2);
}

#[tokio::test]
async fn older_material_than_installed_refuses_as_unchecked() {
    let (_observer, manager, resource) = setup(available(1, 1));
    bind(&resource.db, credential_id(), 3, 3);
    assert_eq!(
        refused::<StrictResident>(&manager).await,
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert_eq!(suspended_for::<StrictResident>(&manager), None);
}

#[tokio::test]
async fn a_store_outage_refuses_without_suspending_or_tripping_the_recovery_gate() {
    for failure in [
        CredentialObserveError::Unavailable,
        CredentialObserveError::SourceUnavailable,
        CredentialObserveError::InvalidState,
    ] {
        let observer = ScriptedObserver::answering(Err(failure));
        let manager = strict_manager(erased(&observer), &Arc::default());
        let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));
        let resource = StrictResident::new();
        manager
            .register(RegistrationSpec {
                resource: resource.clone(),
                config: config(1),
                scope: ScopeLevel::Global,
                slot_identity: tenant(),
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: Some(Arc::clone(&gate)),
                rate_limit: None,
            })
            .expect("register");
        bind(&resource.db, credential_id(), 1, 1);
        let epoch = gate_epoch::<StrictResident>(&manager);

        for _ in 0..50 {
            assert_eq!(
                refused::<StrictResident>(&manager).await,
                Some(CredentialUnavailableReason::CheckUnavailable)
            );
        }
        assert_eq!(suspended_for::<StrictResident>(&manager), None);
        assert_eq!(gate_epoch::<StrictResident>(&manager), epoch);
        assert!(matches!(gate.state(), GateState::Idle));
        assert_eq!(resource.probe.creates(), 0);
        assert_eq!(
            metrics(&manager).denied(CredentialUnavailableReason::CheckUnavailable),
            50
        );
    }
}

#[tokio::test]
async fn an_absent_credential_refuses_as_absent() {
    for failure in [
        CredentialObserveError::Absent,
        CredentialObserveError::WrongCredentialKey,
    ] {
        let (_observer, manager, _resource) = setup(Err(failure));
        assert_eq!(
            refused::<StrictResident>(&manager).await,
            Some(CredentialUnavailableReason::Absent)
        );
        assert_eq!(suspended_for::<StrictResident>(&manager), None);
    }
}

#[tokio::test(start_paused = true)]
async fn a_read_that_does_not_answer_refuses_at_the_earlier_of_its_bound_and_the_deadline() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);

    let started = tokio::time::Instant::now();
    assert_eq!(
        refused::<StrictResident>(&manager).await,
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert_eq!(started.elapsed(), CREDENTIAL_READ_TIMEOUT);

    let started = tokio::time::Instant::now();
    let error = manager
        .acquire_for_identity::<StrictResident>(
            &context(),
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + Duration::from_millis(500)),
            &tenant(),
        )
        .await
        .refused();
    assert_eq!(
        reason(&error),
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert!(started.elapsed() <= Duration::from_millis(500));
    assert!(started.elapsed() > Duration::from_millis(400));
}

#[tokio::test(start_paused = true)]
async fn a_refresh_in_flight_is_joined_for_a_bounded_wait() {
    let (observer, manager, _resource) = setup(refreshing(1));
    let started = tokio::time::Instant::now();
    assert_eq!(
        refused::<StrictResident>(&manager).await,
        Some(CredentialUnavailableReason::RefreshInFlight)
    );
    // Pauses 25, 50, 100, 200 ms, then 400 ms while the next still fits in
    // the join wait: 4 + 11 re-reads after the first.
    assert_eq!(observer.calls(), 16);
    assert_eq!(started.elapsed(), Duration::from_millis(4775));
    assert!(started.elapsed() <= REFRESH_JOIN_WAIT);
    assert_eq!(metrics(&manager).reads()[1], 16);
}

#[tokio::test(start_paused = true)]
async fn a_refresh_that_settles_mid_join_admits_or_rebinds() {
    let (observer, manager, _resource) = setup(available(1, 1));
    observer.then([refreshing(1), refreshing(1)]);
    drop(
        acquire::<StrictResident>(&manager)
            .await
            .expect("left the material usable"),
    );
    assert_eq!(observer.calls(), 3);

    observer.then([refreshing(1)]);
    observer.answer(available(2, 2));
    assert_eq!(
        refused::<StrictResident>(&manager).await,
        Some(CredentialUnavailableReason::Rebinding),
        "the refresh committed new material the row has not installed"
    );
}

#[tokio::test]
async fn dropping_an_acquire_mid_read_leaves_nothing_behind() {
    let observer = ScriptedObserver::gated(reauth(1, 2));
    let manager = Arc::new(strict_manager(erased(&observer), &Arc::default()));
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);

    let pending = spawn_acquire::<StrictResident>(&manager);
    observer.until_calls(1).await;
    pending.abort();
    assert!(pending.await.expect_err("aborted").is_cancelled());

    assert_eq!(row::<StrictResident>(&manager).in_flight_count(), 0);
    assert_eq!(
        manager
            .drain_tracker
            .0
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        manager
            .credential_reads
            .as_deref()
            .expect("strict")
            .lane_count(),
        0
    );
    assert_eq!(suspended_for::<StrictResident>(&manager), None);
}

#[tokio::test]
async fn the_admission_lock_is_not_held_across_the_read() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = Arc::new(strict_manager(erased(&observer), &Arc::default()));
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);

    let pending = spawn_acquire::<StrictResident>(&manager);
    observer.until_calls(1).await;
    // While the read is gated, every other `Manager.admission` user
    // progresses: a suspension, a reload, a registration.
    manager
        .suspend_credential_row(
            &StrictResident::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            BLOCKED,
            None,
        )
        .expect("suspend");
    manager
        .reload_config::<StrictResident>(config(2), &ScopeLevel::Global)
        .expect("reload");
    register(
        &manager,
        UnboundRow(Arc::default()),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register");

    // The read saw the credential usable, but a suspension landed after its
    // ticket: the reopen is superseded and the acquire refused.
    observer.release(1);
    let error = pending.await.expect("joined").refused();
    assert_eq!(reason(&error), Some(BLOCKED));
    assert_eq!(suspended_for::<StrictResident>(&manager), Some(BLOCKED));
    // A fresh ticket reopens as before.
    let ticket = manager
        .credential_gate_ticket(&StrictResident::key(), &ScopeLevel::Global, &tenant())
        .expect("ticket");
    assert_eq!(
        manager
            .reopen_credential_row(
                &StrictResident::key(),
                &ScopeLevel::Global,
                &tenant(),
                "db",
                ticket,
                CredentialObservedAt::new(1).with_admission_epoch(1),
            )
            .expect("reopen"),
        CredentialReopenOutcome::Reopened
    );
}

#[tokio::test]
async fn a_taint_during_the_read_refuses_as_revoked() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = Arc::new(strict_manager(erased(&observer), &Arc::default()));
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);

    let pending = spawn_acquire::<StrictResident>(&manager);
    observer.until_calls(1).await;
    let _tainted = manager
        .taint_slot_for_identity(&StrictResident::key(), ScopeLevel::Global, "db", &tenant())
        .expect("taint");
    observer.release(1);
    let error = pending.await.expect("joined").refused();
    assert_eq!(*error.kind(), ErrorKind::Revoked);
}

#[tokio::test]
async fn a_create_straddling_a_block_read_by_another_acquire_is_refused_at_hand_out() {
    let (observer, manager, resource) = setup(available(1, 1));
    let manager = Arc::new(manager);
    resource.probe.park_next_create();
    let first = spawn_acquire::<StrictResident>(&manager);
    resource.probe.create_entered.notified().await;

    // A second acquire reads the block and suspends the row while the
    // first one's create is still running.
    observer.answer(reauth(1, 2));
    assert_eq!(refused::<StrictResident>(&manager).await, Some(REAUTH));
    resource.probe.release_create.notify_one();

    let error = first.await.expect("joined").refused();
    assert_eq!(reason(&error), Some(REAUTH));
}

#[tokio::test]
async fn a_pooled_row_reads_before_checkout_and_keeps_its_idle_entry_when_refused() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = register(
        &manager,
        StrictPooled::new(),
        Pooled::new(pool_config(), config(1).fingerprint()),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    drop(acquire::<StrictPooled>(&manager).await.expect("admitted"));
    let row = row::<StrictPooled>(&manager);
    assert_eq!(resource.probe.creates(), 1);
    // The release returns the entry through the release queue.
    while row.store.len().await != 1 {
        tokio::task::yield_now().await;
    }

    observer.answer(reauth(1, 2));
    assert_eq!(refused::<StrictPooled>(&manager).await, Some(REAUTH));
    assert_eq!(row.store.len().await, 1, "the idle entry is kept for reuse");
    assert_eq!(resource.probe.creates(), 1, "nothing created");
    // A suspended row's maintenance keeps it, unprobed.
    row.run_maintenance().await;
    assert_eq!(row.store.len().await, 1);
}

#[tokio::test]
async fn an_explicit_warmup_reads_first_and_creates_nothing_when_refused() {
    let observer = ScriptedObserver::answering(reauth(1, 2));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = register(
        &manager,
        StrictPooled::new(),
        Pooled::new(pool_config(), config(1).fingerprint()),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);

    let error = manager
        .warmup_pool::<StrictPooled>(&context())
        .await
        .refused();
    assert_eq!(reason(&error), Some(REAUTH));
    assert_eq!(resource.probe.creates(), 0);

    // Usable again (reauthenticated): the warmup's read reopens the row and
    // the warmup proceeds under the pool's strategy (none here).
    observer.answer(available(1, 3));
    manager
        .warmup_pool::<StrictPooled>(&context())
        .await
        .expect("admitted");
    assert_eq!(suspended_for::<StrictPooled>(&manager), None);
}

#[tokio::test]
async fn a_bounded_row_is_strict_too() {
    let observer = ScriptedObserver::answering(revoking(1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = register(
        &manager,
        StrictBounded::new(),
        crate::Bounded::capped(2).expect("cap"),
    )
    .expect("register");
    bind(&resource.db, credential_id(), 1, 1);
    assert_eq!(refused::<StrictBounded>(&manager).await, Some(BLOCKED));
    assert_eq!(resource.probe.creates(), 0);
}

#[tokio::test]
async fn an_interim_manager_reads_nothing() {
    let manager = Manager::new();
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    assert!(manager.credential_reads.is_none());
    drop(acquire::<StrictResident>(&manager).await.expect("admitted"));
}

#[tokio::test]
async fn an_install_reopens_a_row_a_strict_read_suspended() {
    let (observer, manager, _resource) = setup(reauth(1, 2));
    assert_eq!(refused::<StrictResident>(&manager).await, Some(REAUTH));
    let ticket = manager
        .credential_gate_ticket(&StrictResident::key(), &ScopeLevel::Global, &tenant())
        .expect("ticket");
    // Reauthentication produced new material; the fan-out installs it with
    // the ticket it captured before resolving.
    let managed = manager
        .lookup_any_for_slot_identity_structural(
            &StrictResident::key(),
            &ScopeLevel::Global,
            &tenant(),
        )
        .expect("row");
    let _installed = manager
        .install_and_refresh_resolved(
            &StrictResident::key(),
            "db",
            managed,
            guard(2, 3),
            crate::manager::rotation::ResolvedAt {
                slot_generation: None,
                gate_ticket: Some(ticket),
            },
            (|| Ok(()), || {}, || {}),
        )
        .await
        .expect("install");
    assert_eq!(suspended_for::<StrictResident>(&manager), None);
    observer.answer(available(2, 3));
    drop(acquire::<StrictResident>(&manager).await.expect("admitted"));
}

#[tokio::test]
async fn a_burst_of_acquires_shares_reads() {
    let observer = ScriptedObserver::gated(available(1, 1));
    let manager = Arc::new(strict_manager(erased(&observer), &Arc::default()));
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    let reads = manager.credential_reads.clone().expect("strict");

    let burst: Vec<_> = (0..8)
        .map(|_| spawn_acquire::<StrictResident>(&manager))
        .collect();
    while reads.users(credential_id()) < 8 {
        tokio::task::yield_now().await;
    }
    observer.open();
    for acquire in burst {
        acquire.await.expect("joined").expect("admitted");
    }
    // One read was in flight when the other seven arrived; they share one
    // more.
    assert_eq!(metrics(&manager).reads()[0], 2);
    assert_eq!(metrics(&manager).joined(), 6);
}
