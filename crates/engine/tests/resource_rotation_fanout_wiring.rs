#![cfg(feature = "rotation")]

//! End-to-end: the production rotation fan-out **wiring** (
//! §Deferred "Rotation fan-out is implemented but unwired"; closes #679 /
//! #680 / #681 prerequisites).
//!
//! Unlike `rotation_resource_fanout.rs` (which calls
//! `ResourceFanoutIndex::dispatch_*` directly), this exercises the REAL
//! engine path: a [`ResourceFanoutDriver`] spawned over real
//! `nebula-eventbus` buses, driven by **emitting the exact events the
//! credential-runtime composition root emits in production**
//! ([`CredentialEvent::Refreshed`] / [`CredentialEvent::Revoked`] on
//! `EventBus<CredentialEvent>`, [`LeaseEvent::LeaseRevoked`] on
//! `EventBus<LeaseEvent>` — the `EventMetricObserver` shape, ).
//! Nothing here calls `dispatch_refresh` / `dispatch_revoke` itself; the
//! driver's bus subscription does, exactly as in production.
//!
//! Wired path under test:
//! `EventBus → ResourceFanoutDriver subscriber → ResourceFanoutIndex
//!  ::dispatch_{refresh,revoke} → Manager::{refresh_slot_for,
//!  taint_slot_for + drain_and_revoke} → resource on_credential_* hook`.

