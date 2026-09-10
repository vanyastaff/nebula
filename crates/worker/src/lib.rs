#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # nebula-worker — Durable execution worker runtime
//!
//! A worker is a long-running process that:
//!
//! 1. Boots a flavor's plugins and derives the set of `PluginKey`s it can serve.
//! 2. Derives the exact frozen flavor identity used to claim matching control commands.
//! 3. Drains the durable control queue through [`ControlConsumer`].
//! 4. Recovers accepted turns abandoned by a previous owner and wakes overdue timers.
//! 5. Drains durable resource-event deliveries and execution handoffs.
//!
//! ## Wiring honesty
//!
//! Assembly derives its advertised flavor from the engine's exact runtime
//! configuration. The frozen registry used to execute stored plans also
//! supplies the identity and supported plugins used to claim work.
//!
//! ## Construction
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use nebula_engine::{ExecutionStores, ResourceFanoutCoordinator, WorkflowEngine};
//! use nebula_storage_port::store::{ControlQueue, ExecutionTurnHandoff, TurnRecovery};
//! use nebula_worker::WorkerRuntimeBuilder;
//! use tokio_util::sync::CancellationToken;
//!
//! # fn wire(
//! #     engine: Arc<WorkflowEngine>,
//! #     stores: ExecutionStores,
//! #     control_queue: Arc<dyn ControlQueue>,
//! #     handoff: Arc<dyn ExecutionTurnHandoff>,
//! #     recovery: Arc<dyn TurnRecovery>,
//! #     resource_fanout: Arc<ResourceFanoutCoordinator>,
//! #     proc_id: [u8; 16],
//! #     shutdown_token: CancellationToken,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let runtime = WorkerRuntimeBuilder::from_wired_engine(engine, stores, proc_id)
//!     .with_control_queue(control_queue)
//!     .with_turn_handoff(handoff)
//!     .with_turn_recovery(recovery)
//!     .with_resource_fanout(resource_fanout)
//!     .build()?;
//!
//! runtime.spawn(shutdown_token);
//! # Ok(())
//! # }
//! ```
//!
mod recovery;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nebula_engine::{
    ControlConsumer, DEFAULT_TIMER_SCAN_INTERVAL, EngineControlDispatch, ExecutionStores,
    ResourceFanoutCoordinator, WorkflowEngine,
};
use nebula_storage_port::store::{ControlQueue, ExecutionTurnHandoff, TurnRecovery};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

/// Errors that can be produced when building a [`WorkerRuntime`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerBuildError {
    /// The engine has no paired exact revision loader and frozen registry.
    #[error("exact runtime configuration is required")]
    MissingExactRuntime,

    /// No control queue was wired.
    ///
    /// Without one an execution accepted over HTTP is persisted with a
    /// `Start` command no component ever consumes: the run never begins, and
    /// the only symptom is an execution that stays `Created` forever. Requiring
    /// the queue makes that miswiring a build error instead of silence.
    #[error("no control queue wired — accepted Start commands would never be consumed")]
    NoControlQueue,

    /// No turn handoff was wired.
    ///
    /// The control consumer needs this capability to accept a claimed command
    /// and acquire the execution turn atomically.
    #[error("no turn handoff wired — control commands could not acquire durable execution turns")]
    NoTurnHandoff,

    /// No worker-wide abandoned-turn recovery capability was wired.
    #[error("no turn recovery wired — accepted turns could remain abandoned after owner loss")]
    NoTurnRecovery,

    /// No durable resource fanout coordinator was wired.
    #[error("no resource fanout wired — durable resource deliveries would never be consumed")]
    NoResourceFanout,

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

    /// Accepted-turn discovery could not read durable recovery candidates.
    #[error("accepted-turn recovery failed after {attempts} attempts: {source}")]
    AcceptedTurnRecovery {
        /// Consecutive failed discovery attempts.
        attempts: u32,
        /// The storage failure returned by the durable handoff owner.
        #[source]
        source: nebula_storage_port::StorageError,
    },

    /// Durable resource fanout exhausted its bounded infrastructure retry budget.
    #[error("resource fanout failed after {attempts} attempts; code={error_code}")]
    ResourceFanout {
        /// Consecutive failed drain attempts.
        attempts: u32,
        /// Stable payload-free coordinator failure code.
        error_code: &'static str,
        /// Original payload-free coordinator error.
        #[source]
        source: nebula_engine::ResourceFanoutCoordinatorError,
    },
}

