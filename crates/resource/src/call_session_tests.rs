//! Sessions on a managed row: one unit, one attempt, one checkout; the
//! settled outcome follows what the provider said at close, an abandoned
//! session destroys its instance, and a connection-bound session never runs
//! on an instance built on superseded credentials.

use std::{
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nebula_core::ScopeLevel;
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialObserveError,
};
use tokio::{sync::Notify, time::Instant};

use super::super::{
    Cost, Effect, ManagedRow, OpError, PinSlots, SentState, SessionClosed, SessionSpec, Unit,
};
use crate::{
    CredentialUnavailableReason, Error, ErrorKind, Manager, PoolConfig, Pooled, Provider,
    RegistrationSpec, ResourceEvent,
    manager::strict_fixtures::{
        ScriptedObserver, StrictPooled, StrictPooledSession, bind, config, context, credential_id,
        row, seen, strict_manager, tenant,
    },
    rate_limit::{Rate, RowLimit},
    resource::ResourceConfig as _,
    topology::pooled::config::WarmupStrategy,
};

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

fn pool() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 1,
        idle_timeout: None,
        max_lifetime: None,
        warmup: WarmupStrategy::None,
        maintenance_interval: Duration::from_hours(1),
        ..PoolConfig::default()
    }
}

/// A one-connection pooled row of `R`, bound at `(1, 1)`.
fn register<R>(manager: &Manager, resource: R, limit: Option<RowLimit>) -> R
where
    R: Provider<Config = crate::manager::strict_fixtures::Config, Topology = Pooled<R>>
        + crate::topology::PoolProvider
        + Clone,
{
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: config(1),
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Pooled::new(pool(), config(1).fingerprint()),
            recovery_gate: None,
            rate_limit: limit,
        })
        .expect("register");
    resource
}

fn pooled(manager: &Manager) -> StrictPooled {
    let resource = register(manager, StrictPooled::new(), None);
    bind(&resource.db, credential_id(), 1, 1);
    resource
}

fn facade<R: Provider + PinSlots>(manager: &Manager) -> ManagedRow<R> {
    manager
        .managed_row_for_identity::<R>(&context(), &tenant())
        .expect("row facade")
}

fn write() -> SessionSpec {
    SessionSpec::new(Cost::ONE)
}

fn read() -> SessionSpec {
    SessionSpec::new(Cost::ONE).with_effect(Effect::Read)
}

/// Yields until the row has no lease out and returns its idle entries.
async fn idle_after_release<R: Provider>(manager: &Manager) -> usize {
    let runtime = row::<R>(manager);
    for _ in 0..10_000 {
        if runtime.in_flight_count() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(runtime.in_flight_count(), 0, "every checkout released");
    runtime.store.len().await
}

fn outcome_unknown_events(events: &mut crate::Subscriber<ResourceEvent>) -> usize {
    let mut unknown = 0;
    while let Some(event) = events.try_recv() {
        if matches!(event, ResourceEvent::UnitOutcomeUnknown { .. }) {
            unknown += 1;
        }
    }
    unknown
}

/// A session that adds `n` and commits.
fn add(row: &ManagedRow<StrictPooled>, spec: SessionSpec, n: u64) -> Unit<u64> {
    row.session(spec, move |tx, _cx| {
        Box::pin(async move {
            tx.pending += n;
            Ok(*tx.instance)
        })
    })
}

// ── compile gates ────────────────────────────────────────────────────────

#[test]
fn a_session_unit_crosses_threads() {
    fn send<T: Send>(_: &T) {}
    fn gates(row: &ManagedRow<StrictPooled>) {
        send(&add(row, write(), 1));
    }
    let _ = gates;
}

// ── settled outcomes ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_committed_session_is_sent_and_its_instance_recycled() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);

    let seen = add(&row, write(), 5).await.expect("committed");
    assert_eq!(seen, 0, "the first instance");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);
    let seen = add(&row, write(), 1).await.expect("committed again");
    assert_eq!(seen, 5, "the same instance, with the first commit applied");
    assert_eq!(resource.probe.creates(), 1);
    assert_eq!(resource.probe.opens(), 2);
}

#[tokio::test]
async fn a_failed_body_rolls_back_unsent_and_recycles() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);

    let error = row
        .session(write(), |tx, _cx| {
            Box::pin(async move {
                tx.pending = 9;
                Err::<(), _>(OpError::new(ErrorKind::Transient, "body failed"))
            })
        })
        .await
        .expect_err("rolled back");
    assert_eq!(error.detail(), "body failed");
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(error.is_retryable(), "nothing was applied");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);
    let seen = add(&row, write(), 0).await.expect("reused");
    assert_eq!(seen, 0, "nothing applied");
    assert_eq!(resource.probe.creates(), 1);
}

