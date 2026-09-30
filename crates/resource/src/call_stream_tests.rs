//! Streaming units: item order, the error after the items, backpressure,
//! and every way a stream ends early — drop, cancel, deadline, lease
//! closing. Paused time; no transport.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

use super::{StreamOperation, StreamSink};
use crate::{
    AcquireOptions, Error, ErrorKind, Manager, Provider, RegistrationSpec, Resident,
    ResidentConfig, ResourceConfig, ResourceContext, SlotIdentity,
    call::{Cost, Effect, Lease, OperationCx, OperationError, SentState},
    resource::ResourceMetadataDraft,
    runtime::managed::ManagedResource,
    topology::ResidentProvider,
};

// ── fixtures ─────────────────────────────────────────────────────────────

#[derive(Clone, nebula_schema::Schema)]
struct Config {
    version: u64,
}

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

/// A slot-less shared instance: streaming needs nothing else.
#[derive(Clone)]
struct Feed;

#[async_trait::async_trait]
impl Provider for Feed {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("call-stream")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("call-stream"), "")
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for Feed {}

crate::no_credential_slots!(Feed);

async fn feed(manager: &Manager) -> Lease<Feed> {
    manager
        .register(RegistrationSpec {
            resource: Feed,
            config: Config { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    manager
        .acquire::<Feed>(&context, &AcquireOptions::default())
        .await
        .expect("acquire")
        .into_lease()
}

fn leases(manager: &Manager) -> u64 {
    manager
        .lookup_any_for_slot_identity_structural(
            &Feed::key(),
            &ScopeLevel::Global,
            &SlotIdentity::Unbound,
        )
        .expect("row is registered")
        .as_any_arc()
        .downcast::<ManagedResource<Feed>>()
        .expect("row type")
        .in_flight
        .0
        .load(Ordering::SeqCst)
}

fn capacity(items: usize) -> NonZeroUsize {
    NonZeroUsize::new(items).expect("non-zero")
}

/// Lets every task run until the runtime is idle (paused time only).
async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

/// What a test sees of the operation's side.
#[derive(Clone, Default)]
struct Probe {
    started: Arc<AtomicUsize>,
    sent: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
    granted: Arc<Notify>,
}

/// After one granted `Sent` attempt: sends `items` values, then waits as
/// `wait` says, and ends with `end`.
struct Emit {
    items: u64,
    wait: Wait,
    end: Result<u64, ErrorKind>,
    probe: Probe,
}

enum Wait {
    /// Returns right after the items.
    Nothing,
    /// Waits for the consumer to go away, as while reading a provider.
    ConsumerGone,
    /// Waits for the lease to close.
    LeaseClosing,
    /// Never returns: the unit hits its deadline.
    Forever,
}

impl StreamOperation<Feed> for Emit {
    type Item = u64;
    type Output = u64;
    const KEY: &'static str = "test.emit";
    const EFFECT: Effect = Effect::Write;

    async fn run(
        self,
        cx: &mut OperationCx<'_, Feed>,
        mut sink: StreamSink<u64>,
    ) -> Result<u64, OperationError> {
        self.probe.started.fetch_add(1, Ordering::SeqCst);
        let closing = cx.closing();
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt.settle(SentState::Sent);
        self.probe.granted.notify_one();
        let ended = EndGuard(Arc::clone(&self.probe.ended));
        for value in 0..self.items {
            sink.send(value).await?;
            self.probe.sent.fetch_add(1, Ordering::SeqCst);
        }
        match self.wait {
            Wait::Nothing => {},
            Wait::ConsumerGone => {
                sink.closed().await;
                return Err(OperationError::new(ErrorKind::Cancelled, "consumer gone"));
            },
            Wait::LeaseClosing => {
                closing.closed().await;
                return Err(OperationError::new(ErrorKind::Cancelled, "lease closing"));
            },
            Wait::Forever => std::future::pending::<()>().await,
        }
        drop(ended);
        self.end
            .map_err(|kind| OperationError::new(kind, "provider failed mid-stream"))
    }
}

/// Counts the operation's end however it ends, including a drop.
struct EndGuard(Arc<AtomicUsize>);

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn emit(items: u64, wait: Wait, end: Result<u64, ErrorKind>) -> (Emit, Probe) {
    let probe = Probe::default();
    (
        Emit {
            items,
            wait,
            end,
            probe: probe.clone(),
        },
        probe,
    )
}

// ── delivery ─────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn items_arrive_in_order_then_the_stream_ends_with_the_output() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, _) = emit(5, Wait::Nothing, Ok(42));
    let mut stream = managed.submit_streaming(operation, capacity(2));

    let mut seen = Vec::new();
    while let Some(item) = stream.next().await {
        seen.push(item.expect("no error"));
    }
    assert_eq!(seen, vec![0, 1, 2, 3, 4]);
    assert!(stream.next().await.is_none(), "an ended stream stays ended");
    assert_eq!(stream.finish().await.expect("output"), 42);
}

#[tokio::test(start_paused = true)]
async fn buffered_items_come_before_the_units_error_which_comes_once() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(3, Wait::Nothing, Err(ErrorKind::Transient));
    let mut stream = managed.submit_streaming(operation, capacity(8));

    assert_eq!(stream.next().await.map(Result::ok), Some(Some(0)));
    settle_tasks().await;
    assert_eq!(probe.ended.load(Ordering::SeqCst), 1, "the unit settled");
    assert_eq!(stream.next().await.map(Result::ok), Some(Some(1)));
    assert_eq!(stream.next().await.map(Result::ok), Some(Some(2)));
    let error = stream
        .next()
        .await
        .expect("the error after the items")
        .expect_err("the unit failed");
    assert_eq!(*error.kind(), ErrorKind::Transient);
    assert_eq!(
        error.sent(),
        SentState::Sent,
        "stamped with the settled state"
    );
    assert!(!error.is_retryable(), "a sent write is not retried");
    assert!(stream.next().await.is_none(), "the error comes once");
    let finished = stream.finish().await.expect_err("finish repeats it");
    assert_eq!(*finished.kind(), ErrorKind::Transient);
}

#[tokio::test(start_paused = true)]
async fn a_slow_consumer_holds_the_operation_at_the_buffer() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(10, Wait::Nothing, Ok(0));
    let mut stream = managed.submit_streaming(operation, capacity(2));

    assert_eq!(stream.next().await.map(Result::ok), Some(Some(0)));
    settle_tasks().await;
    // One item read, two buffered, the fourth send waiting.
    assert_eq!(probe.sent.load(Ordering::SeqCst), 3);
    assert_eq!(stream.finish().await.expect("drained to the end"), 0);
    assert_eq!(probe.sent.load(Ordering::SeqCst), 10);
}

