//! A logger resource driven through the managed call facade: a resident
//! sink whose instance owns a bounded buffer, a worker task and an in-memory
//! sink, written to by `Write` and `Flush` operations.
//!
//! It proves the facade on a resource with no credentials and no rate
//! limit: no credential is ever read, "enqueued" and "flushed" are distinct
//! outcomes, a closed lease refuses new attempts without sending, a cancel
//! before the first attempt sends nothing, a dropped waiter does not abort a
//! granted unit and the lease is released only after it ends, and shutdown
//! flushes the buffer within the teardown deadline. It deliberately uses no
//! `tracing-appender`: the sink is the resource's own instance.

use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{
    CoreError, CredentialKey, ResourceKey,
    accessor::{CredentialAccessor, ResourceAccessor},
    context::BaseContext,
    resource_key,
    scope::{Principal, Scope},
};
use nebula_resource::{
    AcquireOptions, Error, ErrorKind, Manager, ManagerConfig, RateLimitProfile, RegistrationSpec,
    Resident, ResidentConfig, ResourceConfig, ResourceContext, ResourceEvent, ScopeLevel,
    ShutdownConfig, SlotIdentity, TeardownCx,
    call::{Cost, Effect, Managed, OpCx, OpError, Operation, SentState},
    resource::{Provider, ResourceMetadataDraft},
    topology::ResidentProvider,
};
use tokio::sync::{Semaphore, mpsc, watch};

// ── the resource ─────────────────────────────────────────────────────────

#[derive(Clone, nebula_schema::Schema)]
struct LoggerConfig {
    buffer: u64,
}

impl ResourceConfig for LoggerConfig {
    fn fingerprint(&self) -> u64 {
        self.buffer
    }
}

/// Where written lines end up, shared with the test.
type Lines = Arc<Mutex<Vec<String>>>;

/// The logger provider. `worker_gate` holds the worker before each line so a
/// test can tell "enqueued" from "flushed"; `lines` is the in-memory sink.
#[derive(Clone)]
struct Logger {
    worker_gate: Arc<Semaphore>,
    lines: Lines,
}

impl Logger {
    fn new() -> Self {
        Self {
            worker_gate: Arc::new(Semaphore::new(0)),
            lines: Arc::default(),
        }
    }

    fn open_gate(&self) {
        self.worker_gate.add_permits(Semaphore::MAX_PERMITS / 2);
    }

    fn written(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Aborts the worker when the sink goes away without a graceful close.
struct Worker(Option<tokio::task::JoinHandle<()>>);

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            worker.abort();
        }
    }
}

/// The logger's instance: a bounded buffer drained by a worker into the
/// sink, and the flushed watermark the worker advances.
struct LogSink {
    buffer: Mutex<Option<mpsc::Sender<String>>>,
    worker: Mutex<Worker>,
    enqueued: Mutex<u64>,
    flushed: watch::Receiver<u64>,
}

/// Why a line was not enqueued.
enum Rejected {
    Full,
    Closed,
}

