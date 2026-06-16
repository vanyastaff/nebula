#![forbid(unsafe_code)]
#![warn(missing_docs)]

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
//! ```rust,ignore
//! use std::sync::Arc;
//! use nebula_worker::WorkerRuntimeBuilder;
//!
//! let runtime = WorkerRuntimeBuilder::from_wired_engine(engine, stores, queue, plugins, proc_id)
//!     .with_batch_size(16)
//!     .build()?;
//!
//! runtime.spawn(shutdown_token);
//! ```
//!
//! [`PluginKey`]: nebula_core::PluginKey
//! [`JobDispatchQueue`]: nebula_storage_port::store::JobDispatchQueue
//! [`EngineExecutionSink`]: nebula_engine::EngineExecutionSink

use std::sync::Arc;
use std::time::Duration;

use nebula_core::PluginKey;
use nebula_engine::{EngineExecutionSink, ExecutionStores, WorkflowEngine};
use nebula_metrics::MetricsRegistry;
use nebula_orchestrator::Orchestrator;
use nebula_storage_port::store::JobDispatchQueue;
use tokio::task::JoinHandle;
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
}

/// An assembled, ready-to-run worker runtime.
///
/// Holds the [`Orchestrator`] configured with an [`EngineExecutionSink`]
/// connected to the provided engine and execution store.
///
/// Obtain via [`WorkerRuntimeBuilder::build`].
#[must_use = "call .run() or .spawn() to start the pull loop"]
pub struct WorkerRuntime {
    orchestrator: Orchestrator,
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
    /// Run the claim-loop on the current task until `shutdown` is cancelled.
    ///
    /// Prefer [`spawn`](Self::spawn) unless integrating into a custom task structure.
    ///
    /// ## Shutdown contract
    ///
    /// Mirrors [`Orchestrator::run`]: flushes the in-flight batch, then returns.
    /// Rows claimed but not yet marked remain `Processing` and are recovered by
    /// the next runner's reclaim sweep.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime starting (ADR-0095 D1)"
        );
        self.orchestrator.run(shutdown).await;
    }

    /// Spawn the claim-loop as a Tokio task.
    ///
    /// Returns a [`JoinHandle`] that completes when `shutdown` is cancelled.
    /// The caller owns signal→[`CancellationToken`] wiring; this crate provides
    /// no `tokio::signal` integration so it composes into any shutdown strategy.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<()> {
        tracing::info!(
            processor = %hex_id(&self.processor_id),
            available_plugins = self.available_plugins_count,
            "worker runtime spawning (ADR-0095 D1)"
        );
        self.orchestrator.spawn(shutdown)
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
    available_plugins: Vec<PluginKey>,
    processor_id: [u8; 16],
    // Optional orchestrator overrides — all None means "use Orchestrator defaults".
    batch_size: Option<u32>,
    poll_interval: Option<Duration>,
    reclaim_after: Option<Duration>,
    reclaim_interval: Option<Duration>,
    max_reclaim_count: Option<u32>,
    metrics: Option<MetricsRegistry>,
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
            available_plugins,
            processor_id,
            batch_size: None,
            poll_interval: None,
            reclaim_after: None,
            reclaim_interval: None,
            max_reclaim_count: None,
            metrics: None,
        }
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

    /// Validate required fields, wire the sink, and construct [`WorkerRuntime`].
    ///
    /// # Errors
    ///
    /// Returns [`WorkerBuildError::NoPlugins`] when `available_plugins` is empty.
    pub fn build(self) -> Result<WorkerRuntime, WorkerBuildError> {
        if self.available_plugins.is_empty() {
            return Err(WorkerBuildError::NoPlugins);
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

        Ok(WorkerRuntime {
            orchestrator,
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
