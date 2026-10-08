//! Retirement must not depend on optional diagnostic storage.

use std::sync::atomic::AtomicBool;

use nebula_core::{DeclaresDependencies, ResourceId, ResourceKey, WorkspaceId, resource_key};
use nebula_engine::resource::{
    ActivationContext, KindActivator, ResourceActivatorRegistry, StoredResourceActivator,
};
use nebula_resource::{
    AcquireOptions, Manager, Provider, Resident, ResourceContext,
    resource::{ResourceMetadataDraft, TeardownCx},
};
use nebula_storage::inmem::InMemoryResourceStore;
use nebula_storage_port::{
    dto::{LiveResourceStatus, ResourceRow, ResourceStatusSnapshot, StatusWorkerId},
    store::{ResourceStatusStore, ResourceStore},
};
use tokio::sync::Notify;

use super::*;

#[derive(Clone)]
struct RetiredResource(Arc<Notify>);

nebula_resource::no_credential_slots!(RetiredResource);

impl DeclaresDependencies for RetiredResource {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

#[async_trait::async_trait]
impl Provider for RetiredResource {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.worker.retirement")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("Retirement probe"),
            "",
        )
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    async fn destroy(&self, (): (), _: TeardownCx) -> Result<(), nebula_resource::Error> {
        self.0.notify_one();
        Ok(())
    }
}

impl nebula_resource::topology::resident::ResidentProvider for RetiredResource {
    fn is_alive_sync(&self, (): &()) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct HungHeartbeat {
    entered: Notify,
    withdrawn: AtomicU32,
}

#[async_trait::async_trait]
impl ResourceStatusStore for HungHeartbeat {
    async fn heartbeat(&self, _: &StatusWorkerId, _: Duration) -> Result<(), StorageError> {
        self.entered.notify_one();
        std::future::pending().await
    }

    async fn publish(
        &self,
        _: &Scope,
        _: &StatusWorkerId,
        _: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        panic!("a stuck heartbeat cannot reach snapshot publication")
    }

    async fn withdraw(&self, _: &Scope, _: &StatusWorkerId, _: &str) -> Result<(), StorageError> {
        panic!("a stuck heartbeat cannot reach row withdrawal")
    }

    async fn withdraw_worker(&self, _: &StatusWorkerId) -> Result<(), StorageError> {
        self.withdrawn.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn live_for(&self, _: &Scope, _: &str) -> Result<Vec<LiveResourceStatus>, StorageError> {
        panic!("the publisher never reads aggregate status")
    }
}

#[derive(Debug)]
struct MaintenanceStore {
    inner: InMemoryResourceStore,
    stalled: AtomicBool,
    entered: Notify,
}

#[async_trait::async_trait]
impl ResourceStore for MaintenanceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        self.inner.create(scope, row).await
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        if self.stalled.load(Ordering::SeqCst) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        self.inner.get(scope, id).await
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        self.inner.list(scope).await
    }

    async fn update(
        &self,
        scope: &Scope,
        row: ResourceRow,
        version: u64,
    ) -> Result<(), StorageError> {
        self.inner.update(scope, row, version).await
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        self.inner.soft_delete(scope, id).await
    }
}

struct Fixture {
    engine: Arc<WorkflowEngine>,
    store: Arc<MaintenanceStore>,
    scope: Scope,
    resource_id: ResourceId,
    destroyed: Arc<Notify>,
}