/// What one supervised task reports: which component it was, and — when the
/// failure happened *inside* it — the join failure it carried out.
type ComponentOutcome = Result<Component, (Component, WorkerRuntimeError)>;

/// One supervised top-level worker task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Component {
    ControlConsumer,
    TimerScanner,
    AcceptedTurnRecovery,
    ResourceFanout,
    EngineShutdownRelay,
}

impl Component {
    const fn label(self) -> &'static str {
        match self {
            Self::ControlConsumer => "control-consumer",
            Self::TimerScanner => "timer-scanner",
            Self::AcceptedTurnRecovery => "accepted-turn-recovery",
            Self::ResourceFanout => "resource-fanout",
            Self::EngineShutdownRelay => "engine-shutdown-relay",
        }
    }
}

/// An assembled, ready-to-run worker runtime.
///
/// Owns the control consumer, abandoned-turn recovery, durable resource
/// fanout, and durable-timer scanner connected to one exact runtime flavor.
///
/// Obtain via [`WorkerRuntimeBuilder::build`].
#[must_use = "call .run() or .spawn() to start the worker runtime"]
pub struct WorkerRuntime {
    engine: Arc<WorkflowEngine>,
    turn_recovery: Arc<dyn TurnRecovery>,
    handoff_lease_ttl: Duration,
    worker_flavor: nebula_core::WorkerFlavorRevisionId,
    timer_scan_interval: Duration,
    /// Drains the durable control queue the API writes accepted commands to.
    ///
    /// Held by the runtime rather than by the caller so it lives and dies with
    /// the other worker components under one cancellation tree.
    control_consumer: ControlConsumer,
    resource_fanout: Arc<ResourceFanoutCoordinator>,
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
    /// Run the durable runtime components on the current task until
    /// `shutdown` is cancelled.
    ///
    /// The timer scanner and resource fanout coordinator run as sibling
    /// background tasks sharing the same shutdown token.
    ///
    /// Prefer [`spawn`](Self::spawn) unless integrating into a custom task structure.
    ///
    /// ## Shutdown contract
    ///
    /// Cancellation stops control polling, accepted-turn recovery, resource
    /// fanout, and timer scanning. In-flight engine turns observe the relayed
    /// engine shutdown.
    pub async fn run(self, shutdown: CancellationToken) -> Result<(), WorkerRuntimeError> {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime starting"
        );

        // One cancellation tree, and every top-level task joined.
        //
        // The timer scanner used to be spawned and its `JoinHandle` dropped.
        // A detached task that panics takes its failure with it: the worker
        // keeps serving, parked executions silently stop waking, and nothing
        // reports it. Joining every sibling means a component death is an error the
        // app can act on, and cancelling the token stops the siblings rather
        // than leaving them running against a half-dead runtime.
        // Each task reports which component it is, so a failure is attributed
        // rather than guessed. A task that panics yields only a `JoinError`, so
        // the id map carries the label the payload no longer can.
        let mut components: JoinSet<ComponentOutcome> = JoinSet::new();
        let mut labels: HashMap<tokio::task::Id, Component> = HashMap::new();

