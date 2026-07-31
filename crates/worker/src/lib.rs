#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # nebula-worker — Generic worker runtime (ADR-0095 D1)
//!
//! A worker is a long-running process that:
//!
//! 1. Boots a flavor's plugins and derives the set of [`PluginKey`]s it can serve.
//! 2. Advertises those keys as `available_plugins` to the pull-loop.
//! 3. Runs a **leaderless claim-loop** via [`nebula_orchestrator::Orchestrator`]:
//!    claims [`JobDispatchQueue`] rows whose `required_plugins ⊆ available_plugins`,
//!    hands them to [`EngineExecutionSink`], and fences each row dispatched or failed.
//! 4. Drives execution into the engine via `resume_execution` (the sink's job).
//!
//! ## Wiring honesty
//!
//! This crate provides the **generic runtime** only. A per-flavor binary that
//! boots concrete plugins and derives `available_plugins` from them is a later
//! unit (U-D1.4+). Today, callers pass the `Vec<PluginKey>` they have already
//! derived from their plugin registry.
//!
//! ## Construction
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use nebula_core::PluginKey;
//! use nebula_engine::{ExecutionStores, WorkflowEngine};
//! use nebula_storage_port::store::JobDispatchQueue;
//! use nebula_worker::WorkerRuntimeBuilder;
//! use tokio_util::sync::CancellationToken;
//!
//! # fn wire(
//! #     engine: Arc<WorkflowEngine>,
//! #     stores: ExecutionStores,
//! #     queue: Arc<dyn JobDispatchQueue>,
//! #     plugins: Vec<PluginKey>,
//! #     proc_id: [u8; 16],
//! #     shutdown_token: CancellationToken,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let runtime = WorkerRuntimeBuilder::from_wired_engine(engine, stores, queue, plugins, proc_id)
//!     .with_batch_size(16)
//!     .build()?;
//!
//! runtime.spawn(shutdown_token);
//! # Ok(())
//! # }
//! ```
//!
//! [`PluginKey`]: nebula_core::PluginKey
//! [`JobDispatchQueue`]: nebula_storage_port::store::JobDispatchQueue
//! [`EngineExecutionSink`]: nebula_engine::EngineExecutionSink

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nebula_core::PluginKey;
use nebula_engine::{
    ControlConsumer, DEFAULT_TIMER_SCAN_INTERVAL, EngineControlDispatch, EngineExecutionSink,
    ExecutionStores, WorkflowEngine,
};
use nebula_metrics::MetricsRegistry;
use nebula_orchestrator::Orchestrator;
use nebula_storage_port::store::{ControlQueue, JobDispatchQueue};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

/// Errors that can be produced when building a [`WorkerRuntime`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerBuildError {
    /// `available_plugins` is empty — the worker would never claim any job.
    ///
    /// A worker with no advertised plugins is a configuration error: the superset
    /// predicate `required_plugins ⊆ available_plugins` is vacuously unsatisfiable
    /// for any non-empty `required_plugins`, and the storage backends short-circuit
    /// on an empty available set rather than scanning the queue.
    #[error("available_plugins is empty — a worker must advertise at least one PluginKey")]
    NoPlugins,

    /// No control queue was wired.
    ///
    /// Without one the worker drains job-dispatch rows but nothing drains the
    /// control queue, so an execution accepted over HTTP is persisted with a
    /// `Start` command no component ever consumes: the run never begins, and
    /// the only symptom is an execution that stays `Created` forever. Requiring
    /// the queue makes that miswiring a build error instead of silence.
    #[error("no control queue wired — accepted Start commands would never be consumed")]
    NoControlQueue,

    /// The timer-scan interval is zero.
    ///
    /// `tokio::time::interval` panics on a zero period, so a zero here does not
    /// mean "scan continuously" — it means the scanner task dies the moment it
    /// starts. Rejecting it at build time keeps a plausible-looking
    /// configuration from turning into a panic inside a supervised task.
    #[error("timer scan interval must be greater than zero")]
    ZeroTimerScanInterval,
}