impl LogSink {
    fn start(capacity: usize, gate: Arc<Semaphore>, lines: Lines) -> Self {
        let (buffer, mut pending) = mpsc::channel::<String>(capacity);
        let (watermark, flushed) = watch::channel(0_u64);
        let worker = tokio::spawn(async move {
            while let Some(line) = pending.recv().await {
                let Ok(permit) = gate.acquire().await else {
                    return;
                };
                permit.forget();
                lines
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line);
                watermark.send_modify(|through| *through += 1);
            }
        });
        Self {
            buffer: Mutex::new(Some(buffer)),
            worker: Mutex::new(Worker(Some(worker))),
            enqueued: Mutex::new(0),
            flushed,
        }
    }

    /// Enqueues without waiting: a full buffer is the caller's backpressure.
    fn enqueue(&self, line: String) -> Result<u64, Rejected> {
        let buffer = self.buffer.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = buffer.as_ref() else {
            return Err(Rejected::Closed);
        };
        let mut enqueued = self.enqueued.lock().unwrap_or_else(PoisonError::into_inner);
        sender.try_send(line).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Rejected::Full,
            mpsc::error::TrySendError::Closed(_) => Rejected::Closed,
        })?;
        *enqueued += 1;
        Ok(*enqueued)
    }

    fn enqueued(&self) -> u64 {
        *self.enqueued.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits until the worker wrote every line up to `through`.
    async fn flushed_through(&self, through: u64) -> Result<u64, ()> {
        let mut flushed = self.flushed.clone();
        flushed
            .wait_for(|written| *written >= through)
            .await
            .map(|written| *written)
            .map_err(|_closed| ())
    }

    /// Closes the buffer and joins the worker, which drains what is left.
    async fn close(&self) -> Option<tokio::task::JoinHandle<()>> {
        drop(
            self.buffer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        self.worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .take()
    }
}

#[async_trait::async_trait]
impl Provider for Logger {
    type Config = LoggerConfig;
    type Instance = LogSink;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("fixture.logger")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("Logger"),
            "An in-memory log sink",
        )
    }

    fn teardown_budget(&self) -> Duration {
        Duration::from_secs(2)
    }

    async fn create(&self, config: &LoggerConfig, _: &ResourceContext) -> Result<LogSink, Error> {
        let capacity = usize::try_from(config.buffer)
            .map_err(|_| Error::permanent("logger buffer does not fit in memory"))?;
        Ok(LogSink::start(
            capacity,
            Arc::clone(&self.worker_gate),
            Arc::clone(&self.lines),
        ))
    }

    async fn destroy(&self, sink: LogSink, cx: TeardownCx) -> Result<(), Error> {
        let Some(worker) = sink.close().await else {
            return Ok(());
        };
        let deadline = tokio::time::Instant::from_std(cx.deadline);
        match tokio::time::timeout_at(deadline, worker).await {
            Ok(_) => Ok(()),
            Err(_elapsed) => Err(Error::transient(
                "log worker did not drain before the teardown deadline",
            )),
        }
    }
}

nebula_resource::no_credential_slots!(Logger);

impl ResidentProvider for Logger {}

// ── the operations ───────────────────────────────────────────────────────

/// A line accepted into the buffer; not yet written.
#[derive(Debug, PartialEq, Eq)]
struct Enqueued {
    seq: u64,
}

/// Every line enqueued before the flush started is written.
#[derive(Debug, PartialEq, Eq)]
struct Flushed {
    through: u64,
}

struct Write {
    line: String,
}

impl Operation<Logger> for Write {
    type Output = Enqueued;
    const EFFECT: Effect = Effect::Write;

    async fn run(self, cx: &mut OpCx<'_, Logger>) -> Result<Enqueued, OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        match attempt.instance().enqueue(self.line) {
            Ok(seq) => {
                attempt.settle(SentState::Sent);
                Ok(Enqueued { seq })
            },
            Err(Rejected::Full) => {
                attempt.settle(SentState::NotSent);
                Err(OpError::new(ErrorKind::Backpressure, "log buffer full"))
            },
            Err(Rejected::Closed) => {
                attempt.settle(SentState::NotSent);
                Err(OpError::new(ErrorKind::Cancelled, "log sink closed"))
            },
        }
    }
}

struct Flush;

impl Operation<Logger> for Flush {
    type Output = Flushed;
    const EFFECT: Effect = Effect::Idempotent;

    async fn run(self, cx: &mut OpCx<'_, Logger>) -> Result<Flushed, OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        let through = attempt.instance().enqueued();
        let flushed = attempt.instance().flushed_through(through).await;
        attempt.settle(SentState::Sent);
        flushed
            .map(|through| Flushed { through })
            .map_err(|()| OpError::new(ErrorKind::Cancelled, "log worker stopped"))
    }
}

// ── harness ──────────────────────────────────────────────────────────────

/// Counts every credential lookup; the logger must make none.
#[derive(Default)]
struct CountingCredentials {
    reads: AtomicUsize,
}

type Lookup<'a, T> = Pin<Box<dyn Future<Output = Result<T, CoreError>> + Send + 'a>>;