use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialEvent, CredentialId, LeaseEvent};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, WorkflowEngine,
};
use nebula_eventbus::EventBus;
use nebula_metrics::MetricsRegistry;
use nebula_resource::Resident;
use nebula_resource::{
    AcquireOptions, Manager, Provider, RegistrationSpec, ResidentConfig, ResourceConfig,
    ResourceContext, SlotIdentity,
    error::Error as ResourceError,
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use nebula_resource::{ResourceFanoutDriver, ResourceFanoutIndex};
use tokio_util::sync::CancellationToken;

trait TestRotationBind {
    fn bind_test(
        &self,
        credential_id: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        identity: SlotIdentity,
    );
}

impl TestRotationBind for ResourceFanoutIndex {
    fn bind_test(
        &self,
        credential_id: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        identity: SlotIdentity,
    ) {
        self.bind_with_context(
            credential_id,
            nebula_resource::Bind {
                resource_key,
                scope,
                slot_name: slot.to_owned(),
                slot_identity: identity,
            },
            nebula_credential::TenantScope::new("org", "workspace"),
            nebula_core::credential_key!("oauth"),
        );
    }
}
use zeroize::Zeroize;

// ── Test resource recording every rotation/revoke hook delivery ──────

#[derive(Debug)]
struct HookError(&'static str);
impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

#[tokio::test(start_paused = true)]
async fn material_replacement_installs_projected_guard_before_refresh_hook() {
    let cid = CredentialId::new();
    let credential_key = "oauth".parse().expect("valid credential key");
    let scope = ScopeLevel::Global;
    let identity = SlotIdentity::from_bindings([("db", cid.to_string().as_str())]);
    let observed = Arc::new(AtomicUsize::new(0));
    let slot = Arc::new(nebula_resource::SlotCell::empty());
    let manager = Arc::new(Manager::new());
    manager
        .register(RegistrationSpec {
            resource: ReplacementResource {
                slot: Arc::clone(&slot),
                observed: Arc::clone(&observed),
                hooks: Arc::new(AtomicUsize::new(0)),
                stall_hook: false,
            },
            config: NoCfg,
            scope: scope.clone(),
            slot_identity: identity.clone(),
            topology: Resident::<ReplacementResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("replacement resource registers");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let warm = manager
        .acquire_resident_for_identity::<ReplacementResource>(
            &context,
            &AcquireOptions::default(),
            &identity,
        )
        .await
        .expect("replacement resource warms");
    drop(warm);

    let index = Arc::new(ResourceFanoutIndex::new());
    index.bind_test(cid, ReplacementResource::key(), scope, "db", identity);
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver: Arc<dyn nebula_credential::CredentialSlotResolver> =
        Arc::new(ReplacementResolver {
            calls: Arc::clone(&calls),
            epoch: None,
        });
    let bus = Arc::new(EventBus::new(8));
    let _driver = ResourceFanoutDriver::spawn_with_resolver(
        index,
        manager,
        Some(resolver),
        Arc::clone(&bus),
        None,
    );

    for _ in 0..100 {
        if calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    bus.emit(CredentialEvent::MaterialReplaced {
        credential_id: cid,
        scope: nebula_credential::TenantScope::new("org", "workspace"),
        credential_key,
    });

    for _ in 0..2_000 {
        if observed.load(Ordering::SeqCst) == 22 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        observed.load(Ordering::SeqCst),
        22,
        "refresh hook must observe the newly projected material"
    );
    assert_eq!(slot.material_epoch(), Some(2));
}

#[tokio::test(start_paused = true)]
async fn lost_material_event_is_recovered_on_startup_and_periodic_scan() {
    let cid = CredentialId::new();
    let owner = nebula_credential::TenantScope::new("org", "workspace");
    let key = "oauth".parse().expect("key");
    let slot = Arc::new(nebula_resource::SlotCell::empty());
    let observed = Arc::new(AtomicUsize::new(0));
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    let manager = Arc::new(Manager::new());
    let resource = ReplacementResource {
        slot: Arc::clone(&slot),
        observed: Arc::clone(&observed),
        hooks: Arc::new(AtomicUsize::new(0)),
        stall_hook: false,
    };
    let installed = resource
        .install_credential_slot(
            "db",
            nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(11)),
                nebula_credential::CredentialGuardMetadata::new(cid, key, 1, 1).with_scope(owner),
            ),
        )
        .expect("initial guard");
    assert_eq!(installed, nebula_resource::SlotUpdate::Installed);
    manager
        .register(RegistrationSpec {
            resource,
            config: NoCfg,
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::<ReplacementResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("warm"),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(EventBus::new(8));
    let index = Arc::new(ResourceFanoutIndex::new());
    index.bind_test(
        cid,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        identity,
    );
    // No event: the published rotation binding makes the live guard eligible
    // for startup and periodic durable reconciliation.
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        index,
        Arc::clone(&manager),
        Some(Arc::new(ReplacementResolver {
            calls: Arc::clone(&calls),
            epoch: None,
        })),
        Arc::clone(&bus),
        None,
    );
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(slot.material_epoch(), Some(2));
    assert_eq!(observed.load(Ordering::SeqCst), 22);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(31)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        calls.load(Ordering::SeqCst) >= 2,
        "periodic durable reread must run without an event"
    );
    assert_eq!(
        slot.generation(),
        2,
        "same durable epoch must not reinstall"
    );
    driver.abort();
}

#[tokio::test(start_paused = true)]
async fn rotation_opt_out_is_not_reconciled_from_projection_metadata() {
    let manager = Arc::new(Manager::new());
    let credential_id = CredentialId::new();
    let resource = register_replacement(&manager, credential_id);
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(EventBus::new(8));
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        Arc::new(ResourceFanoutIndex::new()),
        manager,
        Some(Arc::new(ReplacementResolver {
            calls: Arc::clone(&calls),
            epoch: None,
        })),
        Arc::clone(&bus),
        None,
    );

    tokio::time::advance(Duration::from_secs(31)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(resource.slot.material_epoch(), Some(1));
    assert_eq!(resource.hooks.load(Ordering::SeqCst), 0);
    driver.abort();
}

#[tokio::test(start_paused = true)]
async fn material_replacement_hook_timeout_is_not_counted_as_success() {
    let manager = Manager::new();
    let cid = CredentialId::new();
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    manager
        .register(RegistrationSpec {
            resource: ReplacementResource {
                slot: Arc::new(nebula_resource::SlotCell::empty()),
                observed: Arc::new(AtomicUsize::new(0)),
                hooks: Arc::new(AtomicUsize::new(0)),
                stall_hook: true,
            },
            config: NoCfg,
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::<ReplacementResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("warm"),
    );
    let index = ResourceFanoutIndex::new();
    index.bind_test(
        cid,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        identity.clone(),
    );
    let resolver = ReplacementResolver {
        calls: Arc::new(AtomicUsize::new(0)),
        epoch: None,
    };
    let outcome = index
        .dispatch_material_replacement(
            cid,
            &nebula_credential::TenantScope::new("org", "workspace"),
            &"oauth".parse().expect("key"),
            &resolver,
            &manager,
        )
        .await;
    assert_eq!(outcome.timed_out(), 1);
    assert_eq!(outcome.success(), 0);
    assert_eq!(outcome.dispatched(), 1);
}

#[tokio::test(start_paused = true)]
async fn refreshed_events_and_scans_install_once_per_epoch_in_either_order() {
    for event_first in [true, false] {
        let manager = Arc::new(Manager::new());
        let cid = CredentialId::new();
        let resource = register_replacement(&manager, cid);
        let identity = SlotIdentity::from_bindings([("db", "oauth")]);
        let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        drop(
            manager
                .acquire_resident_for_identity::<ReplacementResource>(
                    &context,
                    &AcquireOptions::default(),
                    &identity,
                )
                .await
                .expect("warm"),
        );
        let epoch = Arc::new(AtomicUsize::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let bus = Arc::new(EventBus::new(8));
        let index = Arc::new(ResourceFanoutIndex::new());
        index.bind_test(
            cid,
            ReplacementResource::key(),
            ScopeLevel::Global,
            "db",
            identity.clone(),
        );
        let driver = ResourceFanoutDriver::spawn_with_resolver(
            index,
            manager,
            Some(Arc::new(ReplacementResolver {
                calls: Arc::clone(&calls),
                epoch: Some(Arc::clone(&epoch)),
            })),
            Arc::clone(&bus),
            None,
        );
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(resource.hooks.load(Ordering::SeqCst), 0);
        epoch.store(2, Ordering::SeqCst);
        if !event_first {
            tokio::time::advance(Duration::from_secs(31)).await;
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
        }
        bus.emit(CredentialEvent::Refreshed { credential_id: cid });
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        bus.emit(CredentialEvent::Refreshed { credential_id: cid });
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(resource.slot.material_epoch(), Some(2));
        assert_eq!(resource.observed.load(Ordering::SeqCst), 22);
        assert_eq!(resource.hooks.load(Ordering::SeqCst), 1);
        if event_first {
            tokio::time::advance(Duration::from_secs(31)).await;
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
        }
        assert!(calls.load(Ordering::SeqCst) >= 4);
        assert_eq!(resource.hooks.load(Ordering::SeqCst), 1);
        assert_eq!(resource.slot.generation(), 2);
        driver.abort();
    }
}

#[tokio::test]
async fn stalled_refresh_scan_does_not_delay_credential_or_lease_revoke() {
    for material_replaced in [false, true] {
        for via_lease in [false, true] {
            let manager = Arc::new(Manager::new());
            let cid = CredentialId::new();
            let _resource = register_replacement(&manager, cid);
            let identity = SlotIdentity::from_bindings([("db", "oauth")]);
            let index = Arc::new(ResourceFanoutIndex::new());
            index.bind_test(
                cid,
                ReplacementResource::key(),
                ScopeLevel::Global,
                "db",
                identity.clone(),
            );
            let entered = Arc::new(tokio::sync::Semaphore::new(0));
            let release = Arc::new(tokio::sync::Semaphore::new(0));
            let cancelled = Arc::new(std::sync::Mutex::new(None));
            let resolver = GatedProjection {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                cancelled: Arc::clone(&cancelled),
            };
            let bus = Arc::new(EventBus::new(8));
            let leases = Arc::new(EventBus::new(8));
            let mut observations = manager.subscribe_events();
            let driver = ResourceFanoutDriver::spawn_with_resolver(
                index,
                Arc::clone(&manager),
                Some(Arc::new(resolver)),
                Arc::clone(&bus),
                Some(Arc::clone(&leases)),
            );
            tokio::time::timeout(Duration::from_secs(1), entered.acquire())
                .await
                .expect("scan started")
                .expect("permit")
                .forget();
            release.add_permits(1);
            tokio::time::timeout(Duration::from_secs(1), async {
                while !cancelled
                    .lock()
                    .expect("lock")
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("startup scan completes");
            if material_replaced {
                bus.emit(CredentialEvent::MaterialReplaced {
                    credential_id: cid,
                    scope: nebula_credential::TenantScope::new("org", "workspace"),
                    credential_key: "oauth".parse().expect("key"),
                });
            } else {
                bus.emit(CredentialEvent::Refreshed { credential_id: cid });
            }
            tokio::time::timeout(Duration::from_secs(1), entered.acquire())
                .await
                .expect("event-requested scan started")
                .expect("permit")
                .forget();
            if via_lease {
                leases.emit(LeaseEvent::LeaseRevoked {
                    credential_id: Some(cid),
                    lease_id: "fixture".into(),
                    provider: "vault".into(),
                });
            } else {
                bus.emit(CredentialEvent::Revoked { credential_id: cid });
            }
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if matches!(
                        observations.recv().await,
                        Some(nebula_resource::ResourceEvent::SlotRevoked { .. })
                    ) {
                        break;
                    }
                }
            })
            .await
            .expect("revoke must complete before the 30-second projection deadline");
            let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
            assert!(
                manager
                    .acquire_resident_for_identity::<ReplacementResource>(
                        &context,
                        &AcquireOptions::default(),
                        &identity
                    )
                    .await
                    .is_err(),
                "revoked row must reject acquisition"
            );
            driver.abort();
        }
    }
}

struct GatedProjection {
    entered: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
    cancelled: Arc<std::sync::Mutex<Option<CancellationToken>>>,
}

struct ConcurrencyProbeProjection {
    entered: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

struct RevokedProjection {
    calls: Arc<AtomicUsize>,
}

struct GatedRevokedProjection {
    entered: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
}

impl nebula_credential::CredentialSlotResolver for RevokedProjection {
    fn resolve_slot<'a>(
        &'a self,
        _scope: &'a nebula_credential::TenantScope,
        _cid: CredentialId,
        _key: nebula_core::CredentialKey,
        _capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(nebula_credential::CredentialSlotResolveError::Revoked) })
    }
}

impl nebula_credential::CredentialSlotResolver for GatedRevokedProjection {
    fn resolve_slot<'a>(
        &'a self,
        _scope: &'a nebula_credential::TenantScope,
        _cid: CredentialId,
        _key: nebula_core::CredentialKey,
        _capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
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
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.expect("release gate").forget();
            Err(nebula_credential::CredentialSlotResolveError::Revoked)
        })
    }
}

