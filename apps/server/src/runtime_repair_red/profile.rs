//! Closed first-party runtime composition used only for RED evidence.

use std::{future::Future, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use nebula_api::{
    ApiConfig, AppState,
    config::{ExecutionBackendKind, ExecutionStoreConfig},
    domain::{
        auth::backend::{
            AuthBackend, InMemoryAuthBackend, SecretString as ApiSecretString, SignupRequest,
        },
        org::InMemoryMembershipStore,
    },
    error::ApiError,
    middleware::InMemoryIdempotencyStore,
    state::{OrgResolver, WorkspaceResolver},
};
use nebula_core::{OrgId, WorkspaceId, accessor::Clock};
use nebula_engine::ExecutionEvent;
use nebula_eventbus::{EventBus, Subscriber};
use secrecy::SecretString;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    task::{JoinError, JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

use super::evidence::{
    EvidenceControls, EvidenceIntegrityError, FileSqliteReopenController,
    LifecycleObservationRegistry, LivePostgresIntegrityProbe,
};
use crate::{
    compose::{self, ProfileBackendLifecycle, TransportInitError},
    transport::{ApiTransport, ServerTransport},
};

/// Exact profile label required by the accepted runtime-repair specification.
pub const PROFILE_LABEL: &str =
    "first-party RED conformance profile (evidence-only; non-deployment; non-SDK)";

const PROFILE_ORG_ID: &str = "org_00000000000000000000000001";
const PROFILE_WORKSPACE_ID: &str = "ws_00000000000000000000000001";
const PROFILE_EMAIL: &str = "runtime-repair@nebula.invalid";
const PROFILE_PASSWORD: &str = "runtime-repair-evidence-password";
const PROFILE_DISPLAY_NAME: &str = "Runtime Repair Evidence";
const PROFILE_PROCESSOR_ID: [u8; 16] = *b"runtime-repair-1";

/// How often the profile sweeps for overdue parked waits.
///
/// The product default is 30 s, which is the right cadence for a deployment
/// but far longer than an evidence run should wait: this profile drives time
/// with an injected clock, so a scenario advances a 60-second delay in an
/// instant and then has to observe the wake. The scan cadence is wall-clock —
/// it is not the behaviour under test, only how often the behaviour is
/// polled — so the profile shortens it rather than making every scenario sit
/// through a production interval.
const PROFILE_TIMER_SCAN_INTERVAL: Duration = Duration::from_millis(25);
const EXECUTION_EVENT_BUFFER: usize = 1_024;

/// Explicit closed configuration for the app-owned RED profile.
///
/// Private fields prevent callers from injecting routers, state, stores,
/// queues, pools, controllers, registries, proofs, tokens, or authority.
pub struct RuntimeRepairProfileConfig {
    preset: ClosedPreset,
    backend: ProfileBackend,
}

#[derive(Debug, Clone, Copy)]
enum ClosedPreset {
    FirstPartyRed,
}

#[derive(Clone)]
enum ProfileBackend {
    InMemory,
    FileSqlite(FileSqliteReopenController),
    LivePostgres(LivePostgresIntegrityProbe),
}

impl std::fmt::Debug for RuntimeRepairProfileConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match self.backend {
            ProfileBackend::InMemory => "in-memory-reference",
            ProfileBackend::FileSqlite(_) => "file-sqlite",
            ProfileBackend::LivePostgres(_) => "live-postgresql-required",
        };
        formatter
            .debug_struct("RuntimeRepairProfileConfig")
            .field("profile", &PROFILE_LABEL)
            .field("backend", &backend)
            .finish()
    }
}