/// Why a supervised worker component stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerRuntimeError {
    /// A supervised component panicked or was cancelled.
    ///
    /// Surfaced rather than swallowed: a worker whose control consumer died is
    /// no longer draining accepted commands, and a silent survivor process is
    /// indistinguishable from a healthy one.
    #[error("worker component `{component}` ended abnormally: {source}")]
    ComponentJoin {
        /// Which component ended.
        component: &'static str,
        /// The join failure.
        #[source]
        source: tokio::task::JoinError,
    },
}

/// What one supervised task reports: which component it was, and — when the
/// failure happened *inside* it — the join failure it carried out.
type ComponentOutcome = Result<Component, (Component, tokio::task::JoinError)>;

/// One supervised top-level worker task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Component {
    Orchestrator,
    ControlConsumer,
    TimerScanner,
}

impl Component {
    const fn label(self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::ControlConsumer => "control-consumer",
            Self::TimerScanner => "timer-scanner",
        }
    }
}

/// An assembled, ready-to-run worker runtime.
///
/// Holds the [`Orchestrator`] configured with an [`EngineExecutionSink`]
/// connected to the provided engine and execution store, plus the engine
/// reference needed to spawn the durable-timer wake scanner.
///
/// Obtain via [`WorkerRuntimeBuilder::build`].
#[must_use = "call .run() or .spawn() to start the pull loop"]
pub struct WorkerRuntime {
    engine: Arc<WorkflowEngine>,
    timer_scan_interval: Duration,
    orchestrator: Orchestrator,
    /// Drains the durable control queue the API writes accepted commands to.
    ///
    /// Held by the runtime rather than by the caller so it lives and dies with
    /// the orchestrator under one cancellation tree.
    control_consumer: ControlConsumer,
    processor_id: [u8; 16],
    available_plugins_count: usize,
}

impl std::fmt::Debug for WorkerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerRuntime")
            .field("processor_id", &hex_id(&self.processor_id))
            .field("available_plugins_count", &self.available_plugins_count)
            .finish_non_exhaustive()
    }
}

impl WorkerRuntime {
    /// Run the claim-loop and durable-timer scanner on the current task until
    /// `shutdown` is cancelled.
    ///
    /// The timer scanner runs as a sibling background task sharing the same
    /// shutdown token so it stops when the worker stops.
    ///
    /// Prefer [`spawn`](Self::spawn) unless integrating into a custom task structure.
    ///
    /// ## Shutdown contract
    ///
    /// Mirrors [`Orchestrator::run`]: flushes the in-flight batch, then returns.
    /// Rows claimed but not yet marked remain `Processing` and are recovered by
    /// the next runner's reclaim sweep.
    pub async fn run(self, shutdown: CancellationToken) -> Result<(), WorkerRuntimeError> {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime starting (ADR-0095 D1)"
        );

        // Recover persisted parked waits *before* joining the steady-state
        // loops.
        //
        // The periodic scanner deliberately skips its first tick, on the
        // reasoning that nothing is overdue the instant a process starts. That
        // is true of a fresh process and false of a restarted one: a wait
        // parked before the restart is overdue precisely *because* the process
        // was down, and waiting a full scan interval to notice leaves the
        // execution stalled for up to that long with every component reporting
        // healthy. Sweeping once at startup makes recovery an ordered step the
        // caller can observe rather than a side effect of the next tick.
        //
        // A sweep failure is logged, not fatal: it re-runs on the next tick,
        // and refusing to start would turn a transient storage blip into an
        // outage.
        match self.engine.sweep_overdue_timers().await {
            Ok(0) => tracing::debug!("startup recovery sweep found no overdue parked waits"),
            Ok(redriven) => tracing::info!(
                redriven,
                "startup recovery sweep re-armed parked waits from persisted state"
            ),
            Err(error) => tracing::error!(
                %error,
                "startup recovery sweep failed; the periodic scanner will retry"
            ),
        }