impl CredentialAccessor for CountingCredentials {
    fn has(&self, _: &CredentialKey) -> bool {
        self.reads.fetch_add(1, Ordering::SeqCst);
        false
    }

    fn resolve_any(&self, key: &CredentialKey) -> Lookup<'_, Box<dyn Any + Send + Sync>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let key = key.clone();
        Box::pin(async move { Err(CoreError::credential_not_found(key)) })
    }

    fn try_resolve_any(&self, _: &CredentialKey) -> Lookup<'_, Option<Box<dyn Any + Send + Sync>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }
}

struct NoResources;

impl ResourceAccessor for NoResources {
    fn has(&self, _: &ResourceKey) -> bool {
        false
    }

    fn acquire_any(&self, key: &ResourceKey) -> Lookup<'_, Box<dyn Any + Send + Sync>> {
        let key = key.as_str().to_owned();
        Box::pin(async move { Err(CoreError::resource_unavailable(key, "none", false, None)) })
    }

    fn try_acquire_any(&self, _: &ResourceKey) -> Lookup<'_, Option<Box<dyn Any + Send + Sync>>> {
        Box::pin(async { Ok(None) })
    }
}

struct Fixture {
    manager: Manager,
    logger: Logger,
    credentials: Arc<CountingCredentials>,
}

impl Fixture {
    fn new(buffer: u64) -> Self {
        let manager = Manager::with_config(
            ManagerConfig::default()
                .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new())),
        );
        let logger = Logger::new();
        manager
            .register(RegistrationSpec {
                resource: logger.clone(),
                config: LoggerConfig { buffer },
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register the logger");
        Self {
            manager,
            logger,
            credentials: Arc::default(),
        }
    }

    fn context(&self) -> ResourceContext {
        let base = BaseContext::builder(Scope::default()).build_with(Principal::System);
        let credentials: Arc<dyn CredentialAccessor> = Arc::clone(&self.credentials) as _;
        ResourceContext::new(base, Arc::new(NoResources), credentials)
    }

    async fn managed(&self) -> Managed<Logger> {
        self.manager
            .acquire::<Logger>(&self.context(), &AcquireOptions::default())
            .await
            .expect("acquire the logger")
            .into_managed()
    }

    fn units_settled(&self) -> nebula_resource::CallUnitsSnapshot {
        self.manager
            .metrics()
            .expect("metrics configured")
            .snapshot()
            .call_units
    }
}

fn write(line: &str) -> Write {
    Write {
        line: line.to_owned(),
    }
}

/// Lets every task run until the runtime is idle (paused time only).
async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn enqueued_and_flushed_are_distinct_and_no_credential_is_read() {
    let fixture = Fixture::new(8);
    let logger = fixture.managed().await;

    assert_eq!(
        logger.submit(write("one")).await.expect("enqueued"),
        Enqueued { seq: 1 }
    );
    assert_eq!(
        logger.submit(write("two")).await.expect("enqueued"),
        Enqueued { seq: 2 }
    );
    assert!(
        fixture.logger.written().is_empty(),
        "enqueued is not written: the worker is held"
    );

    let mut flush = logger.submit(Flush);
    assert!(futures::poll!(&mut flush).is_pending());
    settle_tasks().await;
    assert!(
        futures::poll!(&mut flush).is_pending(),
        "a flush waits for the worker"
    );
    fixture.logger.open_gate();
    assert_eq!(flush.await.expect("flushed"), Flushed { through: 2 });
    assert_eq!(fixture.logger.written(), ["one", "two"]);

    assert_eq!(
        fixture.credentials.reads.load(Ordering::SeqCst),
        0,
        "the facade and the logger read no credential"
    );
    let health = fixture
        .manager
        .health_check::<Logger>(&ScopeLevel::Global)
        .expect("row");
    assert_eq!(health.rate_limit_profile, RateLimitProfile::PerAttempt);
}

