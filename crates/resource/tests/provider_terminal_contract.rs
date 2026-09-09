//! Provider author obligations at the framework-owned terminal boundary.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use nebula_core::ResourceKey;
use nebula_resource::{
    AcquireOptions, Error, ErrorKind, Manager, Provider, Resident, ResidentConfig,
    ResidentProvider, ResourceContext, ShutdownConfig, ShutdownError, TeardownCx, TeardownReason,
    resource_key,
};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

const TEARDOWN_BUDGET: Duration = Duration::from_secs(1);

struct OwnedInstance(Arc<AtomicUsize>);

impl Drop for OwnedInstance {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum TerminalOutcome {
    Complete,
    Fail,
    TransientFail,
    Hang,
}

struct TerminalProvider {
    drops: Arc<AtomicUsize>,
    invocations: Arc<AtomicUsize>,
    entered: mpsc::UnboundedSender<(TeardownCx, bool)>,
    continue_cleanup: Arc<Notify>,
    manager_cancel: CancellationToken,
    outcome: TerminalOutcome,
}

nebula_resource::no_credential_slots!(TerminalProvider);
impl ResidentProvider for TerminalProvider {}

#[async_trait::async_trait]
impl Provider for TerminalProvider {
    type Config = ();
    type Instance = OwnedInstance;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("terminal-contract")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<OwnedInstance, Error> {
        Ok(OwnedInstance(Arc::clone(&self.drops)))
    }

    fn teardown_budget(&self) -> Duration {
        TEARDOWN_BUDGET
    }

    async fn destroy(&self, instance: OwnedInstance, mut cx: TeardownCx) -> Result<(), Error> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.entered
            .send((cx, self.manager_cancel.is_cancelled()))
            .unwrap();
        if matches!(self.outcome, TerminalOutcome::Hang) {
            // Public field mutation cannot extend the framework's captured deadline.
            cx.deadline += Duration::from_hours(1);
            tokio::time::sleep_until(cx.deadline.into()).await;
        } else {
            self.continue_cleanup.notified().await;
        }
        drop(instance);
        match self.outcome {
            TerminalOutcome::Complete => Ok(()),
            TerminalOutcome::Fail => Err(Error::permanent("terminal cleanup failed")),
            TerminalOutcome::TransientFail => Err(Error::transient("terminal cleanup failed")),
            TerminalOutcome::Hang => panic!("framework must abandon the hanging hook"),
        }
    }
}

async fn cancelled_shutdown_preserves_outcome(outcome: TerminalOutcome) {
    let manager = Arc::new(Manager::new());
    let drops = Arc::new(AtomicUsize::new(0));
    let invocations = Arc::new(AtomicUsize::new(0));
    let continue_cleanup = Arc::new(Notify::new());
    let (entered, mut observations) = mpsc::unbounded_channel();
    common::register_resident(
        &manager,
        TerminalProvider {
            drops: Arc::clone(&drops),
            invocations: Arc::clone(&invocations),
            entered,
            continue_cleanup: Arc::clone(&continue_cleanup),
            manager_cancel: manager.cancel_token().clone(),
            outcome,
        },
        (),
        Resident::new(ResidentConfig::default()),
    );
    manager
        .acquire_resident::<TerminalProvider>(&common::test_ctx(), &AcquireOptions::default())
        .await
        .unwrap()
        .release()
        .await
        .unwrap();

    let before_shutdown = Instant::now();
    let caller = tokio::spawn({
        let manager = Arc::clone(&manager);
        async move { manager.graceful_shutdown(ShutdownConfig::default()).await }
    });
    let (cx, manager_was_cancelled) = observations.recv().await.unwrap();
    assert_eq!(cx.reason, TeardownReason::Shutdown);
    assert!(
        manager_was_cancelled,
        "cleanup must run after manager cancellation"
    );
    assert!(cx.deadline >= before_shutdown + TEARDOWN_BUDGET);
    assert!(cx.deadline <= Instant::now() + TEARDOWN_BUDGET);
    assert_eq!(drops.load(Ordering::SeqCst), 0);

    // Entry notification proves the caller is cancelled while the owned hook runs.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    continue_cleanup.notify_one();
    let result = manager.graceful_shutdown(ShutdownConfig::default()).await;
    match outcome {
        TerminalOutcome::Complete => {
            let report = result.unwrap();
            assert_eq!(report.dropped_release_tasks, 0);
            assert_eq!(report.outstanding_handles_after_drain, 0);
        },
        TerminalOutcome::Fail | TerminalOutcome::TransientFail | TerminalOutcome::Hang => {
            let ShutdownError::ResourceTeardownFailed { key, source } = result.unwrap_err() else {
                panic!("shutdown must preserve the typed terminal error");
            };
            assert_eq!(key, TerminalProvider::key());
            let expected = match outcome {
                TerminalOutcome::Fail => ErrorKind::Permanent,
                TerminalOutcome::TransientFail => ErrorKind::Transient,
                TerminalOutcome::Hang => ErrorKind::Cancelled,
                TerminalOutcome::Complete => unreachable!(),
            };
            assert_eq!(*source.kind(), expected);
            assert_eq!(
                source.is_retryable(),
                matches!(outcome, TerminalOutcome::TransientFail)
            );
        },
    }
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn terminal_completion_survives_shutdown_caller_cancellation() {
    cancelled_shutdown_preserves_outcome(TerminalOutcome::Complete).await;
}

#[tokio::test(start_paused = true)]
async fn terminal_error_survives_shutdown_caller_cancellation_without_retry() {
    cancelled_shutdown_preserves_outcome(TerminalOutcome::Fail).await;
}

#[tokio::test(start_paused = true)]
async fn retryable_terminal_error_is_reported_without_retry() {
    cancelled_shutdown_preserves_outcome(TerminalOutcome::TransientFail).await;
}

#[tokio::test(start_paused = true)]
async fn abandoned_terminal_hook_runs_drop_fallback_without_retry() {
    cancelled_shutdown_preserves_outcome(TerminalOutcome::Hang).await;
}

struct DefaultDropProvider(Arc<AtomicUsize>);

nebula_resource::no_credential_slots!(DefaultDropProvider);
impl ResidentProvider for DefaultDropProvider {}

#[async_trait::async_trait]
impl Provider for DefaultDropProvider {
    type Config = ();
    type Instance = OwnedInstance;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("default-drop")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<OwnedInstance, Error> {
        Ok(OwnedInstance(Arc::clone(&self.0)))
    }
}

#[tokio::test(start_paused = true)]
async fn default_destroy_drops_instance_only_after_final_resident_owner() {
    let manager = Manager::new();
    let drops = Arc::new(AtomicUsize::new(0));
    common::register_resident(
        &manager,
        DefaultDropProvider(Arc::clone(&drops)),
        (),
        Resident::new(ResidentConfig::default()),
    );
    let first = manager
        .acquire_resident::<DefaultDropProvider>(&common::test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let second = manager
        .acquire_resident::<DefaultDropProvider>(&common::test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    first.release().await.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    second.release().await.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