impl Fixture {
    async fn new(stores: &TestStores) -> Self {
        let store = Arc::new(MaintenanceStore {
            inner: InMemoryResourceStore::new(),
            stalled: AtomicBool::new(false),
            entered: Notify::new(),
        });
        let scope = Scope::new(
            WorkspaceId::new().to_string(),
            nebula_core::OrgId::new().to_string(),
        );
        let resource_id = ResourceId::new();
        store
            .create(
                &scope,
                ResourceRow {
                    id: resource_id.to_string(),
                    workspace_id: scope.workspace_id.clone(),
                    slug: "retirement-probe".into(),
                    display_name: "Retirement probe".into(),
                    kind: RetiredResource::key().to_string(),
                    config: serde_json::Value::Null,
                    credential_bindings: Default::default(),
                    topology: None,
                    resilience_override: None,
                    created_at: "2026-10-07T00:00:00Z".into(),
                    created_by: "test".into(),
                    version: 0,
                    deleted_at: None,
                },
            )
            .await
            .unwrap();
        let manager = Arc::new(Manager::new());
        let destroyed = Arc::new(Notify::new());
        let provider = RetiredResource(destroyed.clone());
        let mut registrars = ResourceActivatorRegistry::new();
        registrars
            .insert(
                RetiredResource::key().as_str(),
                Arc::new(KindActivator::new(
                    move || provider.clone(),
                    nebula_resource::topology::fixed(|| {
                        Resident::<RetiredResource>::new(Default::default())
                    }),
                )),
            )
            .unwrap();
        let activator = StoredResourceActivator::new(store.clone());
        let activated = activator
            .activate(
                &ActivationContext {
                    registrars: &registrars,
                    manager: &manager,
                    credentials: None,
                    expr_engine: &Default::default(),
                    fanout: None,
                },
                &scope,
                resource_id,
                &RetiredResource::key(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let context = ResourceContext::minimal(
            nebula_resource::context::minimal_scope_for_level(&activated.scope),
            CancellationToken::new(),
        );
        drop(
            manager
                .acquire_for_identity::<RetiredResource>(
                    &context,
                    &AcquireOptions::default(),
                    &activated.slot_identity,
                )
                .await
                .expect("materialize a provider instance before retiring it"),
        );
        let (engine, _) = make_engine(stores).await;
        let engine = Arc::new(
            Arc::try_unwrap(engine)
                .unwrap_or_else(|_| panic!("fixture owns its engine"))
                .with_resource_manager(manager)
                .with_resource_registrars(registrars)
                .with_stored_resources(activator)
                .with_credential_resolver(Arc::new(EmptyDeploymentResolver)),
        );
        Self {
            engine,
            store,
            scope,
            resource_id,
            destroyed,
        }
    }
}

async fn assert_retirement_independent_of_status(with_hung_status: bool) {
    let stores = TestStores::new();
    let fixture = Fixture::new(&stores).await;
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let status = Arc::new(HungHeartbeat::default());
    let mut builder = WorkerRuntimeBuilder::from_wired_engine(
        fixture.engine.clone(),
        stores.execution_stores(),
        proc16(0x95),
    )
    .with_control_queue(queue.clone())
    .with_turn_handoff(stores.turn_handoff())
    .with_turn_recovery(stores.turn_handoff())
    .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]));
    if with_hung_status {
        builder = builder.with_resource_status_store(status.clone());
    }
    let shutdown = CancellationToken::new();
    let handle = builder.build().unwrap().spawn(shutdown.clone());
    tokio::time::timeout(Duration::from_secs(1), queue.first_poll.notified())
        .await
        .unwrap();
    if with_hung_status {
        tokio::time::timeout(Duration::from_secs(1), status.entered.notified())
            .await
            .unwrap();
    }
    fixture
        .store
        .soft_delete(&fixture.scope, &fixture.resource_id.to_string())
        .await
        .unwrap();
    let retired = tokio::time::timeout(Duration::from_secs(30), fixture.destroyed.notified()).await;
    let still_live = !fixture.engine.resource_status_snapshot().live.is_empty();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    retired.expect(
        "deleted resource is physically destroyed before worker shutdown, independently of status",
    );
    assert!(
        !still_live,
        "the deleted row is removed from the live registry"
    );
    assert_eq!(
        status.withdrawn.load(Ordering::SeqCst),
        u32::from(with_hung_status)
    );
}

#[tokio::test(start_paused = true)]
async fn worker_retires_deleted_resources_without_status_publication() {
    assert_retirement_independent_of_status(false).await;
}

#[tokio::test(start_paused = true)]
async fn worker_retires_deleted_resources_while_status_heartbeat_is_stuck() {
    assert_retirement_independent_of_status(true).await;
}

#[tokio::test(start_paused = true)]
async fn worker_shutdown_cancels_an_inflight_resource_sweep() {
    let stores = TestStores::new();
    let fixture = Fixture::new(&stores).await;
    fixture.store.stalled.store(true, Ordering::SeqCst);
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime = reconciliation_runtime(&stores, fixture.engine, queue);
    let shutdown = CancellationToken::new();
    let handle = runtime.spawn(shutdown.clone());
    let entered =
        tokio::time::timeout(Duration::from_secs(30), fixture.store.entered.notified()).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    entered.expect("ordinary worker must start resource maintenance without a publisher");
}