impl RuntimeRepairProfileConfig {
    /// Select the process-local InMemory reference backend.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            preset: ClosedPreset::FirstPartyRed,
            backend: ProfileBackend::InMemory,
        }
    }

    /// Select an internally provisioned file-SQLite backend.
    ///
    /// Its path remains opaque and is retained by the harness so repeated
    /// launches can close and reopen the same file.
    #[must_use]
    pub fn file_sqlite() -> Self {
        Self {
            preset: ClosedPreset::FirstPartyRed,
            backend: ProfileBackend::FileSqlite(FileSqliteReopenController::new()),
        }
    }

    /// Select required live PostgreSQL.
    ///
    /// The DSN is held as a secret and never appears on the lifecycle handle,
    /// in diagnostics, or in evidence artifacts. Launch fails rather than
    /// skipping or substituting another backend when PostgreSQL is unavailable.
    #[must_use]
    pub fn live_postgres(database_dsn: SecretString) -> Self {
        let database_dsn = Arc::new(database_dsn);
        Self {
            preset: ClosedPreset::FirstPartyRed,
            backend: ProfileBackend::LivePostgres(LivePostgresIntegrityProbe::new(database_dsn)),
        }
    }

    /// Retain this closed configuration with its authority-free controls.
    #[must_use]
    pub fn into_harness(self) -> RuntimeRepairHarness {
        let evidence_controls = match &self.backend {
            ProfileBackend::InMemory => EvidenceControls::in_memory(),
            ProfileBackend::FileSqlite(controller) => {
                EvidenceControls::file_sqlite(controller.clone())
            },
            ProfileBackend::LivePostgres(probe) => EvidenceControls::live_postgres(probe.clone()),
        };
        RuntimeRepairHarness {
            configuration: self,
            evidence_controls,
        }
    }
}

/// Retained profile fixture with deterministic evidence controls.
///
/// The fixture exposes no durable store or authority capability. The lifecycle
/// returned from [`launch`](Self::launch) is a separate opaque handle.
///
/// `nebula-worker` currently starts its durable timer scanner internally and
/// drops that nested `JoinHandle`. The profile's single cancellation tree does
/// stop the scanner, but a scanner-only panic cannot yet be surfaced by this
/// app supervisor. HTTP, the worker pull-loop, and the sanitized lifecycle
/// observer are fully owned and joined; making the nested scanner join-visible
/// requires a later worker-runtime API change and is recorded as a residual
/// integrity limitation.
pub struct RuntimeRepairHarness {
    configuration: RuntimeRepairProfileConfig,
    evidence_controls: EvidenceControls,
}

impl std::fmt::Debug for RuntimeRepairHarness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeRepairHarness")
            .field("configuration", &self.configuration)
            .field("evidence_controls", &self.evidence_controls)
            .finish()
    }
}

impl RuntimeRepairHarness {
    /// Borrow the authority-free deterministic controls.
    #[must_use]
    pub fn evidence_controls(&self) -> &EvidenceControls {
        &self.evidence_controls
    }

