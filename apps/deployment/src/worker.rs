//! Shared core-flavor runtime assembly for first-party applications.
//!
//! Callers supply admitted storage and credential capabilities. Environment,
//! database connections and process signals stay in the executable roots.

use std::sync::Arc;
use std::time::Duration;

use nebula_core::ArtifactSetDigest;
use nebula_credential::CredentialSlotResolver;
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, EngineError, ExecutionStores,
    InProcessRunner, PluginKey, PluginWiringError, ResourceFanoutCoordinator,
    ResourceFanoutCoordinatorBuildError, WorkflowEngine, WorkflowStartBuildError,
    WorkflowStartService, WorkflowStores,
};
use nebula_metrics::MetricsRegistry;
use nebula_storage_port::{
    dto::{
        ClaimResourceRuntimeWorkRequest, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
    },
    store::{
        ExecutionTurnHandoff, ResourceEventFanoutStore, ResourceExecutionHandoffStore,
        ResourceRuntimeRecovery, ResourceSubscriptionStore, TurnRecovery,
    },
};
use nebula_worker::{WorkerBuildError, WorkerRuntimeBuilder};

use crate::{CoreRelease, CoreReleaseError};

#[cfg(feature = "runtime-repair-red")]
pub use nebula_worker::WorkerRuntimeError;