// ── ending early ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_stream_dropped_before_its_first_poll_never_runs() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(1, Wait::Nothing, Ok(0));
    drop(managed.submit_streaming(operation, capacity(1)));
    settle_tasks().await;
    assert_eq!(probe.started.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_before_the_grant_settles_cancelled_not_sent() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(1, Wait::Nothing, Ok(0));
    let mut stream = managed.submit_streaming(operation, capacity(1));
    stream.cancel();

    let error = stream
        .next()
        .await
        .expect("the unit's error")
        .expect_err("cancelled");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(stream.next().await.is_none());
    assert_eq!(probe.started.load(Ordering::SeqCst), 0, "never ran");
}

#[tokio::test(start_paused = true)]
async fn a_cancel_after_the_grant_closes_the_sink() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(1, Wait::ConsumerGone, Ok(0));
    let mut stream = managed.submit_streaming(operation, capacity(4));

    assert_eq!(stream.next().await.map(Result::ok), Some(Some(0)));
    stream.cancel();
    let error = stream.finish().await.expect_err("the operation stopped");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::Sent);
    assert_eq!(probe.ended.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_consumer_ends_the_operation_and_releases_the_lease() {
    // More items than the buffer: the operation ends at a failed send.
    // One item: it ends while it waits on the provider.
    for (items, wait) in [(4, Wait::Nothing), (1, Wait::ConsumerGone)] {
        let manager = Manager::new();
        let managed = feed(&manager).await;
        let (operation, probe) = emit(items, wait, Ok(0));
        let mut stream = managed.submit_streaming(operation, capacity(1));
        assert_eq!(stream.next().await.map(Result::ok), Some(Some(0)));
        drop(managed);
        assert_eq!(leases(&manager), 1, "the running unit holds the lease");

        drop(stream);
        settle_tasks().await;
        assert_eq!(probe.ended.load(Ordering::SeqCst), 1, "the operation ended");
        assert_eq!(leases(&manager), 0, "released once the unit ended");
    }
}

#[tokio::test(start_paused = true)]
async fn a_deadline_mid_stream_is_maybe_sent() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(1, Wait::Forever, Ok(0));
    let mut stream = managed
        .submit_streaming(operation, capacity(1))
        .with_deadline((Instant::now() + Duration::from_secs(1)).into_std());

    assert_eq!(stream.next().await.map(Result::ok), Some(Some(0)));
    let error = stream
        .next()
        .await
        .expect("the unit's error")
        .expect_err("deadline");
    assert_eq!(*error.kind(), ErrorKind::Transient);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(!error.is_retryable(), "a write with an unknown outcome");
    assert_eq!(
        probe.ended.load(Ordering::SeqCst),
        1,
        "the operation was dropped"
    );
}

#[tokio::test(start_paused = true)]
async fn a_closing_lease_ends_a_stream_that_selects_on_it() {
    let manager = Manager::new();
    let managed = feed(&manager).await;
    let (operation, probe) = emit(0, Wait::LeaseClosing, Ok(0));
    let stream = managed.submit_streaming(operation, capacity(1));
    let running = tokio::spawn(stream.finish());
    probe.granted.notified().await;

    manager.remove(&Feed::key()).expect("remove");
    let error = running.await.expect("task").expect_err("closing");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::Sent);
}

#[test]
fn the_stream_handles_cross_threads() {
    fn send<T: Send + Unpin>() {}
    send::<super::Streaming<u64, u64>>();
    send::<StreamSink<u64>>();
}