        // One cancellation tree, and every top-level task joined.
        //
        // The timer scanner used to be spawned and its `JoinHandle` dropped.
        // A detached task that panics takes its failure with it: the worker
        // keeps serving, parked executions silently stop waking, and nothing
        // reports it. Joining all three means a component death is an error the
        // app can act on, and cancelling the token stops the siblings rather
        // than leaving them running against a half-dead runtime.
        // Each task reports which component it is, so a failure is attributed
        // rather than guessed. A task that panics yields only a `JoinError`, so
        // the id map carries the label the payload no longer can.
        let mut components: JoinSet<ComponentOutcome> = JoinSet::new();
        let mut labels: HashMap<tokio::task::Id, Component> = HashMap::new();

        let orchestrator_shutdown = shutdown.clone();
        let orchestrator = self.orchestrator;
        let handle = components.spawn(async move {
            orchestrator.run(orchestrator_shutdown).await;
            Ok(Component::Orchestrator)
        });
        labels.insert(handle.id(), Component::Orchestrator);

        let consumer_shutdown = shutdown.clone();
        let control_consumer = self.control_consumer;
        let handle = components.spawn(async move {
            control_consumer.run(consumer_shutdown).await;
            Ok(Component::ControlConsumer)
        });
        labels.insert(handle.id(), Component::ControlConsumer);

        let scanner_shutdown = shutdown.clone();
        let scanner_engine = Arc::clone(&self.engine);
        let scan_interval = self.timer_scan_interval;
        let handle = components.spawn(async move {
            let scanner = scanner_engine.spawn_timer_scanner(scan_interval, scanner_shutdown);
            // The scanner owns its own task, so its failure has to be carried
            // out deliberately. Discarding the `JoinError` here would report a
            // panicked scanner as a clean stop: the runtime would keep serving
            // with nothing waking parked executions, and the supervision loop
            // would neither record the error nor stop the siblings — the exact
            // silence joining the task was meant to end.
            match scanner.await {
                Ok(()) => Ok(Component::TimerScanner),
                Err(source) => Err((Component::TimerScanner, source)),
            }
        });
        labels.insert(handle.id(), Component::TimerScanner);

        let mut first_failure = None;
        while let Some(joined) = components.join_next().await {
            let failure = match joined {
                Ok(Ok(component)) => {
                    tracing::debug!(component = component.label(), "worker component stopped");
                    None
                },
                // A task this runtime supervises failed inside itself.
                Ok(Err((component, source))) => Some((component, source)),
                // The supervised task itself panicked or was cancelled; the id
                // map says which one.
                Err(source) => {
                    let component = labels
                        .get(&source.id())
                        .copied()
                        .unwrap_or(Component::Orchestrator);
                    Some((component, source))
                },
            };
            if let Some((component, source)) = failure {
                tracing::error!(
                    component = component.label(),
                    error = %source,
                    "worker component ended abnormally; stopping the runtime"
                );
                if first_failure.is_none() {
                    first_failure = Some(WorkerRuntimeError::ComponentJoin {
                        component: component.label(),
                        source,
                    });
                }
                shutdown.cancel();
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    /// Spawn the claim-loop and durable-timer scanner as a single Tokio task.
    ///
    /// Returns a [`JoinHandle`] that completes when `shutdown` is cancelled.
    /// Both the orchestrator and the timer scanner share the same shutdown token
    /// so they stop together. The caller owns signal→[`CancellationToken`] wiring;
    /// this crate provides no `tokio::signal` integration so it composes into any
    /// shutdown strategy.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<Result<(), WorkerRuntimeError>> {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime spawning (ADR-0095 D1)"
        );
        tokio::spawn(async move { self.run(shutdown).await })
    }
}