    /// Construct and launch the closed first-party profile.
    ///
    /// This never reads process-global configuration, installs telemetry, or
    /// installs OS signal handlers. The listener is bound before readiness,
    /// and readiness is published only after HTTP, worker, and lifecycle
    /// observer siblings have entered the supervisor's start barrier.
    ///
    /// # Errors
    ///
    /// Returns a secret-free typed error if closed preset construction,
    /// required backend integrity, listener binding, worker composition, or
    /// task startup fails.
    pub async fn launch(&self) -> Result<RuntimeRepairProfileHandle, RuntimeRepairProfileError> {
        debug_assert!(matches!(
            self.configuration.preset,
            ClosedPreset::FirstPartyRed
        ));

        let mut api_config = ApiConfig::for_test();
        let explicit_postgres_dsn = match &self.configuration.backend {
            ProfileBackend::InMemory => {
                api_config.execution = ExecutionStoreConfig {
                    backend: ExecutionBackendKind::Memory,
                    db_path: String::new(),
                };
                None
            },
            ProfileBackend::FileSqlite(controller) => {
                api_config.execution = ExecutionStoreConfig {
                    backend: ExecutionBackendKind::Sqlite,
                    db_path: controller.database_path().to_string_lossy().into_owned(),
                };
                None
            },
            ProfileBackend::LivePostgres(probe) => {
                probe
                    .verify_required()
                    .await
                    .map_err(ProfileErrorKind::Evidence)?;
                api_config.execution = ExecutionStoreConfig {
                    backend: ExecutionBackendKind::Postgres,
                    db_path: String::new(),
                };
                Some(probe.database_dsn())
            },
        };

        let execution_bundle = compose::build_execution_stores(&api_config, explicit_postgres_dsn)
            .await
            .map_err(ProfileErrorKind::Composition)?;
        let worker_projection = execution_bundle.worker_projection();
        let backend_lifecycle = execution_bundle.backend_lifecycle();
        let metrics_registry = Arc::new(nebula_metrics::MetricsRegistry::new());
        let mut state =
            compose::default_state(&api_config, Arc::clone(&metrics_registry), execution_bundle)
                .map_err(ProfileErrorKind::Composition)?;
        state = compose_closed_identity(state, &api_config).await?;
        let bind_address = SocketAddr::from(([127, 0, 0, 1], 0));
        state = ApiTransport
            .prepare_state(state, bind_address)
            .map_err(ProfileErrorKind::Composition)?;
        let router = ApiTransport
            .build_router(state, &api_config)
            .map_err(ProfileErrorKind::Composition)?;

        let execution_event_bus = EventBus::<ExecutionEvent>::new(EXECUTION_EVENT_BUFFER);
        let execution_event_subscriber = execution_event_bus.subscribe();
        let engine_clock: Arc<dyn Clock> = self.evidence_controls.clock();
        let (worker_builder, worker_metrics, _) =
            nebula_worker_bin::compose::build_core_flavor_runtime_for_runtime_repair_red(
                worker_projection.execution_stores,
                worker_projection.workflow_stores,
                worker_projection.job_dispatch_queue,
                worker_projection.turn_handoff,
                PROFILE_PROCESSOR_ID,
                engine_clock,
                execution_event_bus,
            )
            .map_err(ProfileErrorKind::WorkerComposition)?;
        // The same control queue `AppState` enqueues accepted commands onto.
        // Without this the profile drains job-dispatch rows only, and an
        // execution accepted over HTTP sits `Created` with a `Start` command no
        // component consumes — the run never begins.
        let worker_runtime = worker_builder
            .with_control_queue(worker_projection.control_queue)
            .with_timer_scan_interval(PROFILE_TIMER_SCAN_INTERVAL)
            .with_metrics(worker_metrics)
            .build()
            .map_err(|_| ProfileErrorKind::WorkerBuild)?;

        let listener = TcpListener::bind(bind_address)
            .await
            .map_err(ProfileErrorKind::Listener)?;
        let address = listener.local_addr().map_err(ProfileErrorKind::Listener)?;
        let shutdown = CancellationToken::new();
        let (readiness, readiness_receiver) = watch::channel(ReadinessState::Starting);
        let supervisor_shutdown = shutdown.clone();
        let worker_shutdown = shutdown.clone();
        let observations = self.evidence_controls.observation_registry();
        let worker_future = async move {
            // A component death inside the worker is reported, not swallowed:
            // a runtime whose control consumer stopped is no longer draining
            // accepted commands and must not read as healthy.
            if let Err(error) = worker_runtime.run(worker_shutdown).await {
                tracing::error!(%error, "profile worker runtime ended abnormally");
            }
        };
        let supervisor = tokio::spawn(async move {
            supervise_profile(ProfileSupervisorInputs {
                router,
                listener,
                worker_future,
                execution_events: execution_event_subscriber,
                observations,
                backend_lifecycle,
                shutdown: supervisor_shutdown,
                readiness,
            })
            .await
        });

        Ok(RuntimeRepairProfileHandle {
            address,
            readiness: readiness_receiver,
            shutdown,
            supervisor: Some(supervisor),
        })
    }
}

async fn compose_closed_identity(
    state: AppState,
    api_config: &ApiConfig,
) -> Result<AppState, RuntimeRepairProfileError> {
    let auth_backend = Arc::new(InMemoryAuthBackend::new());
    let bootstrap_user = auth_backend
        .register_user(SignupRequest {
            email: PROFILE_EMAIL.to_owned(),
            password: ApiSecretString::new(PROFILE_PASSWORD.to_owned()),
            display_name: PROFILE_DISPLAY_NAME.to_owned(),
        })
        .await
        .map_err(ProfileErrorKind::Authentication)?;
    let membership =
        InMemoryMembershipStore::seeded_bootstrap(PROFILE_ORG_ID, &bootstrap_user.user_id)
            .map_err(ProfileErrorKind::Membership)?;
    let tenant_resolver = Arc::new(ClosedTenantResolver::parse()?);
    let idempotency_store = InMemoryIdempotencyStore::with_ttl_and_capacity(
        Duration::from_secs(api_config.idempotency.ttl_secs),
        api_config.idempotency.max_entries,
    );

    let auth_backend: Arc<dyn AuthBackend> = auth_backend;
    Ok(state
        .with_auth_backend(auth_backend)
        .with_membership_store(membership.into_arc())
        .with_org_resolver(tenant_resolver.clone())
        .with_workspace_resolver(tenant_resolver)
        .with_idempotency_store(Arc::new(idempotency_store)))
}