        // Relay this runtime's stop to the engine before anything else.
        //
        // The engine holds each in-flight execution's lease, and only the
        // engine can release it: a dropped dispatch future runs
        // `LeaseGuard::drop`, which cannot `await`. Without this relay a
        // graceful restart leaves every parked execution's lease alive for its
        // full TTL, so the successor that just started — the whole reason for
        // the restart — is fenced out of work nobody is doing.
        let engine_shutdown = self.engine.shutdown_token();
        let relay_shutdown = shutdown.clone();
        let handle = components.spawn(async move {
            relay_shutdown.cancelled().await;
            engine_shutdown.cancel();
            Ok(Component::EngineShutdownRelay)
        });
        labels.insert(handle.id(), Component::EngineShutdownRelay);

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
            // Run the initial timer sweep as a supervised sibling. A resumed
            // long action must not block control consumption or shutdown setup.
            let startup_sweep = tokio::select! {
                biased;
                () = scanner_shutdown.cancelled() => return Ok(Component::TimerScanner),
                result = scanner_engine.sweep_overdue_timers() => result,
            };
            match startup_sweep {
                Ok(redriven) => tracing::debug!(redriven, "startup timer recovery sweep completed"),
                Err(error) => tracing::error!(%error, "startup timer recovery sweep failed; periodic scanner will retry"),
            }
            // The child must also stop if this supervised parent is aborted.
            // A bare JoinHandle would detach it and retain the old engine.
            let scanner = tokio_util::task::AbortOnDropHandle::new(
                scanner_engine.spawn_timer_scanner(scan_interval, scanner_shutdown),
            );
            // The scanner owns its own task, so its failure has to be carried
            // out deliberately. Discarding the `JoinError` here would report a
            // panicked scanner as a clean stop: the runtime would keep serving
            // with nothing waking parked executions, and the supervision loop
            // would neither record the error nor stop the siblings — the exact
            // silence joining the task was meant to end.
            match scanner.await {
                Ok(()) => Ok(Component::TimerScanner),
                Err(source) => Err((
                    Component::TimerScanner,
                    WorkerRuntimeError::ComponentJoin {
                        component: Component::TimerScanner.label(),
                        source,
                    },
                )),
            }
        });
        labels.insert(handle.id(), Component::TimerScanner);

        let recovery_engine = Arc::clone(&self.engine);
        let recovery_shutdown = shutdown.clone();
        let recovery_holder = format!("recovery:{}", hex_id(&self.processor_id));
        let handle = components.spawn(async move {
            recovery::run(
                recovery_engine,
                self.turn_recovery,
                self.worker_flavor,
                recovery_holder,
                self.handoff_lease_ttl,
                recovery_shutdown,
            )
            .await
            .map(|()| Component::AcceptedTurnRecovery)
            .map_err(|source| (Component::AcceptedTurnRecovery, source))
        });
        labels.insert(handle.id(), Component::AcceptedTurnRecovery);

        let resource_fanout = self.resource_fanout;
        let resource_fanout_shutdown = shutdown.clone();
        let handle = components.spawn(async move {
            resource_fanout
                .run(resource_fanout_shutdown)
                .await
                .map(|()| Component::ResourceFanout)
                .map_err(|source| {
                    (
                        Component::ResourceFanout,
                        WorkerRuntimeError::ResourceFanout {
                            attempts: source.attempts(),
                            error_code: source.error_code(),
                            source,
                        },
                    )
                })
        });
        labels.insert(handle.id(), Component::ResourceFanout);

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
                        .unwrap_or(Component::ControlConsumer);
                    Some((
                        component,
                        WorkerRuntimeError::ComponentJoin {
                            component: component.label(),
                            source,
                        },
                    ))
                },
            };
            if let Some((component, source)) = failure {
                tracing::error!(
                    component = component.label(),
                    error = %source,
                    "worker component ended abnormally; stopping the runtime"
                );
                if first_failure.is_none() {
                    first_failure = Some(source);
                }
                shutdown.cancel();
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    /// Spawn the durable runtime components as a single Tokio task.
    ///
    /// Returns a [`JoinHandle`] that completes when `shutdown` is cancelled.
    /// Every component shares the same shutdown token so they stop together.
    /// The caller owns signal→[`CancellationToken`] wiring;
    /// this crate provides no `tokio::signal` integration so it composes into any
    /// shutdown strategy.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<Result<(), WorkerRuntimeError>> {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime spawning"
        );
        tokio::spawn(async move { self.run(shutdown).await })
    }
}