/// Builder for [`WorkerRuntime`].
///
/// Obtained via [`WorkerRuntimeBuilder::from_wired_engine`]. Optional overrides
/// mirror [`Orchestrator`]'s builder methods.
#[must_use = "call .build() to produce a WorkerRuntime"]
pub struct WorkerRuntimeBuilder {
    engine: Arc<WorkflowEngine>,
    stores: ExecutionStores,
    queue: Arc<dyn JobDispatchQueue>,
    /// Durable control queue the API writes accepted commands to. Required at
    /// `build` time — see [`WorkerBuildError::NoControlQueue`].
    control_queue: Option<Arc<dyn ControlQueue>>,
    available_plugins: Vec<PluginKey>,
    processor_id: [u8; 16],
    // Optional orchestrator overrides — all None means "use Orchestrator defaults".
    batch_size: Option<u32>,
    poll_interval: Option<Duration>,
    reclaim_after: Option<Duration>,
    reclaim_interval: Option<Duration>,
    max_reclaim_count: Option<u32>,
    metrics: Option<MetricsRegistry>,
    // Optional timer scanner override — None means DEFAULT_TIMER_SCAN_INTERVAL.
    timer_scan_interval: Option<Duration>,
}

impl WorkerRuntimeBuilder {
    /// Create a builder wired to a pre-built engine and its stores.
    ///
    /// ## Construction invariant
    ///
    /// `stores.execution` MUST be the same `Arc<dyn ExecutionStore>` the `engine`
    /// was wired with via `WorkflowEngine::with_execution_stores`. If they differ,
    /// the sink's idempotency read and the engine's internal lease CAS observe
    /// different rows, which breaks the idempotency contract. Passing the
    /// `ExecutionStores` bundle here makes that structurally difficult to get wrong:
    /// the same bundle that was passed to `with_execution_stores` provides the
    /// `execution` field the sink needs.
    ///
    /// Pass the **same `ExecutionStores` bundle** you handed to
    /// `WorkflowEngine::with_execution_stores` — do not construct a second bundle
    /// from a different store clone. The sink's idempotency read and the engine's
    /// lease CAS must observe the identical rows.
    ///
    /// `available_plugins` is the set of [`PluginKey`]s this worker can serve.
    /// A worker with no plugins would never claim any job; [`build`] rejects
    /// that case as [`WorkerBuildError::NoPlugins`].
    ///
    /// `processor_id` is a fixed 16-byte fence token recorded in the job row's
    /// `processed_by` field. Supply the full 16 bytes — no truncation or padding
    /// is performed, so two distinct workers with different ids cannot collapse
    /// to the same token.
    ///
    /// [`build`]: Self::build
    pub fn from_wired_engine(
        engine: Arc<WorkflowEngine>,
        stores: ExecutionStores,
        queue: Arc<dyn JobDispatchQueue>,
        available_plugins: Vec<PluginKey>,
        processor_id: [u8; 16],
    ) -> Self {
        Self {
            engine,
            stores,
            queue,
            control_queue: None,
            available_plugins,
            processor_id,
            batch_size: None,
            poll_interval: None,
            reclaim_after: None,
            reclaim_interval: None,
            max_reclaim_count: None,
            metrics: None,
            timer_scan_interval: None,
        }
    }

    /// Wire the durable control queue this worker drains.
    ///
    /// MUST be the same backend the API enqueues accepted commands onto —
    /// pointing it at a different store leaves the run undriven while every
    /// component reports healthy.
    pub fn with_control_queue(mut self, control_queue: Arc<dyn ControlQueue>) -> Self {
        self.control_queue = Some(control_queue);
        self
    }

    /// Override the claim batch size (default: [`Orchestrator`] default = 32).
    pub fn with_batch_size(mut self, n: u32) -> Self {
        self.batch_size = Some(n);
        self
    }