#[tokio::test(start_paused = true)]
async fn durable_tombstone_reconciliation_terminally_revokes_resource() {
    let manager = Arc::new(Manager::new());
    let cid = CredentialId::new();
    register_replacement(&manager, cid);
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("projected resource warms before reconciliation"),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(EventBus::new(8));
    let index = Arc::new(ResourceFanoutIndex::new());
    index.bind_test(
        cid,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        identity.clone(),
    );
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        index,
        Arc::clone(&manager),
        Some(Arc::new(RevokedProjection {
            calls: Arc::clone(&calls),
        })),
        Arc::clone(&bus),
        None,
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .is_err(),
        "startup reconciliation must synchronously taint a tombstoned credential row"
    );
    driver.abort();
}

#[tokio::test(start_paused = true)]
async fn empty_bound_slot_reconciles_a_lost_durable_tombstone() {
    let manager = Arc::new(Manager::new());
    let credential_id = CredentialId::new();
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    let resource = ReplacementResource {
        slot: Arc::new(nebula_resource::SlotCell::empty()),
        observed: Arc::new(AtomicUsize::new(0)),
        hooks: Arc::new(AtomicUsize::new(0)),
        stall_hook: false,
    };
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: NoCfg,
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::<ReplacementResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register empty bound slot");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("empty resource warms"),
    );
    let index = Arc::new(ResourceFanoutIndex::new());
    index.bind_test(
        credential_id,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        identity.clone(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(EventBus::new(8));
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        index,
        Arc::clone(&manager),
        Some(Arc::new(RevokedProjection {
            calls: Arc::clone(&calls),
        })),
        Arc::clone(&bus),
        None,
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        manager
            .acquire_resident_for_identity::<ReplacementResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .is_err(),
        "lost tombstone must taint an initially empty bound slot"
    );
    driver.abort();
}

#[tokio::test(start_paused = true)]
async fn superseded_projection_fences_delayed_tombstone_reconciliation() {
    for replace_with_value in [true, false] {
        let manager = Arc::new(Manager::new());
        let cid = CredentialId::new();
        let resource = register_replacement(&manager, cid);
        let identity = SlotIdentity::from_bindings([("db", "oauth")]);
        let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        drop(
            manager
                .acquire_resident_for_identity::<ReplacementResource>(
                    &context,
                    &AcquireOptions::default(),
                    &identity,
                )
                .await
                .expect("resource warms before reconciliation"),
        );
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let bus = Arc::new(EventBus::new(8));
        let index = Arc::new(ResourceFanoutIndex::new());
        index.bind_test(
            cid,
            ReplacementResource::key(),
            ScopeLevel::Global,
            "db",
            identity.clone(),
        );
        let driver = ResourceFanoutDriver::spawn_with_resolver(
            index,
            Arc::clone(&manager),
            Some(Arc::new(GatedRevokedProjection {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            })),
            Arc::clone(&bus),
            None,
        );
        entered.acquire().await.expect("projection starts").forget();
        if replace_with_value {
            resource
                .slot
                .store(Arc::new(nebula_credential::CredentialGuard::new(
                    ReplacementMaterial(77),
                )));
        } else {
            resource.slot.take();
        }
        release.add_permits(1);
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        drop(
            manager
                .acquire_resident_for_identity::<ReplacementResource>(
                    &context,
                    &AcquireOptions::default(),
                    &identity,
                )
                .await
                .expect("superseded credential projection must not taint the row"),
        );
        driver.abort();
    }
}

impl nebula_credential::CredentialSlotResolver for ConcurrencyProbeProjection {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a nebula_credential::TenantScope,
        cid: CredentialId,
        key: nebula_core::CredentialKey,
        _capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
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
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            self.entered.add_permits(1);
            self.release
                .acquire()
                .await
                .expect("probe release")
                .forget();
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(99)),
                nebula_credential::CredentialGuardMetadata::new(cid, key, 2, 2)
                    .with_scope(scope.clone()),
            ))
        })
    }
}

#[tokio::test]
async fn material_replacement_bounds_projection_concurrency() {
    const CREDENTIALS: usize = 2;
    const ROWS: usize = 40;
    const LIMIT: usize = 32;

    let manager = Arc::new(Manager::new());
    let index = Arc::new(ResourceFanoutIndex::new());
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(ConcurrencyProbeProjection {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
        active: Arc::clone(&active),
        maximum: Arc::clone(&maximum),
    });
    let dispatches = (0..CREDENTIALS)
        .map(|credential| {
            let cid = CredentialId::new();
            for row in 0..ROWS {
                let identity_value = format!("oauth-{credential}-{row}");
                register_replacement_with_identity(&manager, cid, &identity_value);
                index.bind_test(
                    cid,
                    ReplacementResource::key(),
                    ScopeLevel::Global,
                    "db",
                    SlotIdentity::from_bindings([("db", identity_value.as_str())]),
                );
            }
            let index = Arc::clone(&index);
            let manager = Arc::clone(&manager);
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move {
                index
                    .dispatch_material_replacement(
                        cid,
                        &nebula_credential::TenantScope::new("org", "workspace"),
                        &"oauth".parse().expect("key"),
                        resolver.as_ref(),
                        &manager,
                    )
                    .await
            })
        })
        .collect::<Vec<_>>();

    tokio::time::timeout(Duration::from_secs(2), entered.acquire_many(LIMIT as u32))
        .await
        .expect("first bounded batch starts")
        .expect("entered semaphore")
        .forget();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(active.load(Ordering::SeqCst), LIMIT);
    assert_eq!(maximum.load(Ordering::SeqCst), LIMIT);
    assert_eq!(entered.available_permits(), 0, "row 33 must remain queued");

    release.add_permits(CREDENTIALS * ROWS);
    let outcomes = tokio::time::timeout(
        Duration::from_secs(2),
        futures::future::join_all(dispatches),
    )
    .await
    .expect("bounded fan-outs complete");
    assert_eq!(
        outcomes
            .into_iter()
            .map(|outcome| outcome.expect("dispatch task").success())
            .sum::<usize>(),
        CREDENTIALS * ROWS
    );
    assert_eq!(maximum.load(Ordering::SeqCst), LIMIT);
}