/// Typed errors emitted by the composition root.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ComposeError {
    /// The linked plugin set cannot establish an exact deployment flavor.
    #[error("linked release admission failed")]
    Release(#[from] CoreReleaseError),

    /// `WorkflowEngine::with_plugin` rejected the plugin (duplicate plugin key
    /// or duplicate action key against an already-registered action).
    #[error("plugin wiring into engine failed: {0}")]
    Wiring(#[from] PluginWiringError),

    /// Engine or action-runtime construction failed (metrics registry rejected
    /// counter/histogram registration, or the engine failed to initialize its
    /// shared state).
    #[error("engine / runtime construction failed: {0}")]
    Engine(#[from] EngineError),

    /// The linked plugins' resource factories cannot form a closed allowlist.
    #[error("resource wiring failed: {0}")]
    ResourceWiring(#[from] nebula_engine::ResourceWiringError),

    /// The credential projection cannot check availability before resource use.
    #[error("worker resources require credential availability observation")]
    MissingCredentialObserver,

    /// The workflow-start owner rejected the deployment execution budget.
    #[error("workflow-start service construction failed: {0}")]
    WorkflowStart(#[from] WorkflowStartBuildError),

    /// A fixed resource-fanout lease value violated the storage contract.
    #[error("resource fanout claim configuration is invalid: {0}")]
    ResourceLease(#[from] nebula_storage_port::dto::ResourceLeaseValueError),

    /// The fixed resource-fanout batch size violated the storage contract.
    #[error("resource fanout batch configuration is invalid: {0}")]
    ResourceBatch(#[from] nebula_storage_port::dto::SharedResourceValueError),

    /// The resource-fanout coordinator rejected its supervision settings.
    #[error("resource fanout coordinator construction failed: {0}")]
    ResourceFanout(#[from] ResourceFanoutCoordinatorBuildError),

    /// `WorkerRuntimeBuilder::build` rejected the assembled configuration.
    ///
    /// Required worker wiring or timing configuration is incomplete.
    #[error("worker runtime builder construction failed: {0}")]
    Worker(#[from] WorkerBuildError),
}

const RESOURCE_FANOUT_CLAIM_TTL: Duration = Duration::from_secs(30);
const RESOURCE_FANOUT_BATCH_SIZE: u16 = 1;
const RESOURCE_FANOUT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const RESOURCE_FANOUT_MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Same-backend inputs for durable resource fanout, stored resource rows,
/// their published runtime status and workflow starts.
#[derive(Clone)]
pub struct ResourceFanoutInputs {
    workflows: WorkflowStores,
    rows: Arc<dyn nebula_storage_port::store::ResourceStore>,
    status: Arc<dyn nebula_storage_port::store::ResourceStatusStore>,
    recovery: Arc<dyn ResourceRuntimeRecovery>,
    subscriptions: Arc<dyn ResourceSubscriptionStore>,
    fanout: Arc<dyn ResourceEventFanoutStore>,
    handoffs: Arc<dyn ResourceExecutionHandoffStore>,
    /// Store of rate limits every worker shares; `None` keeps each limit in
    /// this process.
    shared_limits: Option<Arc<dyn nebula_engine::resource::rate_limit::ErasedLimitStore>>,
}

impl std::fmt::Debug for ResourceFanoutInputs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResourceFanoutInputs")
            .finish_non_exhaustive()
    }
}

impl ResourceFanoutInputs {
    /// Project one concrete resource runtime into every durable coordinator
    /// role. `rows` holds the stored resource rows executions bind and
    /// `status` receives this worker's runtime status of them; both must
    /// live on the same backend the API reads and writes.
    #[must_use]
    pub fn from_runtime<T>(
        workflows: WorkflowStores,
        runtime: Arc<T>,
        rows: Arc<dyn nebula_storage_port::store::ResourceStore>,
        status: Arc<dyn nebula_storage_port::store::ResourceStatusStore>,
    ) -> Self
    where
        T: ResourceRuntimeRecovery
            + ResourceSubscriptionStore
            + ResourceEventFanoutStore
            + ResourceExecutionHandoffStore,
    {
        Self {
            workflows,
            rows,
            status,
            recovery: runtime.clone(),
            subscriptions: runtime.clone(),
            fanout: runtime.clone(),
            handoffs: runtime,
            shared_limits: None,
        }
    }

    /// Enforces cluster-scoped resource rate limits through `store`, shared
    /// by every worker on the same backend, so a provider's quota and its
    /// "slow down" hold across processes. Without it each worker enforces
    /// its limits alone (a single-process deployment needs nothing more).
    #[must_use]
    pub fn with_shared_limits(
        mut self,
        store: Arc<dyn nebula_engine::resource::rate_limit::ErasedLimitStore>,
    ) -> Self {
        self.shared_limits = Some(store);
        self
    }
}

/// Assemble a core-flavor `WorkerRuntime` from the supplied durable stores.
///
/// Steps performed:
/// 1. Admit the linked [`CoreRelease`] and its [`nebula_plugin::ResolvedPlugin`].
/// 2. Build a [`WorkflowEngine`] with an [`ActionRuntime`] and attach the
///    durable execution stores.
/// 3. Wire the resolved plugin via [`WorkflowEngine::with_plugin`] so the
///    engine can dispatch `core.*` actions.
/// 4. Retain the release's frozen registry with the exact catalog loader in the engine. The
///    [`WorkerRuntimeBuilder`] derives its routing context from this configuration.
/// 5. Build the workflow-start owner and durable resource coordinator from the
///    same backend roles and frozen registry, then attach it to the worker.
///
/// Returns a ready-to-configure [`WorkerRuntimeBuilder`], the shared
/// [`MetricsRegistry`], and the [`PluginKey`]
/// advertised by this flavor (`"core"`). The engine is captured inside the
/// builder's `Arc<WorkflowEngine>`; callers apply optional tuning via the
/// builder's `with_*` methods and then call `.build()` to materialise the
/// `WorkerRuntime`.
///
/// Returning the builder lets the caller attach the deployment's control queue
/// and runtime cadence before materialising the runtime.
///
/// `artifact_set_digest` identifies the worker artifact set selected by the
/// trusted deployment manifest. It is not a hash of plugin metadata or the
/// processor identity. The frozen registry combines it with the linked plugin
/// contracts and runtime contract version; this does not verify authenticity.
///
/// # Errors
///
/// Returns [`ComposeError`] if any boot step fails. All failures are
/// fail-closed: the process must not start with a mis-wired engine.
///
/// # Panics
///
/// Must be called inside a Tokio runtime: the resource manager it creates
/// starts its release workers immediately.
pub fn build_core_flavor_runtime(
    execution_stores: ExecutionStores,
    turn_handoff: Arc<dyn ExecutionTurnHandoff>,
    turn_recovery: Arc<dyn TurnRecovery>,
    processor_id: [u8; 16],
    revisions: CoreFlavorRevisionInputs,
    resource_fanout: ResourceFanoutInputs,
) -> Result<(WorkerRuntimeBuilder, MetricsRegistry, PluginKey), ComposeError> {
    build_core_flavor_runtime_impl(
        execution_stores,
        turn_handoff,
        turn_recovery,
        processor_id,
        revisions,
        resource_fanout,
        EngineEvidenceInputs::Ordinary,
    )
}

/// Assemble the core-flavor worker for the server-owned runtime-repair profile.
///
/// This purpose-specific, feature-gated seam injects the deterministic clock
/// and ephemeral execution-event bus before the engine is sealed in its
/// internal `Arc`. It delegates every other composition step to the same
/// implementation as [`build_core_flavor_runtime`]; ordinary worker behavior
/// and defaults are unchanged.
///
/// The returned builder still exposes only worker tuning. Neither the engine
/// nor its stores are returned to the server profile.
///
/// # Errors
///
/// Returns [`ComposeError`] under the same fail-closed conditions as the
/// ordinary core-flavor builder.
#[cfg(feature = "runtime-repair-red")]
pub fn build_core_flavor_runtime_for_runtime_repair_red(
    execution_stores: ExecutionStores,
    turn_handoff: Arc<dyn ExecutionTurnHandoff>,
    turn_recovery: Arc<dyn TurnRecovery>,
    processor_id: [u8; 16],
    revisions: CoreFlavorRevisionInputs,
    resource_fanout: ResourceFanoutInputs,
    evidence: RuntimeRepairEvidenceInputs,
) -> Result<(WorkerRuntimeBuilder, MetricsRegistry, PluginKey), ComposeError> {
    build_core_flavor_runtime_impl(
        execution_stores,
        turn_handoff,
        turn_recovery,
        processor_id,
        revisions,
        resource_fanout,
        EngineEvidenceInputs::RuntimeRepair(evidence),
    )
}

/// Deterministic clock and observations supplied by the server evidence profile.
#[cfg(feature = "runtime-repair-red")]
pub struct RuntimeRepairEvidenceInputs {
    /// Clock shared with the evidence scenario controls.
    pub clock: Arc<dyn nebula_core::accessor::Clock>,
    /// Ephemeral execution observations collected by the evidence scenario.
    pub event_bus: nebula_eventbus::EventBus<nebula_engine::ExecutionEvent>,
}

/// Exact revision storage and deployment identity used by the worker engine.
pub struct CoreFlavorRevisionInputs {
    /// Shared registry used by catalog and execution instrumentation.
    pub metrics: MetricsRegistry,
    /// Worker artifact identity from the trusted deployment manifest.
    pub artifact_set_digest: ArtifactSetDigest,
    /// Catalog on the same persistence backend as admitted executions.
    pub catalog: Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
    /// Contract reader on the same persistence backend as admitted executions.
    pub bundles: Arc<dyn nebula_storage_port::store::StartAcceptanceStore>,
    /// Read/project-only credential capability on the deployment credential store.
    /// Must expose both borrowed and owned availability observers for background
    /// reconciliation and per-acquire admission.
    pub credential_resolver: Arc<dyn CredentialSlotResolver>,
}

enum EngineEvidenceInputs {
    Ordinary,
    #[cfg(feature = "runtime-repair-red")]
    RuntimeRepair(RuntimeRepairEvidenceInputs),
}

/// The resource manager configuration of a worker: metrics into the engine's
/// registry, cluster rate limits through `shared_limits`, and strict
/// per-acquire credential admission through `resolver`'s availability
/// observer — every new credentialed unit reads its credential's
/// availability first, so a credential store outage refuses new
/// credentialed egress. A resolver without an observer is rejected at startup.
fn resource_manager_config(
    metrics: &MetricsRegistry,
    shared_limits: Option<Arc<dyn nebula_engine::resource::rate_limit::ErasedLimitStore>>,
    resolver: &Arc<dyn CredentialSlotResolver>,
) -> Result<nebula_engine::resource::ManagerConfig, ComposeError> {
    // Reconciliation borrows the observer; per-acquire admission owns it.
    resolver
        .as_availability_observer()
        .ok_or(ComposeError::MissingCredentialObserver)?;
    let observer = Arc::clone(resolver)
        .into_availability_observer()
        .ok_or(ComposeError::MissingCredentialObserver)?;
    let mut config = nebula_engine::resource::ManagerConfig::default()
        .with_metrics_registry(Arc::new(metrics.clone()));
    if let Some(store) = shared_limits {
        config = config.with_shared_limit_store(store);
    }
    Ok(config.with_credential_observer(observer))
}

fn build_core_flavor_runtime_impl(
    execution_stores: ExecutionStores,
    turn_handoff: Arc<dyn ExecutionTurnHandoff>,
    turn_recovery: Arc<dyn TurnRecovery>,
    processor_id: [u8; 16],
    revisions: CoreFlavorRevisionInputs,
    resource_fanout: ResourceFanoutInputs,
    evidence_inputs: EngineEvidenceInputs,
) -> Result<(WorkerRuntimeBuilder, MetricsRegistry, PluginKey), ComposeError> {
    let CoreRelease {
        plugin: resolved,
        registry: frozen,
    } = CoreRelease::new(revisions.artifact_set_digest)?;
    let plugin_key = resolved.key().clone();

    // Build the action runtime and workflow engine.
    let metrics = revisions.metrics;
    let registry = Arc::new(ActionRegistry::new());
    let runner = Arc::new(InProcessRunner::new());
    // `try_new` returns `Result<_, MetricsError>`; `MetricsError: Into<EngineError>`
    // via `EngineError::Telemetry(#[from] MetricsError)`, so `.map_err` bridges
    // the two error types through the shared `EngineError` wrapper.
    let action_runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .map_err(EngineError::from)?,
    );

    let start_clock: Arc<dyn nebula_core::accessor::Clock> = match &evidence_inputs {
        EngineEvidenceInputs::Ordinary => Arc::new(nebula_core::accessor::SystemClock),
        #[cfg(feature = "runtime-repair-red")]
        EngineEvidenceInputs::RuntimeRepair(evidence) => Arc::clone(&evidence.clock),
    };
    // Stored resource rows are activated lazily, per row, when an execution
    // that binds them is driven; nothing is read or connected at boot.
    let manager_config = resource_manager_config(
        &metrics,
        resource_fanout.shared_limits.clone(),
        &revisions.credential_resolver,
    )?;
    let engine = WorkflowEngine::new(action_runtime, metrics.clone())?
        .with_execution_stores(execution_stores.clone())
        .with_credential_resolver(revisions.credential_resolver)
        .with_resource_manager(Arc::new(nebula_engine::resource::Manager::with_config(
            manager_config,
        )))
        .with_stored_resources(nebula_engine::StoredResourceActivator::new(Arc::clone(
            &resource_fanout.rows,
        )));
    let engine = match evidence_inputs {
        EngineEvidenceInputs::Ordinary => engine,
        #[cfg(feature = "runtime-repair-red")]
        EngineEvidenceInputs::RuntimeRepair(evidence) => engine
            .with_clock(evidence.clock)
            .with_event_bus(evidence.event_bus),
    };

    // Wire the core plugin into the engine.
    let engine = engine.with_plugin(Arc::clone(&resolved))?;
    let start_service = Arc::new(WorkflowStartService::new(
        resource_fanout.workflows,
        Arc::clone(&execution_stores.execution),
        Arc::clone(&revisions.bundles),
        nebula_engine::PlanFlavorRevisionLoader::new(Arc::clone(&revisions.catalog)),
        Arc::clone(&frozen),
        start_clock,
        Default::default(),
    )?);
    let resource_claim = ClaimResourceRuntimeWorkRequest::new(
        ResourceLeaseHolder::new(format!("resource-fanout:{}", hex_id(&processor_id)))?,
        ResourceLeaseTtl::new(RESOURCE_FANOUT_CLAIM_TTL)?,
        ResourcePageSize::new(RESOURCE_FANOUT_BATCH_SIZE)?,
    );
    let resource_status = resource_fanout.status;
    let resource_fanout = Arc::new(ResourceFanoutCoordinator::new(
        resource_fanout.recovery,
        resource_fanout.subscriptions,
        resource_fanout.fanout,
        resource_fanout.handoffs,
        start_service,
        resource_claim,
        RESOURCE_FANOUT_POLL_INTERVAL,
        RESOURCE_FANOUT_MAX_CONSECUTIVE_FAILURES,
    )?);
    // The closed kind allowlist stored resource rows are activated through;
    // the API validates configs through an allowlist built from the same
    // plugin set, so a kind it accepts is one this engine can register.
    let resource_registrars = nebula_engine::resource_registrars_from(
        frozen.all_resources().map(|(_plugin, factory)| factory),
    )?;
    let engine = Arc::new(
        engine
            .with_resource_registrars(resource_registrars)
            .with_plan_flavor_runtime(
                Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(
                    revisions.catalog,
                )),
                frozen,
                revisions.bundles,
            ),
    );

    // Construct the worker runtime builder.
    //
    // The turn handoff and global recovery capability are deployment-owned
    // durable boundaries, so composition wires them before returning the builder.
    let builder = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        execution_stores,
        processor_id,
    )
    .with_turn_handoff(turn_handoff)
    .with_turn_recovery(turn_recovery)
    .with_resource_fanout(resource_fanout)
    .with_resource_status_store(resource_status);

    tracing::info!(
        plugin = %plugin_key,
        processor = %hex_id(&processor_id),
        "core-flavor plugin wired; worker runtime builder ready"
    );

    Ok((builder, metrics, plugin_key))
}

/// Hex-encode a processor-id byte slice for structured log fields.
fn hex_id(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A projection resolver that is also its own availability observer, as
    /// the first-party `CredentialProjectionRuntime` is.
    struct ObservingResolver {
        observer: bool,
    }

    impl CredentialSlotResolver for ObservingResolver {
        fn as_availability_observer(
            &self,
        ) -> Option<&dyn nebula_credential::CredentialAvailabilityObserver> {
            self.observer.then_some(self as &_)
        }

        fn resolve_slot<'a>(
            &'a self,
            _scope: &'a nebula_credential::TenantScope,
            _credential_id: nebula_credential::CredentialId,
            _expected_key: nebula_credential::CredentialKey,
            _required_capabilities: nebula_credential::Capabilities,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn Future<
                        Output = Result<
                            nebula_credential::ErasedCredentialGuard,
                            nebula_credential::CredentialSlotResolveError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(nebula_credential::CredentialSlotResolveError::Unavailable) })
        }

        fn into_availability_observer(
            self: Arc<Self>,
        ) -> Option<Arc<dyn nebula_credential::CredentialAvailabilityObserver>> {
            self.observer.then_some(self as Arc<_>)
        }
    }

    impl nebula_credential::CredentialAvailabilityObserver for ObservingResolver {
        fn observe_availability<'a>(
            &'a self,
            _scope: &'a nebula_credential::TenantScope,
            _credential_id: nebula_credential::CredentialId,
            _expected_key: nebula_credential::CredentialKey,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn Future<
                        Output = Result<
                            nebula_credential::CredentialAvailabilityObservation,
                            nebula_credential::CredentialObserveError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(nebula_credential::CredentialObserveError::Unavailable) })
        }
    }

    #[test]
    fn the_worker_manager_reads_credential_availability_per_acquire() {
        let resolver: Arc<dyn CredentialSlotResolver> =
            Arc::new(ObservingResolver { observer: true });
        let config = resource_manager_config(&MetricsRegistry::new(), None, &resolver)
            .expect("resolver supplies an observer");
        assert!(
            config.credential_observer.is_some(),
            "the composed manager is strict"
        );
        assert!(config.metrics_registry.is_some());

        let resolver: Arc<dyn CredentialSlotResolver> =
            Arc::new(ObservingResolver { observer: false });
        assert!(
            matches!(
                resource_manager_config(&MetricsRegistry::new(), None, &resolver),
                Err(ComposeError::MissingCredentialObserver)
            ),
            "a resolver without an observer cannot configure worker resources"
        );
    }
}