#[tokio::test(start_paused = true)]
async fn a_full_buffer_is_backpressure_and_nothing_is_sent() {
    let fixture = Fixture::new(1);
    let logger = fixture.managed().await;
    logger.submit(write("fits")).await.expect("enqueued");
    settle_tasks().await;
    logger
        .submit(write("fills"))
        .await
        .expect("the worker holds one line, the buffer the next");

    let error = logger
        .submit(write("overflows"))
        .await
        .expect_err("buffer full");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(
        error.sent(),
        SentState::NotSent,
        "the author settled NotSent"
    );
    assert!(error.is_retryable());
    assert_eq!(fixture.units_settled().not_sent, 1);
}

#[tokio::test(start_paused = true)]
async fn a_removed_row_refuses_new_attempts_without_sending() {
    let fixture = Fixture::new(8);
    let logger = fixture.managed().await;
    fixture
        .manager
        .remove(&Logger::key())
        .expect("remove the row");
    assert!(logger.is_closing());

    let error = logger
        .submit(write("late"))
        .await
        .expect_err("a closed lease admits nothing");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    let as_error = Error::from(error);
    assert_eq!(*as_error.kind(), ErrorKind::Cancelled);
    assert!(
        matches!(
            as_error.to_core_error(),
            CoreError::ResourceUnavailable {
                retryable: false,
                ..
            }
        ),
        "a removed row is not retried"
    );
}

#[tokio::test(start_paused = true)]
async fn a_cancel_before_the_first_attempt_sends_nothing() {
    let fixture = Fixture::new(8);
    let logger = fixture.managed().await;
    let unit = logger.submit(write("never"));
    unit.cancel();
    let error = unit.await.expect_err("cancelled");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    fixture.logger.open_gate();
    assert_eq!(
        logger.submit(Flush).await.expect("flushed"),
        Flushed { through: 0 },
        "the cancelled line never reached the buffer"
    );
}

#[tokio::test(start_paused = true)]
async fn a_dropped_waiter_does_not_abort_a_granted_flush_and_the_lease_outlives_it() {
    let fixture = Fixture::new(8);
    let mut events = fixture.manager.subscribe_events();
    let logger = fixture.managed().await;
    logger.submit(write("held")).await.expect("enqueued");

    let mut flush = logger.submit(Flush);
    assert!(futures::poll!(&mut flush).is_pending());
    settle_tasks().await;
    drop(flush);
    drop(logger);
    settle_tasks().await;
    let released_early = std::iter::from_fn(|| events.try_recv())
        .any(|event| matches!(event, ResourceEvent::Released { .. }));
    assert!(!released_early, "the running flush keeps the lease");

    fixture.logger.open_gate();
    settle_tasks().await;
    assert_eq!(fixture.logger.written(), ["held"]);
    assert_eq!(fixture.units_settled().sent, 2, "the flush settled");
    let released = std::iter::from_fn(|| events.try_recv())
        .any(|event| matches!(event, ResourceEvent::Released { .. }));
    assert!(released, "the lease is released once the unit ended");
}

#[tokio::test(start_paused = true)]
async fn shutdown_flushes_the_buffer_within_the_teardown_deadline() {
    let fixture = Fixture::new(8);
    let logger = fixture.managed().await;
    for line in ["a", "b", "c"] {
        logger.submit(write(line)).await.expect("enqueued");
    }
    drop(logger);

    let started = tokio::time::Instant::now();
    let shutdown = fixture.manager.graceful_shutdown(ShutdownConfig::default());
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        finished = &mut shutdown => panic!("shutdown ended before the buffer drained: {finished:?}"),
        () = tokio::time::sleep(Duration::from_millis(10)) => {},
    }
    assert!(
        fixture.logger.written().is_empty(),
        "the worker is still held"
    );

    fixture.logger.open_gate();
    shutdown.await.expect("graceful shutdown");
    assert_eq!(
        fixture.logger.written(),
        ["a", "b", "c"],
        "destroy closed the buffer and joined the worker, which drained it"
    );
    assert!(
        started.elapsed() < Logger::new().teardown_budget(),
        "drained within the teardown deadline"
    );
}