impl nebula_credential::CredentialSlotResolver for GatedProjection {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a nebula_credential::TenantScope,
        cid: CredentialId,
        key: nebula_core::CredentialKey,
        _capabilities: nebula_credential::Capabilities,
        cancel: CancellationToken,
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
        Box::pin(async move {
            *self.cancelled.lock().expect("lock") = Some(cancel);
            self.entered.add_permits(1);
            self.release.acquire().await.expect("gate").forget();
            Ok(nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(99)),
                nebula_credential::CredentialGuardMetadata::new(cid, key, 99, 99)
                    .with_scope(scope.clone()),
            ))
        })
    }
}

fn register_replacement(manager: &Manager, cid: CredentialId) -> ReplacementResource {
    register_replacement_with_identity(manager, cid, "oauth")
}

fn register_replacement_with_identity(
    manager: &Manager,
    cid: CredentialId,
    identity: &str,
) -> ReplacementResource {
    register_replacement_with_hook_behavior(manager, cid, identity, false)
}

fn register_replacement_with_hook_behavior(
    manager: &Manager,
    cid: CredentialId,
    identity: &str,
    stall_hook: bool,
) -> ReplacementResource {
    let resource = ReplacementResource {
        slot: Arc::new(nebula_resource::SlotCell::empty()),
        observed: Arc::new(AtomicUsize::new(0)),
        hooks: Arc::new(AtomicUsize::new(0)),
        stall_hook,
    };
    let installed = resource
        .install_credential_slot(
            "db",
            nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(11)),
                nebula_credential::CredentialGuardMetadata::new(
                    cid,
                    "oauth".parse().expect("key"),
                    1,
                    1,
                )
                .with_scope(nebula_credential::TenantScope::new("org", "workspace")),
            ),
        )
        .expect("initial projection");
    assert_eq!(installed, nebula_resource::SlotUpdate::Installed);
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: NoCfg,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::from_bindings([("db", identity)]),
            topology: Resident::<ReplacementResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");
    resource
}