#[derive(Debug)]
struct ClosedTenantResolver {
    organization_id: OrgId,
    workspace_id: WorkspaceId,
}

impl ClosedTenantResolver {
    fn parse() -> Result<Self, RuntimeRepairProfileError> {
        let organization_id =
            OrgId::from_str(PROFILE_ORG_ID).map_err(|_| ProfileErrorKind::InvalidClosedPreset)?;
        let workspace_id = WorkspaceId::from_str(PROFILE_WORKSPACE_ID)
            .map_err(|_| ProfileErrorKind::InvalidClosedPreset)?;
        Ok(Self {
            organization_id,
            workspace_id,
        })
    }
}

#[async_trait]
impl OrgResolver for ClosedTenantResolver {
    async fn resolve_by_slug(&self, slug: &str) -> Result<OrgId, ApiError> {
        if slug == "runtime-repair" {
            Ok(self.organization_id)
        } else {
            Err(ApiError::NotFound("organization not found".to_owned()))
        }
    }
}

#[async_trait]
impl WorkspaceResolver for ClosedTenantResolver {
    async fn resolve_by_slug(
        &self,
        organization_id: OrgId,
        slug: &str,
    ) -> Result<WorkspaceId, ApiError> {
        if organization_id == self.organization_id && slug == "runtime-repair" {
            Ok(self.workspace_id)
        } else {
            Err(ApiError::NotFound("workspace not found".to_owned()))
        }
    }

