//! `Manager::acquire_erased` exercises the registry-stored `ErasedAcquireFn` hook.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_core::{ExecutionId, OrgId, ResourceKey, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, ResourceContext, ScopeLevel,
    dedup::SLOT_IDENTITY_UNBOUND,
    error::Error,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::{self, Resident},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct ProbeError(String);

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProbeError {}

impl From<ProbeError> for Error {
    fn from(e: ProbeError) -> Self {
        Error::permanent(e.0)
    }
}

#[derive(Clone, Debug, Default)]
struct ProbeConfig;

nebula_schema::impl_empty_has_schema!(ProbeConfig);

impl ResourceConfig for ProbeConfig {}

#[derive(Clone)]
struct ProbeResource {
    create_count: Arc<AtomicU64>,
}

impl ProbeResource {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for ProbeResource {
    type Config = ProbeConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = ProbeError;

    fn key() -> ResourceKey {
        nebula_core::resource_key!("test.acquire_erased.probe")
    }

    fn create(
        &self,
        _config: &ProbeConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, ProbeError>> + Send {
        let counter = self.create_count.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(counter)
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for ProbeResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

#[tokio::test]
async fn acquire_erased_returns_guard_and_runs_create_once() {
    let manager = Arc::new(Manager::new());
    let resource = ProbeResource::new();
    let create_count = Arc::clone(&resource.create_count);

    manager
        .register(
            resource,
            ProbeConfig,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            None,
            None,
        )
        .expect("register");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let key = ProbeResource::key();
    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
    )
    .await
    .expect("acquire_erased");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast to ResourceGuard");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
    assert_eq!(create_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn acquire_erased_finds_org_scoped_row_from_execution_scope_bag() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let resource = ProbeResource::new();

    manager
        .register(
            resource,
            ProbeConfig,
            ScopeLevel::Organization(org),
            TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            None,
            None,
        )
        .expect("register at org scope");

    let key = ProbeResource::key();
    assert!(
        manager.has_registered_for(&key, &ScopeLevel::Organization(org), SLOT_IDENTITY_UNBOUND),
        "row must exist at org scope before acquire"
    );

    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        org_id: Some(org),
        ..Default::default()
    };
    assert!(
        manager.has_registered_for_scope(&key, &scope, SLOT_IDENTITY_UNBOUND),
        "scope-chain lookup must find org row"
    );

    let ctx = ResourceContext::minimal(scope, CancellationToken::new());

    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &ProbeResource::key(),
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
    )
    .await
    .expect("execution-scoped acquire must reach org-scoped row");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast to ResourceGuard from org-scoped row");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
}

/// Global + Organization rows at the same `slot_identity` must not let typed
/// acquire bind Global while walking an execution+org scope bag.
#[tokio::test]
async fn acquire_erased_and_typed_pick_org_not_global_fallback() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let global_resource = ProbeResource::new();
    let global_count = Arc::clone(&global_resource.create_count);
    let org_resource = ProbeResource::new();
    let org_count = Arc::clone(&org_resource.create_count);

    manager
        .register(
            global_resource,
            ProbeConfig,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            None,
            None,
        )
        .expect("register global row");

    manager
        .register(
            org_resource,
            ProbeConfig,
            ScopeLevel::Organization(org),
            TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            None,
            None,
        )
        .expect("register org row");

    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        org_id: Some(org),
        ..Default::default()
    };
    let ctx = ResourceContext::minimal(scope, CancellationToken::new());
    let key = ProbeResource::key();

    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
    )
    .await
    .expect("acquire_erased");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
    assert_eq!(org_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        global_count.load(Ordering::Relaxed),
        0,
        "must not create via Global row when org row matches scope bag"
    );

    let guard = manager
        .acquire_resident_for::<ProbeResource>(
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
        )
        .await
        .expect("typed acquire_resident_for");
    assert_eq!(guard.load(Ordering::Relaxed), 1);
    assert_eq!(
        org_count.load(Ordering::Relaxed),
        1,
        "typed acquire must reuse the org row without a second create"
    );
    assert_eq!(global_count.load(Ordering::Relaxed), 0);
}