#[tokio::test]
async fn projection_capacity_is_released_before_hook_observation() {
    const ROWS: usize = 40;
    let manager = Arc::new(Manager::new());
    let index = Arc::new(ResourceFanoutIndex::new());
    let cid = CredentialId::new();
    for row in 0..ROWS {
        let identity_value = format!("oauth-stalled-{row}");
        register_replacement_with_hook_behavior(&manager, cid, &identity_value, true);
        index.bind_test(
            cid,
            ReplacementResource::key(),
            ScopeLevel::Global,
            "db",
            SlotIdentity::from_bindings([("db", identity_value.as_str())]),
        );
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let dispatch = tokio::spawn({
        let dispatch_calls = Arc::clone(&calls);
        async move {
            index
                .dispatch_material_replacement(
                    cid,
                    &nebula_credential::TenantScope::new("org", "workspace"),
                    &"oauth".parse().expect("key"),
                    &ReplacementResolver {
                        calls: dispatch_calls,
                        epoch: None,
                    },
                    &manager,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) != ROWS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rows after the projection limit start while earlier hooks remain stalled");
    dispatch.abort();
}

#[tokio::test(start_paused = true)]
async fn material_projection_timeout_cancels_resolver_and_is_not_success() {
    let manager = Manager::new();
    let cid = CredentialId::new();
    let resource = register_replacement(&manager, cid);
    let index = ResourceFanoutIndex::new();
    index.bind_test(
        cid,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        SlotIdentity::from_bindings([("db", "oauth")]),
    );
    let resolver = GatedProjection {
        entered: Arc::new(tokio::sync::Semaphore::new(0)),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
        cancelled: Arc::new(std::sync::Mutex::new(None)),
    };
    let outcome = index
        .dispatch_material_replacement(
            cid,
            &nebula_credential::TenantScope::new("org", "workspace"),
            &"oauth".parse().expect("key"),
            &resolver,
            &manager,
        )
        .await;
    assert_eq!(outcome.timed_out(), 1);
    assert_eq!(outcome.success(), 0);
    assert!(
        resolver
            .cancelled
            .lock()
            .expect("lock")
            .as_ref()
            .expect("token")
            .is_cancelled()
    );
    assert_eq!(resource.slot.material_epoch(), Some(1));
}

#[tokio::test]
async fn delayed_projection_cannot_install_into_rebound_registration() {
    for retirement in 0..3 {
        let manager = Arc::new(Manager::new());
        let cid = CredentialId::new();
        let original = register_replacement(&manager, cid);
        let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        drop(
            manager
                .acquire_resident_for_identity::<ReplacementResource>(
                    &context,
                    &AcquireOptions::default(),
                    &SlotIdentity::from_bindings([("db", "oauth")]),
                )
                .await
                .expect("warm original"),
        );
        let index = Arc::new(ResourceFanoutIndex::new());
        index.bind_test(
            cid,
            ReplacementResource::key(),
            ScopeLevel::Global,
            "db",
            SlotIdentity::from_bindings([("db", "oauth")]),
        );
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let resolver = GatedProjection {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            cancelled: Arc::new(std::sync::Mutex::new(None)),
        };
        let dispatch = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move {
                index
                    .dispatch_material_replacement(
                        cid,
                        &nebula_credential::TenantScope::new("org", "workspace"),
                        &"oauth".parse().expect("key"),
                        &resolver,
                        &manager,
                    )
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), entered.acquire())
            .await
            .expect("started")
            .expect("gate")
            .forget();
        match retirement {
            0 => manager
                .remove(&ReplacementResource::key())
                .expect("remove original"),
            1 => manager
                .remove_for(
                    &ReplacementResource::key(),
                    &ScopeLevel::Global,
                    &SlotIdentity::from_bindings([("db", "oauth")]),
                )
                .expect("remove exact original"),
            _ => {}, // Registration below retires the exact existing row.
        }
        let replacement_cid = CredentialId::new();
        let replacement = register_replacement(&manager, replacement_cid);
        release.add_permits(1);
        let outcome = dispatch.await.expect("dispatch");
        assert_eq!(outcome.failed(), 1);
        assert_eq!(outcome.success(), 0);
        assert_eq!(original.slot.material_epoch(), Some(1));
        assert_eq!(original.hooks.load(Ordering::SeqCst), 0);
        assert_eq!(replacement.slot.material_epoch(), Some(1));
        assert_eq!(
            replacement
                .slot
                .projection_metadata()
                .expect("metadata")
                .credential_id(),
            replacement_cid
        );
        assert_eq!(replacement.observed.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn revoked_slots_are_not_reprojected_on_startup_or_periodic_scans() {
    for terminal_cell in [false, true] {
        let manager = Arc::new(Manager::new());
        let resource = register_replacement(&manager, CredentialId::new());
        if terminal_cell {
            assert_eq!(resource.slot.revoke(), nebula_resource::SlotUpdate::Revoked);
        } else {
            let tainted = manager
                .taint_slot_for_identity(
                    &ReplacementResource::key(),
                    ScopeLevel::Global,
                    "db",
                    &SlotIdentity::from_bindings([("db", "oauth")]),
                )
                .expect("taint");
            drop(tainted);
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let bus = Arc::new(EventBus::new(8));
        let driver = ResourceFanoutDriver::spawn_with_resolver(
            Arc::new(ResourceFanoutIndex::new()),
            manager,
            Some(Arc::new(ReplacementResolver {
                epoch: None,
                calls: Arc::clone(&calls),
            })),
            Arc::clone(&bus),
            None,
        );
        for _ in 0..3 {
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
            tokio::time::advance(Duration::from_secs(30)).await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(resource.hooks.load(Ordering::SeqCst), 0);
        driver.abort();
    }
}

#[tokio::test]
async fn unqualified_writes_fence_in_flight_scans_and_material_events() {
    for via_event in [false, true] {
        for use_store in [false, true] {
            let manager = Arc::new(Manager::new());
            let cid = CredentialId::new();
            let resource = register_replacement(&manager, cid);
            let index = Arc::new(ResourceFanoutIndex::new());
            index.bind_test(
                cid,
                ReplacementResource::key(),
                ScopeLevel::Global,
                "db",
                SlotIdentity::from_bindings([("db", "oauth")]),
            );
            let entered = Arc::new(tokio::sync::Semaphore::new(0));
            let release = Arc::new(tokio::sync::Semaphore::new(0));
            let cancelled = Arc::new(std::sync::Mutex::new(None));
            let resolver = Arc::new(GatedProjection {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                cancelled: Arc::clone(&cancelled),
            });
            let bus = Arc::new(EventBus::new(8));
            let mut dispatch = None;
            let mut driver = None;
            if via_event {
                let manager = Arc::clone(&manager);
                dispatch = Some(tokio::spawn(async move {
                    index
                        .dispatch_material_replacement(
                            cid,
                            &nebula_credential::TenantScope::new("org", "workspace"),
                            &"oauth".parse().expect("key"),
                            resolver.as_ref(),
                            &manager,
                        )
                        .await
                }));
            } else {
                driver = Some(ResourceFanoutDriver::spawn_with_resolver(
                    index,
                    Arc::clone(&manager),
                    Some(resolver),
                    Arc::clone(&bus),
                    None,
                ));
            }
            tokio::time::timeout(Duration::from_secs(2), entered.acquire())
                .await
                .expect("projection started")
                .expect("permit")
                .forget();
            if use_store {
                resource
                    .slot
                    .store(Arc::new(nebula_credential::CredentialGuard::new(
                        ReplacementMaterial(123),
                    )));
            } else {
                assert!(resource.slot.take().is_some());
            }
            let generation = resource.slot.generation();
            release.add_permits(1);
            if let Some(dispatch) = dispatch {
                let outcome = dispatch.await.expect("dispatch");
                assert_eq!(outcome.failed(), 1);
                assert_eq!(outcome.success(), 0);
            } else {
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        if cancelled
                            .lock()
                            .expect("lock")
                            .as_ref()
                            .is_some_and(CancellationToken::is_cancelled)
                        {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("scan settled");
            }
            assert_eq!(resource.slot.generation(), generation);
            assert_eq!(
                resource.slot.load().map(|guard| guard.0),
                use_store.then_some(123)
            );
            assert!(resource.slot.projection_metadata().is_none());
            assert_eq!(resource.hooks.load(Ordering::SeqCst), 0);
            if let Some(driver) = driver {
                driver.abort();
            }
        }
    }
}

#[tokio::test]
async fn material_event_does_not_restore_a_superseded_projection() {
    for use_store in [false, true] {
        let manager = Manager::new();
        let cid = CredentialId::new();
        let resource = register_replacement(&manager, cid);
        let index = ResourceFanoutIndex::new();
        index.bind_test(
            cid,
            ReplacementResource::key(),
            ScopeLevel::Global,
            "db",
            SlotIdentity::from_bindings([("db", "oauth")]),
        );
        if use_store {
            resource
                .slot
                .store(Arc::new(nebula_credential::CredentialGuard::new(
                    ReplacementMaterial(123),
                )));
        } else {
            assert!(resource.slot.take().is_some());
        }
        let generation = resource.slot.generation();
        let calls = Arc::new(AtomicUsize::new(0));
        let outcome = index
            .dispatch_material_replacement(
                cid,
                &nebula_credential::TenantScope::new("org", "workspace"),
                &"oauth".parse().expect("key"),
                &ReplacementResolver {
                    epoch: None,
                    calls: Arc::clone(&calls),
                },
                &manager,
            )
            .await;

        assert_eq!(outcome.failed(), 1);
        assert_eq!(outcome.success(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(resource.slot.generation(), generation);
        assert_eq!(
            resource.slot.load().map(|guard| guard.0),
            use_store.then_some(123)
        );
        assert!(resource.slot.projection_metadata().is_none());
        assert_eq!(resource.hooks.load(Ordering::SeqCst), 0);
    }
}

#[derive(Zeroize)]
struct ReplacementMaterial(u64);

#[derive(Clone)]
struct ReplacementResource {
    stall_hook: bool,
    slot: Arc<nebula_resource::SlotCell<nebula_credential::CredentialGuard<ReplacementMaterial>>>,
    observed: Arc<AtomicUsize>,
    hooks: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ReplacementResource {
    type Config = NoCfg;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("fanout-replacement-order")
    }

    async fn create(&self, _c: &NoCfg, _x: &ResourceContext) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn on_credential_refresh(&self, _slot: &str, _runtime: &()) -> Result<(), ResourceError> {
        if self.stall_hook {
            std::future::pending::<()>().await;
        }
        let value = self
            .slot
            .load()
            .expect("replacement guard installed before hook");
        self.observed.store(value.0 as usize, Ordering::SeqCst);
        self.hooks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("fanout-replacement-order"),
            "",
        )
    }
}

impl HasCredentialSlots for ReplacementResource {
    fn credential_slot_metadata(
        &self,
        slot: &str,
    ) -> Option<nebula_credential::CredentialGuardMetadata> {
        (slot == "db")
            .then(|| self.slot.projection_metadata())
            .flatten()
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<nebula_credential::CredentialGuardMetadata>)> {
        (slot == "db").then(|| self.slot.projection_snapshot())
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        if slot != "db" {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<ReplacementMaterial>()
            .map_err(|_| nebula_resource::SlotInstallError::CredentialTypeMismatch)?;
        self.slot
            .install_projected_at_generation(expected_generation, metadata, Arc::new(guard))
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), nebula_resource::SlotInstallError> {
        if slot != "db" {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        self.slot
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn credential_slot_epoch(&self) -> u64 {
        self.slot.generation()
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }

    fn install_credential_slot(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        if slot != "db" {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<ReplacementMaterial>()
            .map_err(|_| nebula_resource::SlotInstallError::CredentialTypeMismatch)?;
        self.slot.install_projected(metadata, Arc::new(guard))
    }
}

#[async_trait::async_trait]
impl ResidentProvider for ReplacementResource {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

struct ReplacementResolver {
    epoch: Option<Arc<AtomicUsize>>,
    calls: Arc<AtomicUsize>,
}

struct TwoCredentialProjection {
    first: CredentialId,
    first_calls: Arc<AtomicUsize>,
    second_calls: Arc<AtomicUsize>,
}

impl nebula_credential::CredentialSlotResolver for ReplacementResolver {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a nebula_credential::TenantScope,
        credential_id: CredentialId,
        expected_key: nebula_core::CredentialKey,
        _required_capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        let epoch = self
            .epoch
            .as_ref()
            .map_or(2, |epoch| epoch.load(Ordering::SeqCst)) as u64;
        let metadata = nebula_credential::CredentialGuardMetadata::new(
            credential_id,
            expected_key,
            epoch,
            epoch,
        )
        .with_scope(scope.clone());
        Box::pin(async move {
            Ok(nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(22)),
                metadata,
            ))
        })
    }
}

impl nebula_credential::CredentialSlotResolver for TwoCredentialProjection {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a nebula_credential::TenantScope,
        credential_id: CredentialId,
        expected_key: nebula_core::CredentialKey,
        _required_capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
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
        if credential_id == self.first {
            self.first_calls.fetch_add(1, Ordering::SeqCst);
        } else {
            self.second_calls.fetch_add(1, Ordering::SeqCst);
        }
        let metadata =
            nebula_credential::CredentialGuardMetadata::new(credential_id, expected_key, 2, 2)
                .with_scope(scope.clone());
        Box::pin(async move {
            Ok(nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(ReplacementMaterial(22)),
                metadata,
            ))
        })
    }
}

#[tokio::test(start_paused = true)]
async fn refreshed_event_scans_only_the_observed_credential() {
    let manager = Arc::new(Manager::new());
    let first = CredentialId::new();
    let second = CredentialId::new();
    let first_resource = register_replacement_with_identity(&manager, first, "oauth-first");
    let second_resource = register_replacement_with_identity(&manager, second, "oauth-second");
    let index = Arc::new(ResourceFanoutIndex::new());
    index.bind_test(
        first,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        SlotIdentity::from_bindings([("db", "oauth-first")]),
    );
    index.bind_test(
        second,
        ReplacementResource::key(),
        ScopeLevel::Global,
        "db",
        SlotIdentity::from_bindings([("db", "oauth-second")]),
    );
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(EventBus::new(8));
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        index,
        manager,
        Some(Arc::new(TwoCredentialProjection {
            first,
            first_calls: Arc::clone(&first_calls),
            second_calls: Arc::clone(&second_calls),
        })),
        Arc::clone(&bus),
        None,
    );
    for _ in 0..100 {
        if first_resource.observed.load(Ordering::SeqCst) == 22
            && second_resource.observed.load(Ordering::SeqCst) == 22
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    first_calls.store(0, Ordering::SeqCst);
    second_calls.store(0, Ordering::SeqCst);
    bus.emit(CredentialEvent::Refreshed {
        credential_id: first,
    });
    for _ in 0..100 {
        if first_calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 0);
    driver.abort();
}
impl std::error::Error for HookError {}
impl From<HookError> for ResourceError {
    fn from(e: HookError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Hook returns `Ok` immediately.
    Ok,
    /// Hook never completes — models a wedged resource so the
    /// per-resource timeout must fire (`timed_out`) without ever
    /// un-tainting the row (the #681 invariant, end-to-end).
    Hang,
}

#[derive(Clone, Default)]
struct Recorder {
    refresh: Arc<AtomicUsize>,
    revoke: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct Recording {
    behaviour: Behaviour,
    rec: Recorder,
}

#[async_trait::async_trait]
impl Provider for Recording {
    type Config = NoCfg;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("fanout-wiring-rec")
    }

    async fn create(&self, _c: &NoCfg, _x: &ResourceContext) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn on_credential_refresh(&self, _s: &str, _r: &()) -> Result<(), ResourceError> {
        self.rec.refresh.fetch_add(1, Ordering::SeqCst);
        drive(self.behaviour).await
    }

    async fn on_credential_revoke(&self, _s: &str, _r: &()) -> Result<(), ResourceError> {
        self.rec.revoke.fetch_add(1, Ordering::SeqCst);
        drive(self.behaviour).await
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("fanout-wiring-rec"),
            "",
        )
    }
}

// A real declared "db" credential slot — every scenario in this file wires
// the production rotation fan-out over `SlotIdentity::from_bindings([("db",
// ..)])` rows and drives `on_credential_refresh`/`on_credential_revoke`
// against them, so `no_credential_slots!` would misrepresent this fixture as
// slot-less and (fail-closed) reject every one of those dispatches.
impl HasCredentialSlots for Recording {
    fn credential_slot_epoch(&self) -> u64 {
        // This file's wiring is proven via the `Recorder` hook-entry
        // counters, not the epoch fold.
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }
}

#[async_trait::async_trait]
impl ResidentProvider for Recording {
    fn is_alive_sync(&self, _r: &()) -> bool {
        true
    }
}

async fn drive(b: Behaviour) -> Result<(), ResourceError> {
    match b {
        Behaviour::Ok => Ok(()),
        Behaviour::Hang => {
            std::future::pending::<()>().await;
            // guard-justified: `std::future::pending()` never resolves,
            // so this line is statically unreachable (the wedged arm).
            unreachable!()
        },
    }
}

#[derive(Clone, nebula_schema::Schema)]
struct NoCfg;
impl ResourceConfig for NoCfg {
    fn validate(&self) -> Result<(), ResourceError> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

// ── Harness ─────────────────────────────────────────────────────────

/// Spin up a real `Manager` + bound resident row under one
/// `(key, scope)` keyed by `slot_identity == cid.bits`, a real
/// `EventBus<CredentialEvent>` + `EventBus<LeaseEvent>`, and the
/// production [`ResourceFanoutDriver`] spawned over them. Returns the
/// pieces a test drives the wired path with.
struct Wired {
    cred_bus: Arc<EventBus<CredentialEvent>>,
    lease_bus: Arc<EventBus<LeaseEvent>>,
    mgr: Arc<Manager>,
    index: Arc<ResourceFanoutIndex>,
    cid: CredentialId,
    org: OrgId,
    slot_identity: SlotIdentity,
    rec: Recorder,
    // Held: dropping aborts the driver task.
    _driver: ResourceFanoutDriver,
}

async fn wire(behaviour: Behaviour) -> Wired {
    let rec = Recorder::default();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Arc::new(Manager::new());
    let index = Arc::new(ResourceFanoutIndex::new());
    let cid = CredentialId::new();
    // The collision-free structural identity of the single resolved row,
    // derived from the same `(slot, credential)` binding the resource
    // would resolve — used at register, acquire, and bind.
    let slot_identity = SlotIdentity::from_bindings([("db", "wired-cred")]);

    mgr.register(RegistrationSpec {
        resource: Recording {
            behaviour,
            rec: rec.clone(),
        },
        config: NoCfg,
        scope: scope.clone(),
        slot_identity: slot_identity.clone(),
        topology: Resident::<Recording>::new(ResidentConfig::default()),
        recovery_gate: None,
    })
    .expect("register resolved-credential row");

    // Resident materializes its shared runtime lazily on first acquire —
    // warm it so the rotation/revoke hook has a live `&Runtime`.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let _g = mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &slot_identity,
        )
        .await
        .expect("warm resident runtime");
    drop(_g);

    // The bind seam: this is what the resource-activation path records
    // when a credential resolves into a `#[credential]` slot. We bind
    // directly here (the production registrar bind path is covered by
    // the registrar unit tests) so this test isolates the *driver*
    // wiring: bus event → driver → fan-out → hook.
    index.bind_test(
        cid,
        Recording::key(),
        scope.clone(),
        "db",
        slot_identity.clone(),
    );

    let cred_bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let lease_bus = Arc::new(EventBus::<LeaseEvent>::new(16));
    let driver = ResourceFanoutDriver::spawn(
        Arc::clone(&index),
        Arc::clone(&mgr),
        Arc::clone(&cred_bus),
        Some(Arc::clone(&lease_bus)),
    );

    Wired {
        cred_bus,
        lease_bus,
        mgr,
        index,
        cid,
        org,
        slot_identity,
        rec,
        _driver: driver,
    }
}

/// Poll `cond` up to ~2s (yielding) — the driver runs on its own task,
/// so a bus emission is observed asynchronously. Fails loudly on
/// timeout rather than hanging the runner.
async fn eventually(label: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("condition `{label}` not reached within ~2s — driver wiring did not fire");
}

// ── Tests ───────────────────────────────────────────────────────────

/// A `CredentialEvent::Refreshed` on the credential bus (exactly what
/// `EventMetricObserver::on_refresh` emits after a refresh CAS-persists
/// fresh material) must drive `dispatch_refresh` through the spawned
/// driver and deliver `on_credential_refresh` to the bound resource.
#[tokio::test]
async fn refreshed_event_drives_fanout_to_resource_hook() {
    let w = wire(Behaviour::Ok).await;

    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });

    eventually("refresh hook delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "a Refreshed event must not drive the revoke hook"
    );
}

/// A `CredentialEvent::Revoked` (the facade-level revoke signal)
/// must drive `dispatch_revoke` → taint → drain → `on_credential_revoke`.
#[tokio::test]
async fn credential_revoked_event_drives_revoke_fanout() {
    let w = wire(Behaviour::Ok).await;

    w.cred_bus.emit(CredentialEvent::Revoked {
        credential_id: w.cid,
    });

    eventually("revoke hook delivered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;
}

/// A `LeaseEvent::LeaseRevoked` carrying an attributed `credential_id`
/// (what the lease scheduler emits via `EventBus<LeaseEvent>` after
/// `LeaseLifecycle::revoke_for_credential`) must drive the revoke
/// fan-out: the row is tainted (a subsequent acquire on that exact
/// resolved row is rejected) and `on_credential_revoke` is delivered —
/// the → path end-to-end.
#[tokio::test]
async fn lease_revoked_event_taints_row_and_delivers_revoke_hook() {
    use nebula_error::{Classify, ErrorCategory};

    let w = wire(Behaviour::Ok).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-xyz".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    eventually("revoke hook delivered via lease bus", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // The decisive cross-ADR assertion: the revoke fan-out tainted the
    // resolved row, so a fresh acquire on it is now rejected.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(w.org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let acquired = w
        .mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &w.slot_identity,
        )
        .await;
    let err = match acquired {
        Err(e) => e,
        Ok(_) => unreachable!(
            // guard-justified: a live guard here means the lease-revoke
            // fan-out did not taint the row — the exact wiring
            // regression this test exists to catch; fail loudly.
            "acquire after a LeaseRevoked-driven revoke must be rejected (row tainted)"
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "post-revoke acquire must be the Revoked/Unavailable taint rejection, got: {err}"
    );
}

/// #681 end-to-end through the wired path: a `LeaseRevoked` whose
/// resource revoke hook **hangs** must record `timed_out` inside the
/// fan-out yet still leave the row tainted (the synchronous
/// `taint_slot_for` ran outside the per-resource timeout). Proven via
/// the wired driver, not a direct `dispatch_revoke`.
#[tokio::test]
async fn lease_revoked_with_hung_hook_still_taints_row() {
    use nebula_error::{Classify, ErrorCategory};

    let w = wire(Behaviour::Hang).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-hang".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // The hung hook is entered (phase 2 reached it) — proof the revoke
    // fan-out ran through the wired driver even though it will time out.
    eventually("hung revoke hook entered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // Even while the drain tail is still timing out, the synchronous
    // phase-1 taint already revoked the row: a fresh acquire is
    // rejected. The hook having been entered above proves phase 2
    // started, which means phase 1 (the synchronous taint) already
    // completed — so the row is tainted *now*. One acquire attempt,
    // bounded so a wiring regression (taint not applied ⇒ the resident
    // acquire would otherwise succeed) fails loudly instead of hanging.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(w.org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let acquired = tokio::time::timeout(
        Duration::from_secs(2),
        w.mgr.acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &w.slot_identity,
        ),
    )
    .await
    .expect("acquire on a tainted row must resolve immediately (rejected), not hang");
    let err = match acquired {
        Err(e) => e,
        Ok(_) => unreachable!(
            // guard-justified: a live guard means the synchronous
            // phase-1 taint did not stick across the hung phase-2 —
            // the exact #681 wiring regression; fail loudly.
            "acquire during a hung revoke must be rejected — phase-1 taint \
             ran synchronously before the timeout (#681)"
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "hung-revoke acquire must hit the Revoked/Unavailable taint, got: {err}"
    );
}

/// Orphan lease (`credential_id == None`) cannot address a reverse-index
/// row — the driver must treat it as a no-op fan-out, never an error,
/// and never touch the bound resource.
#[tokio::test]
async fn orphan_lease_revoked_is_noop() {
    let w = wire(Behaviour::Ok).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: None,
        lease_id: "orphan".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // Drive a real refresh afterwards so we can prove the driver is
    // alive and processing — and that the orphan revoke did NOT deliver
    // a revoke hook.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("post-orphan refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "an orphan LeaseRevoked (no credential id) must not deliver any revoke hook"
    );
}

/// After the bound resource is removed from the manager, a subsequent
/// rotation for that credential fans to zero rows: no stale `Bind`, no
/// bogus `failed`. Proven through the wired driver (emit `Refreshed`
/// post-remove; the resource hook must NOT be delivered and nothing
/// errors).
#[tokio::test]
async fn rotation_after_resource_removed_fans_to_zero_rows() {
    let w = wire(Behaviour::Ok).await;

    // First refresh: delivered (sanity that wiring is live).
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("pre-remove refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;

    // Remove the resource from the manager.
    w.mgr.remove(&Recording::key()).expect("resource removed");
    assert!(
        w.index.affected(&w.cid).is_empty(),
        "manager removal must synchronously prune the attached reverse index"
    );

    // Second refresh after removal. The attached reverse index no longer
    // contains the retired row, so the fan-out is an empty no-op.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });

    // Give the driver ample time to process the second event.
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(
        w.rec.refresh.load(Ordering::SeqCst),
        1,
        "after the resource was removed, a rotation must not deliver its hook again"
    );
}

/// #690 review (P1, comment 3255603311 / 3255606374) — one logical
/// credential revoke double-emits across the two buses
/// (`LeaseEvent::LeaseRevoked` then the facade `CredentialEvent::Revoked`,
/// back-to-back inside one `CredentialService::revoke`). The driver's
/// per-credential dedupe window must collapse them so
/// `on_credential_revoke` fans out **exactly once** per logical revoke
/// (non-idempotent hooks must not double-fire).
#[tokio::test]
async fn duplicate_revoke_across_both_buses_fans_out_once() {
    let w = wire(Behaviour::Ok).await;

    // Lease bus first (the lease scheduler emits LeaseRevoked for the
    // released lease) ...
    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-dup".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });
    // ... then the facade emits CredentialEvent::Revoked for the SAME
    // credential — the second surfacing of one logical revoke.
    w.cred_bus.emit(CredentialEvent::Revoked {
        credential_id: w.cid,
    });

    // The first revoke is delivered.
    eventually("first revoke hook delivered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // A rejected refresh is an observable barrier behind the duplicate on
    // the credential bus. Revoke is terminal, so this must not invoke a hook.
    let mut events = w.mgr.subscribe_events();
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(event) = events.recv().await {
            if let nebula_resource::ResourceEvent::SlotRefreshFailed { kind, .. } = event {
                assert_eq!(kind, nebula_resource::ErrorKind::Revoked);
                return;
            }
        }
        panic!("event bus closed before refresh rejection");
    })
    .await
    .expect("post-duplicate refresh rejection observed");
    assert_eq!(w.rec.refresh.load(Ordering::SeqCst), 0);
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        1,
        "the lease-bus + credential-bus double-emission of ONE logical \
         revoke must fan out the revoke hook exactly once (deduped)"
    );
}

/// #690 review (CodeRabbit nitpick, fix J) — a `LeaseRevoked` carrying a
/// `credential_id` that was **never bound** (a real credential id, but no
/// reverse-index row for it) exercises the "lookup succeeds, zero binds"
/// branch — distinct from the "no credential id" orphan branch
/// ([`orphan_lease_revoked_is_noop`]). Processing must continue and the
/// revoke hook count must stay 0 (nothing bound that credential).
#[tokio::test]
async fn lease_revoked_for_never_bound_credential_is_zero_binds_noop() {
    let w = wire(Behaviour::Ok).await;

    // A real, attributed credential id — but one that no resource row
    // ever bound (distinct from `w.cid`, which is the bound one). The
    // reverse-index lookup succeeds yet yields zero rows.
    let never_bound = CredentialId::new();
    assert_ne!(never_bound, w.cid, "must be a different credential id");
    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(never_bound),
        lease_id: "lease-unbound".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // Prove the driver is still alive and processing after the
    // zero-binds revoke: a real refresh for the BOUND credential is
    // still delivered.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("post-zero-binds refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "a LeaseRevoked for a never-bound credential must fan to zero rows \
         — no revoke hook delivered (lookup-succeeds-zero-binds branch)"
    );
}

// ── Fix D: idempotent engine driver spawn ───────────────────────────

/// No-op action executor — the engine under test never dispatches a
/// workflow; it only exercises `spawn_resource_rotation_fanout`.
fn noop_engine_with_manager(manager: Arc<Manager>) -> WorkflowEngine {
    let registry = Arc::new(ActionRegistry::new());
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("ActionRuntime::try_new"),
    );
    WorkflowEngine::new(runtime, metrics)
        .expect("WorkflowEngine::new")
        .with_resource_manager(manager)
}

/// #690 review (Major, comment 3255607651) —
/// `WorkflowEngine::spawn_resource_rotation_fanout` must be **single-shot**:
/// a second call must NOT spawn a second subscriber pair (which would
/// double-dispatch every refresh/revoke). The second call returns `None`,
/// and a single emitted refresh is delivered to the bound resource hook
/// **exactly once** (a second subscriber would deliver it twice).
#[tokio::test]
async fn engine_spawn_resource_rotation_fanout_is_idempotent() {
    let rec = Recorder::default();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Arc::new(Manager::new());
    let cid = CredentialId::new();
    let slot_identity = SlotIdentity::from_bindings([("db", "wired-cred-2")]);

    mgr.register(RegistrationSpec {
        resource: Recording {
            behaviour: Behaviour::Ok,
            rec: rec.clone(),
        },
        config: NoCfg,
        scope: scope.clone(),
        slot_identity: slot_identity.clone(),
        topology: Resident::<Recording>::new(ResidentConfig::default()),
        recovery_gate: None,
    })
    .expect("register resolved-credential row");

    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let _g = mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &slot_identity,
        )
        .await
        .expect("warm resident runtime");
    drop(_g);

    let engine = noop_engine_with_manager(Arc::clone(&mgr));
    // Bind the resolved row into the engine-owned reverse index so a
    // rotation has a row to fan to.
    engine.resource_fanout_index().bind(
        cid,
        Recording::key(),
        scope.clone(),
        "db",
        slot_identity.clone(),
    );

    let cred_bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let lease_bus = Arc::new(EventBus::<LeaseEvent>::new(16));

    // First spawn: succeeds.
    let _driver = engine
        .spawn_resource_rotation_fanout(Arc::clone(&cred_bus), Some(Arc::clone(&lease_bus)))
        .expect("first spawn must return a driver");

    // Second spawn: idempotent — must NOT spawn a second subscriber.
    let second =
        engine.spawn_resource_rotation_fanout(Arc::clone(&cred_bus), Some(Arc::clone(&lease_bus)));
    assert!(
        second.is_none(),
        "a second spawn_resource_rotation_fanout must return None (driver \
         already running) — no second subscriber"
    );

    // One refresh emitted. With a single subscriber the bound resource
    // hook fires exactly once; a leaked second subscriber would fire it
    // twice.
    cred_bus.emit(CredentialEvent::Refreshed { credential_id: cid });

    // Wait for the hook, then drain extra scheduler turns and assert the
    // count never exceeds 1 (the double-subscribe regression).
    for _ in 0..2000 {
        if rec.refresh.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(
        rec.refresh.load(Ordering::SeqCst),
        1,
        "exactly one refresh-hook delivery — a second spawn must not have \
         created a second subscriber that double-dispatches"
    );
}