    async fn resolve_by_id(
        &self,
        organization_id: OrgId,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceId, ApiError> {
        if organization_id == self.organization_id && workspace_id == self.workspace_id {
            Ok(workspace_id)
        } else {
            Err(ApiError::NotFound("workspace not found".to_owned()))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Component {
    Http,
    Worker,
    Observer,
}

#[derive(Debug)]
enum ComponentSignal {
    Staged(Component),
    Started(Component),
}

#[derive(Debug)]
enum ComponentExit {
    Http(Result<(), std::io::Error>),
    Worker,
    Observer(Result<(), EvidenceIntegrityError>),
    /// Shutdown arrived before the supervisor opened the start gate, so this
    /// component ended without ever entering its run phase.
    NeverStarted,
}

struct ProfileSupervisorInputs<WorkerFuture> {
    router: axum::Router,
    listener: TcpListener,
    worker_future: WorkerFuture,
    execution_events: Subscriber<ExecutionEvent>,
    observations: LifecycleObservationRegistry,
    backend_lifecycle: ProfileBackendLifecycle,
    shutdown: CancellationToken,
    readiness: watch::Sender<ReadinessState>,
}

async fn supervise_profile<WorkerFuture>(
    inputs: ProfileSupervisorInputs<WorkerFuture>,
) -> Result<(), RuntimeRepairProfileError>
where
    WorkerFuture: Future<Output = ()> + Send + 'static,
{
    let ProfileSupervisorInputs {
        router,
        listener,
        worker_future,
        execution_events,
        observations,
        backend_lifecycle,
        shutdown,
        readiness,
    } = inputs;
    let (component_signals, mut signals) = mpsc::unbounded_channel();
    let (start, _) = watch::channel(false);
    let mut components = JoinSet::new();

    let http_shutdown = shutdown.clone();
    let http_signals = component_signals.clone();
    let mut http_start = start.subscribe();
    components.spawn(async move {
        let _ = http_signals.send(ComponentSignal::Staged(Component::Http));
        if wait_for_start(&mut http_start, &http_shutdown).await == StartGate::Aborted {
            return ComponentExit::NeverStarted;
        }
        let _ = http_signals.send(ComponentSignal::Started(Component::Http));
        ComponentExit::Http(
            compose::serve_prebound(router, listener, http_shutdown.cancelled_owned()).await,
        )
    });

    let worker_signals = component_signals.clone();
    let worker_shutdown = shutdown.clone();
    let mut worker_start = start.subscribe();
    components.spawn(async move {
        let _ = worker_signals.send(ComponentSignal::Staged(Component::Worker));
        if wait_for_start(&mut worker_start, &worker_shutdown).await == StartGate::Aborted {
            return ComponentExit::NeverStarted;
        }
        let _ = worker_signals.send(ComponentSignal::Started(Component::Worker));
        worker_future.await;
        ComponentExit::Worker
    });

    let observer_signals = component_signals.clone();
    let observer_shutdown = shutdown.clone();
    let mut observer_start = start.subscribe();
    components.spawn(async move {
        let _ = observer_signals.send(ComponentSignal::Staged(Component::Observer));
        if wait_for_start(&mut observer_start, &observer_shutdown).await == StartGate::Aborted {
            return ComponentExit::NeverStarted;
        }
        let _ = observer_signals.send(ComponentSignal::Started(Component::Observer));
        ComponentExit::Observer(
            observe_execution_lifecycle(execution_events, observations, observer_shutdown).await,
        )
    });
    drop(component_signals);

    if let Err(error) =
        wait_for_components(&mut signals, &mut components, SignalPhase::Staged).await
    {
        shutdown.cancel();
        while components.join_next().await.is_some() {}
        backend_lifecycle.close().await;
        return Err(error);
    }
    start.send_replace(true);
    if let Err(error) =
        wait_for_components(&mut signals, &mut components, SignalPhase::Started).await
    {
        shutdown.cancel();
        while components.join_next().await.is_some() {}
        backend_lifecycle.close().await;
        return Err(error);
    }
    readiness.send_replace(ReadinessState::Ready);

    let mut first_failure = None;
    while let Some(join_result) = components.join_next().await {
        let component_failure = classify_component_exit(join_result, shutdown.is_cancelled());
        if first_failure.is_none()
            && let Some(component_failure) = component_failure
        {
            first_failure = Some(component_failure);
            shutdown.cancel();
        }
    }
    readiness.send_replace(ReadinessState::Stopped);
    backend_lifecycle.close().await;
    first_failure.map_or(Ok(()), Err)
}

async fn observe_execution_lifecycle(
    mut execution_events: Subscriber<ExecutionEvent>,
    observations: LifecycleObservationRegistry,
    shutdown: CancellationToken,
) -> Result<(), EvidenceIntegrityError> {
    loop {
        if shutdown.is_cancelled() {
            while let Some(event) = execution_events.try_recv() {
                observations.record_event(event)?;
                if execution_events.lagged_count() > 0 {
                    return Err(EvidenceIntegrityError::ObservationLagged);
                }
            }
            return if execution_events.lagged_count() > 0 {
                Err(EvidenceIntegrityError::ObservationLagged)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            () = shutdown.cancelled() => {},
            event = execution_events.recv() => {
                let Some(event) = event else {
                    return if shutdown.is_cancelled() {
                        Ok(())
                    } else {
                        Err(EvidenceIntegrityError::ControlClosed)
                    };
                };
                observations.record_event(event)?;
                if execution_events.lagged_count() > 0 {
                    return Err(EvidenceIntegrityError::ObservationLagged);
                }
            },
        }
    }
}

/// Outcome of waiting on the supervisor's start gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartGate {
    /// The supervisor released the components into their run phase.
    Released,
    /// Shutdown arrived first; the component must not enter its run phase.
    Aborted,
}

/// Wait for the supervisor to open the start gate, or for shutdown.
///
/// Observing `shutdown` is what makes the staged-failure path finite. The
/// supervisor cancels and then drains, but it only opens the gate *after* that
/// drain — so a wait that watched the gate alone would park forever holding
/// the drain open, and the sender it is waiting on is a live local in the
/// supervisor's own frame, so it never closes either.
async fn wait_for_start(
    start: &mut watch::Receiver<bool>,
    shutdown: &CancellationToken,
) -> StartGate {
    loop {
        if *start.borrow_and_update() {
            return StartGate::Released;
        }
        if shutdown.is_cancelled() {
            return StartGate::Aborted;
        }
        tokio::select! {
            () = shutdown.cancelled() => return StartGate::Aborted,
            changed = start.changed() => {
                if changed.is_err() {
                    return StartGate::Aborted;
                }
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SignalPhase {
    Staged,
    Started,
}

async fn wait_for_components(
    signals: &mut mpsc::UnboundedReceiver<ComponentSignal>,
    components: &mut JoinSet<ComponentExit>,
    expected_phase: SignalPhase,
) -> Result<(), RuntimeRepairProfileError> {
    let mut observed_http = false;
    let mut observed_worker = false;
    let mut observed_observer = false;
    while !(observed_http && observed_worker && observed_observer) {
        tokio::select! {
            signal = signals.recv() => {
                let Some(signal) = signal else {
                    return Err(ProfileErrorKind::ComponentEndedBeforeReadiness.into());
                };
                let component = match (expected_phase, signal) {
                    (SignalPhase::Staged, ComponentSignal::Staged(component))
                    | (SignalPhase::Started, ComponentSignal::Started(component)) => component,
                    _ => continue,
                };
                match component {
                    Component::Http => observed_http = true,
                    Component::Worker => observed_worker = true,
                    Component::Observer => observed_observer = true,
                }
            },
            join_result = components.join_next() => {
                return match join_result {
                    Some(Err(source)) => Err(ProfileErrorKind::ComponentPanicked(source).into()),
                    Some(Ok(_)) | None => Err(ProfileErrorKind::ComponentEndedBeforeReadiness.into()),
                };
            },
        }
    }
    Ok(())
}

fn classify_component_exit(
    join_result: Result<ComponentExit, JoinError>,
    is_shutting_down: bool,
) -> Option<RuntimeRepairProfileError> {
    match join_result {
        Err(source) => Some(ProfileErrorKind::ComponentPanicked(source).into()),
        Ok(ComponentExit::Http(Err(source))) => {
            Some(ProfileErrorKind::HttpComponent(source).into())
        },
        Ok(ComponentExit::Observer(Err(source))) => {
            Some(ProfileErrorKind::ObservationComponent(source).into())
        },
        Ok(
            ComponentExit::Http(Ok(()))
            | ComponentExit::Worker
            | ComponentExit::Observer(Ok(()))
            | ComponentExit::NeverStarted,
        ) if !is_shutting_down => Some(ProfileErrorKind::ComponentExited.into()),
        Ok(
            ComponentExit::Http(Ok(()))
            | ComponentExit::Worker
            | ComponentExit::Observer(Ok(()))
            | ComponentExit::NeverStarted,
        ) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadinessState {
    Starting,
    Ready,
    Stopped,
}

/// Opaque lifecycle for one running profile.
///
/// It exposes only the bound address, readiness wait, cancellation request,
/// and joined completion. Dropping it cancels and aborts the supervisor so no
/// top-level task is detached; callers should prefer `shutdown` plus `join` for
/// graceful teardown and error propagation.
pub struct RuntimeRepairProfileHandle {
    address: SocketAddr,
    readiness: watch::Receiver<ReadinessState>,
    shutdown: CancellationToken,
    supervisor: Option<JoinHandle<Result<(), RuntimeRepairProfileError>>>,
}

impl RuntimeRepairProfileHandle {
    /// Return the already-bound loopback address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.address
    }

    /// Wait for every selected top-level component to enter its run phase.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervisor ends before readiness.
    pub async fn readiness(&self) -> Result<(), RuntimeRepairProfileError> {
        let mut readiness = self.readiness.clone();
        loop {
            match *readiness.borrow_and_update() {
                ReadinessState::Ready => return Ok(()),
                ReadinessState::Stopped => {
                    return Err(ProfileErrorKind::ComponentEndedBeforeReadiness.into());
                },
                ReadinessState::Starting => {},
            }
            readiness
                .changed()
                .await
                .map_err(|_| ProfileErrorKind::ComponentEndedBeforeReadiness)?;
        }
    }

    /// Cancel the profile's single top-level cancellation tree.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Join the supervisor and surface every top-level error or panic.
    ///
    /// # Errors
    ///
    /// Returns a component or supervisor panic/error after all siblings have
    /// been cancelled and joined.
    pub async fn join(mut self) -> Result<(), RuntimeRepairProfileError> {
        let supervisor = self
            .supervisor
            .take()
            .ok_or(ProfileErrorKind::SupervisorAlreadyJoined)?;
        supervisor
            .await
            .map_err(ProfileErrorKind::SupervisorPanicked)?
    }
}

impl Drop for RuntimeRepairProfileHandle {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.abort();
        }
    }
}

/// Secret-free launch or lifecycle error for the RED profile.
///
/// The kind is boxed so every `Result<_, RuntimeRepairProfileError>` on this
/// module's hot launch path stays pointer-sized; the widest source variant is
/// well over a hundred bytes.
pub struct RuntimeRepairProfileError(Box<ProfileErrorKind>);

impl std::fmt::Debug for RuntimeRepairProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("RuntimeRepairProfileError")
            .field(&self.0.to_string())
            .finish()
    }
}

impl std::fmt::Display for RuntimeRepairProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for RuntimeRepairProfileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<ProfileErrorKind> for RuntimeRepairProfileError {
    fn from(error: ProfileErrorKind) -> Self {
        Self(Box::new(error))
    }
}

#[derive(Debug, Error)]
enum ProfileErrorKind {
    #[error("closed RED profile state composition failed")]
    Composition(#[source] TransportInitError),
    #[error("closed RED profile authentication seed failed")]
    Authentication(#[source] nebula_api::domain::auth::backend::AuthError),
    #[error("closed RED profile membership seed failed")]
    Membership(#[source] nebula_api::domain::org::BootstrapSeedError),
    #[error("closed RED profile constants are invalid")]
    InvalidClosedPreset,
    #[error("worker flavor composition failed")]
    WorkerComposition(#[source] nebula_worker_bin::compose::ComposeError),
    #[error("worker runtime build rejected the closed preset")]
    WorkerBuild,
    #[error("profile listener setup failed")]
    Listener(#[source] std::io::Error),
    #[error("profile evidence integrity failed")]
    Evidence(#[source] EvidenceIntegrityError),
    #[error("a profile component ended before readiness")]
    ComponentEndedBeforeReadiness,
    #[error("a profile component exited without shutdown")]
    ComponentExited,
    #[error("profile HTTP component failed")]
    HttpComponent(#[source] std::io::Error),
    #[error("profile lifecycle observation component failed")]
    ObservationComponent(#[source] EvidenceIntegrityError),
    #[error("a profile component panicked")]
    ComponentPanicked(#[source] JoinError),
    #[error("profile supervisor panicked")]
    SupervisorPanicked(#[source] JoinError),
    #[error("profile supervisor was already joined")]
    SupervisorAlreadyJoined,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_core::{NodeKey, id::ExecutionId};

    use super::*;

    /// Red→green proof that the staged-failure path can actually drain.
    ///
    /// On that path the supervisor cancels, then drains, and only opens the
    /// start gate afterwards — so a component parked on the gate alone would
    /// hold the drain open forever. The gate's sender is a live local in the
    /// supervisor's frame, so it never closes; this test keeps `start` alive
    /// to reproduce exactly that, and the wait must still return.
    #[tokio::test]
    async fn start_gate_releases_on_shutdown_so_a_staged_failure_can_drain() {
        let (start, mut subscriber) = watch::channel(false);
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        let gate = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_start(&mut subscriber, &shutdown),
        )
        .await
        .expect("the start gate must not park once shutdown is requested");

        assert_eq!(
            gate,
            StartGate::Aborted,
            "a component must abort rather than enter its run phase after shutdown"
        );
        // Held to the end on purpose: the supervisor owns the sender for its
        // whole frame, so `changed()` never errors and cannot free the wait.
        drop(start);
    }

    #[tokio::test]
    async fn shutdown_materializes_pending_eventbus_lag_before_success() {
        let bus = EventBus::new(1);
        let subscriber = bus.subscribe();
        let observations = LifecycleObservationRegistry::new();
        let shutdown = CancellationToken::new();
        let node_key = NodeKey::new("lag_probe").expect("valid node key");

        for _ in 0..2 {
            let _ = bus.emit(ExecutionEvent::NodeStarted {
                execution_id: ExecutionId::new(),
                node_key: node_key.clone(),
                action_key: "not-retained".to_owned(),
            });
        }
        shutdown.cancel();

        assert_eq!(
            observe_execution_lifecycle(subscriber, observations, shutdown).await,
            Err(EvidenceIntegrityError::ObservationLagged),
            "shutdown cannot turn already-buffered evidence loss into success"
        );
    }

    #[tokio::test]
    async fn shutdown_drains_pending_lifecycle_fact_without_lag() {
        let bus = EventBus::new(2);
        let subscriber = bus.subscribe();
        let observations = LifecycleObservationRegistry::new();
        let shutdown = CancellationToken::new();
        let execution_id = ExecutionId::new();
        let node_key = NodeKey::new("drain_probe").expect("valid node key");
        let _ = bus.emit(ExecutionEvent::NodeStarted {
            execution_id,
            node_key: node_key.clone(),
            action_key: "not-retained".to_owned(),
        });
        shutdown.cancel();

        assert_eq!(
            observe_execution_lifecycle(subscriber, observations.clone(), shutdown).await,
            Ok(())
        );
        assert_eq!(
            observations
                .await_node_started(execution_id, node_key, Duration::from_secs(1))
                .await,
            Ok(())
        );
    }
}