/// Builder for [`WorkerRuntime`].
///
/// Obtained via [`WorkerRuntimeBuilder::from_wired_engine`].
#[must_use = "call .build() to produce a WorkerRuntime"]
pub struct WorkerRuntimeBuilder {
    engine: Arc<WorkflowEngine>,
    stores: ExecutionStores,
    /// Durable control queue the API writes accepted commands to. Required at
    /// `build` time — see [`WorkerBuildError::NoControlQueue`].
    control_queue: Option<Arc<dyn ControlQueue>>,
    /// Durable owner of the control-claim → execution-turn handoff. Required
    /// at `build` time — see [`WorkerBuildError::NoTurnHandoff`].
    turn_handoff: Option<Arc<dyn ExecutionTurnHandoff>>,
    turn_recovery: Option<Arc<dyn TurnRecovery>>,
    resource_fanout: Option<Arc<ResourceFanoutCoordinator>>,
    processor_id: [u8; 16],
    handoff_lease_ttl: Option<Duration>,
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
    /// The engine's exact runtime configuration supplies the advertised flavor.
    /// [`Self::build`] rejects an engine without that configuration.
    ///
    /// `processor_id` is a fixed 16-byte fence token recorded on claimed work.
    /// Supply the full 16 bytes — no truncation or padding
    /// is performed, so two distinct workers with different ids cannot collapse
    /// to the same token.
    ///
    /// [`build`]: Self::build
    pub fn from_wired_engine(
        engine: Arc<WorkflowEngine>,
        stores: ExecutionStores,
        processor_id: [u8; 16],
    ) -> Self {
        Self {
            engine,
            stores,
            control_queue: None,
            turn_handoff: None,
            turn_recovery: None,
            resource_fanout: None,
            processor_id,
            handoff_lease_ttl: None,
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

    /// Wire the durable owner of the control-claim → execution-turn handoff.
    ///
    /// MUST be constructed over the same backend as the control queue and the
    /// engine's execution store because acceptance is one transaction.
    pub fn with_turn_handoff(mut self, handoff: Arc<dyn ExecutionTurnHandoff>) -> Self {
        self.turn_handoff = Some(handoff);
        self
    }

    /// Wire worker-wide discovery and acceptance of abandoned durable turns.
    ///
    /// This capability spans tenants and must remain in the worker composition
    /// root rather than a tenant-facing request state.
    pub fn with_turn_recovery(mut self, recovery: Arc<dyn TurnRecovery>) -> Self {
        self.turn_recovery = Some(recovery);
        self
    }

    /// Wire the engine-owned durable resource fanout coordinator.
    ///
    /// Its persistence roles and workflow-start service must use the same
    /// deployment backend and frozen registry as this worker's engine.
    pub fn with_resource_fanout(mut self, resource_fanout: Arc<ResourceFanoutCoordinator>) -> Self {
        self.resource_fanout = Some(resource_fanout);
        self
    }

    /// Override the lease TTL the handoff mints for each accepted turn
    /// (default: 30 s).
    pub fn with_handoff_lease_ttl(mut self, d: Duration) -> Self {
        self.handoff_lease_ttl = Some(d);
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

    /// Validate required fields and construct [`WorkerRuntime`].
    ///
    /// # Errors
    ///
    /// Returns [`WorkerBuildError::NoControlQueue`] when no control queue was
    /// wired, and [`WorkerBuildError::NoTurnHandoff`] when no turn handoff
    /// was wired. Returns [`WorkerBuildError::NoTurnRecovery`] when the
    /// worker-wide recovery capability is absent. Returns
    /// [`WorkerBuildError::NoResourceFanout`] when the durable resource
    /// coordinator is absent. Returns
    /// [`WorkerBuildError::MissingExactRuntime`] if the engine has no exact
    /// revision loader and frozen registry.
    pub fn build(self) -> Result<WorkerRuntime, WorkerBuildError> {
        let control_queue = self.control_queue.ok_or(WorkerBuildError::NoControlQueue)?;
        let turn_handoff = self.turn_handoff.ok_or(WorkerBuildError::NoTurnHandoff)?;
        let turn_recovery = self.turn_recovery.ok_or(WorkerBuildError::NoTurnRecovery)?;
        let resource_fanout = self
            .resource_fanout
            .ok_or(WorkerBuildError::NoResourceFanout)?;
        let timer_scan_interval = self
            .timer_scan_interval
            .unwrap_or(DEFAULT_TIMER_SCAN_INTERVAL);
        if timer_scan_interval.is_zero() {
            return Err(WorkerBuildError::ZeroTimerScanInterval);
        }

        let flavor = self
            .engine
            .worker_flavor_context()
            .ok_or(WorkerBuildError::MissingExactRuntime)?;
        let available_plugins_count = flavor.plugin_keys().len();
        let worker_flavor_revision = flavor.revision_id();

        // Control dispatch reads status through the same execution store the
        // engine commits against, so the consumer's idempotency check and the
        // engine's CAS observe one row.
        let control_consumer = ControlConsumer::for_flavor(
            control_queue,
            Arc::new(EngineControlDispatch::new(
                Arc::clone(&self.engine),
                Arc::clone(&self.stores.execution),
                Arc::clone(&turn_handoff),
                format!("control:{}", hex_id(&self.processor_id)),
                self.handoff_lease_ttl.unwrap_or(Duration::from_secs(30)),
            )),
            self.processor_id,
            worker_flavor_revision,
        );

        Ok(WorkerRuntime {
            engine: Arc::clone(&self.engine),
            turn_recovery,
            handoff_lease_ttl: self.handoff_lease_ttl.unwrap_or(Duration::from_secs(30)),
            worker_flavor: worker_flavor_revision,
            timer_scan_interval,
            control_consumer,
            resource_fanout,
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