#[tokio::test]
async fn a_refused_commit_is_unsent_and_recycles() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    resource.probe.close_next_with(SessionClosed::RolledBack {
        refused: Some(OpError::new(ErrorKind::Permanent, "constraint violated")),
    });

    let error = add(&row, write(), 1).await.expect_err("refused");
    assert_eq!(error.detail(), "constraint violated");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);
    assert_eq!(resource.probe.creates(), 1);
}

#[tokio::test]
async fn an_unknown_close_is_maybe_sent_and_destroys_the_instance() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    let mut events = manager.subscribe_events();
    resource
        .probe
        .close_next_with(SessionClosed::Unknown(OpError::new(
            ErrorKind::Transient,
            "connection lost during commit",
        )));

    let error = add(&row, write(), 1).await.expect_err("unknown");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(!error.is_retryable());
    assert_eq!(*Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(outcome_unknown_events(&mut events), 1);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 0);

    add(&row, write(), 0).await.expect("a fresh instance");
    assert_eq!(resource.probe.creates(), 2, "the tainted one was destroyed");
}

#[tokio::test(start_paused = true)]
async fn a_deadline_mid_body_is_maybe_sent_and_destroys_the_instance() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    let hang = |spec: SessionSpec| {
        row.session(spec, |_tx, _cx| {
            Box::pin(async {
                std::future::pending::<()>().await;
                Ok(())
            })
        })
        .with_deadline(Instant::now().into_std() + Duration::from_secs(1))
    };

    let error = hang(read()).await.expect_err("deadline");
    assert_eq!(*error.kind(), ErrorKind::Transient);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(error.is_retryable(), "a read session may be replayed");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 0);

    let error = hang(write()).await.expect_err("deadline");
    assert_eq!(*Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 0);
    assert_eq!(
        resource.probe.creates(),
        2,
        "each cut-off session destroyed its instance"
    );
}

#[tokio::test]
async fn a_panicking_body_is_maybe_sent_and_destroys_the_instance() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);

    let error = row
        .session(write(), |tx, _cx| {
            Box::pin(async move {
                assert_eq!(tx.pending, 1, "a body bug panics mid-session");
                Ok(())
            })
        })
        .await
        .expect_err("panicked");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 0);
    add(&row, write(), 0).await.expect("the row survives");
    assert_eq!(resource.probe.creates(), 2);
}

#[tokio::test]
async fn a_failed_open_is_unsent_destroys_the_instance_and_skips_the_body() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    resource.probe.fail_next_open();
    let called = Arc::new(AtomicBool::new(false));

    let error = row
        .session(write(), {
            let called = Arc::clone(&called);
            move |_tx, _cx| {
                called.store(true, Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            }
        })
        .await
        .expect_err("open refused");
    assert_eq!(error.detail(), "open refused");
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(!called.load(Ordering::SeqCst), "the body never ran");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 0);
    add(&row, write(), 0).await.expect("fresh");
    assert_eq!(resource.probe.creates(), 2);
}

// ── admission ────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_session_books_its_quota_before_it_checks_out() {
    let manager = Manager::new();
    let resource = register(
        &manager,
        StrictPooled::new(),
        Some(RowLimit::rate(
            Rate::per_second(NonZeroU32::MIN)
                .with_burst(NonZeroU32::MIN)
                .expect("valid rate"),
        )),
    );
    bind(&resource.db, credential_id(), 1, 1);
    let row = facade::<StrictPooled>(&manager);
    add(&row, write(), 0).await.expect("the only permit");

    let started = Instant::now();
    let waiting = tokio::spawn(add(&row, write(), 0));
    tokio::time::sleep(Duration::from_millis(999)).await;
    assert_eq!(
        crate::manager::strict_fixtures::row::<StrictPooled>(&manager).in_flight_count(),
        0,
        "no connection held while waiting for quota"
    );
    assert_eq!(resource.probe.opens(), 1);
    waiting.await.expect("joined").expect("granted");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test]
async fn a_connection_bound_session_never_runs_on_an_instance_built_on_older_credentials() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    add(&row, write(), 0).await.expect("builds at material 1");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);

    bind(&resource.db, credential_id(), 2, 1);
    add(&row, write(), 0).await.expect("runs at material 2");
    assert_eq!(resource.probe.creates(), 2, "the idle instance was evicted");
    let opened = resource.probe.opened();
    assert_eq!(opened[1], (1, vec![("db", Some(2))]), "a fresh instance");
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);
}