    /// Override the idle poll interval (default: [`Orchestrator`] default = 100 ms).
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = Some(d);
        self
    }

    /// Override the staleness window before a `Processing` row becomes reclaimable
    /// (default: [`Orchestrator`] default = 150 s).
    pub fn with_reclaim_after(mut self, d: Duration) -> Self {
        self.reclaim_after = Some(d);
        self
    }

    /// Override the reclaim sweep cadence (default: [`Orchestrator`] default = 30 s).
    pub fn with_reclaim_interval(mut self, d: Duration) -> Self {
        self.reclaim_interval = Some(d);
        self
    }

    /// Override the max retry budget before an exhausted row moves to `Failed`
    /// (default: [`Orchestrator`] default = 3).
    pub fn with_max_reclaim_count(mut self, n: u32) -> Self {
        self.max_reclaim_count = Some(n);
        self
    }

    /// Inject the shared [`MetricsRegistry`] the orchestrator emits counters into.
    ///
    /// Without this the counters increment against a private registry no scraper
    /// sees. Production composition roots should inject the shared registry so
    /// counters reach the Prometheus scrape endpoint.
    pub fn with_metrics(mut self, m: MetricsRegistry) -> Self {
        self.metrics = Some(m);
        self
    }

    /// Override the durable-timer scanner cadence (default:
    /// [`DEFAULT_TIMER_SCAN_INTERVAL`] = 30 s).
    ///
    /// Shorter intervals reduce recovery latency for stranded timers at the
    /// cost of more storage reads per unit time.
    pub fn with_timer_scan_interval(mut self, d: Duration) -> Self {
        self.timer_scan_interval = Some(d);
        self
    }

    /// Validate required fields, wire the sink, and construct [`WorkerRuntime`].
    ///
    /// # Errors
    ///
    /// Returns [`WorkerBuildError::NoPlugins`] when `available_plugins` is empty.
    pub fn build(self) -> Result<WorkerRuntime, WorkerBuildError> {
        if self.available_plugins.is_empty() {
            return Err(WorkerBuildError::NoPlugins);
        }
        let control_queue = self.control_queue.ok_or(WorkerBuildError::NoControlQueue)?;
        let timer_scan_interval = self
            .timer_scan_interval
            .unwrap_or(DEFAULT_TIMER_SCAN_INTERVAL);
        if timer_scan_interval.is_zero() {
            return Err(WorkerBuildError::ZeroTimerScanInterval);
        }

        let sink = Arc::new(EngineExecutionSink::new(
            Arc::clone(&self.engine),
            // Extract execution store from the bundle.
            // INVARIANT: this must be the same Arc passed to `with_execution_stores`
            // — enforced by documentation on `from_wired_engine`.
            Arc::clone(&self.stores.execution),
        ));

        let available_plugins_count = self.available_plugins.len();

        let mut orchestrator =
            Orchestrator::new(self.queue, sink, self.processor_id, self.available_plugins);

        if let Some(n) = self.batch_size {
            orchestrator = orchestrator.with_batch_size(n);
        }
        if let Some(d) = self.poll_interval {
            orchestrator = orchestrator.with_poll_interval(d);
        }
        if let Some(d) = self.reclaim_after {
            orchestrator = orchestrator.with_reclaim_after(d);
        }
        if let Some(d) = self.reclaim_interval {
            orchestrator = orchestrator.with_reclaim_interval(d);
        }
        if let Some(n) = self.max_reclaim_count {
            orchestrator = orchestrator.with_max_reclaim_count(n);
        }
        if let Some(m) = self.metrics {
            orchestrator = orchestrator.with_metrics(m);
        }

        // The dispatch reads status through the *same* execution store the
        // engine commits against, so the consumer's idempotency check and the
        // engine's CAS observe one row.
        let control_consumer = ControlConsumer::new(
            control_queue,
            Arc::new(EngineControlDispatch::new(
                Arc::clone(&self.engine),
                Arc::clone(&self.stores.execution),
            )),
            self.processor_id,
        );

        Ok(WorkerRuntime {
            engine: Arc::clone(&self.engine),
            timer_scan_interval,
            orchestrator,
            control_consumer,
            processor_id: self.processor_id,
            available_plugins_count,
        })
    }
}

/// Hex-encode `processor_id` bytes for structured log fields.
fn hex_id(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