#[tokio::test]
async fn a_session_bound_session_reuses_an_instance_built_on_older_credentials() {
    let manager = Manager::new();
    let resource = register(&manager, StrictPooledSession::new(), None);
    bind(&resource.db, credential_id(), 1, 1);
    let row = facade::<StrictPooledSession>(&manager);
    let noop = |row: &ManagedRow<StrictPooledSession>| {
        row.session(write(), |_tx, _cx| Box::pin(async { Ok(()) }))
    };
    noop(&row).await.expect("builds at material 1");
    assert_eq!(idle_after_release::<StrictPooledSession>(&manager).await, 1);

    bind(&resource.db, credential_id(), 2, 1);
    noop(&row).await.expect("reuses the instance");
    assert_eq!(resource.probe.creates(), 1);
    assert_eq!(
        resource.probe.opened()[1],
        (0, vec![("db", Some(2))]),
        "the session authenticates with the new pin itself"
    );
}

fn suspend(manager: &Manager) {
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
}

#[tokio::test]
async fn closing_is_cooperative_mid_session() {
    let manager = Manager::new();
    pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    let entered = Arc::new(Notify::new());

    // A body that listens stops early and rolls back.
    let unit = tokio::spawn(row.session(write(), {
        let entered = Arc::clone(&entered);
        move |tx, cx| {
            Box::pin(async move {
                tx.pending = 3;
                entered.notify_one();
                cx.closing().closed().await;
                Err::<(), _>(OpError::new(ErrorKind::Cancelled, "row closing; stopped"))
            })
        }
    }));
    entered.notified().await;
    suspend(&manager);
    let error = unit.await.expect("joined").expect_err("stopped");
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);

    // A body that ignores it completes and commits.
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    let release = Arc::new(Notify::new());
    let unit = tokio::spawn(row.session(write(), {
        let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
        move |tx, _cx| {
            Box::pin(async move {
                tx.pending = 4;
                entered.notify_one();
                release.notified().await;
                Ok(())
            })
        }
    }));
    entered.notified().await;
    suspend(&manager);
    release.notify_one();
    unit.await
        .expect("joined")
        .expect("a granted session is never aborted");
    assert_eq!(resource.probe.opens(), 1);
}

#[tokio::test]
async fn a_strict_session_reads_once_for_an_idle_hit_twice_for_a_create_and_never_opens_blocked() {
    let observer = ScriptedObserver::answering(available(1, 1));
    let manager = strict_manager(erased(&observer), &Arc::default());
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);

    add(&row, write(), 0).await.expect("create");
    assert_eq!(observer.calls(), 2);
    assert_eq!(idle_after_release::<StrictPooled>(&manager).await, 1);
    add(&row, write(), 0).await.expect("idle hit");
    assert_eq!(observer.calls(), 3);

    observer.answer(reauth(1, 2));
    let error = add(&row, write(), 0).await.expect_err("blocked");
    assert_eq!(
        *error.kind(),
        ErrorKind::CredentialUnavailable {
            reason: CredentialUnavailableReason::ReauthRequired
        }
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(resource.probe.opens(), 2, "no session opened");
}

#[tokio::test]
async fn a_cancel_before_the_grant_opens_nothing_and_after_it_is_ignored() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = facade::<StrictPooled>(&manager);

    let unit = add(&row, write(), 1);
    unit.cancel();
    let error = unit.await.expect_err("cancelled");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(resource.probe.opens(), 0);

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut unit = row.session(write(), {
        let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
        move |_tx, _cx| {
            Box::pin(async move {
                entered.notify_one();
                release.notified().await;
                Ok(())
            })
        }
    });
    assert!(futures::poll!(&mut unit).is_pending());
    entered.notified().await;
    unit.cancel();
    release.notify_one();
    unit.await.expect("granted sessions settle");
}

#[tokio::test]
async fn a_nested_session_on_the_same_row_is_refused_permanently() {
    let manager = Manager::new();
    pooled(&manager);
    let row = facade::<StrictPooled>(&manager);
    let inner_row = row.clone();

    let inner = row
        .session(write(), move |_tx, _cx| {
            Box::pin(async move {
                let inner = add(&inner_row, write(), 1).await;
                Ok(inner.expect_err("nested on the same row"))
            })
        })
        .await
        .expect("the outer session commits");
    assert_eq!(*inner.kind(), ErrorKind::Permanent);
    assert_eq!(inner.sent(), SentState::NotSent);
    assert!(!inner.is_retryable());
}
