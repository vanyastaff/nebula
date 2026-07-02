//! Scope-aware lookup, multi-resource coexistence, and pool concurrency
//! integration tests for nebula-resource v2: `ScopeLevel` exact-match vs
//! global-fallback vs mismatch resolution, two independently registered
//! resources coexisting on one `Manager`, and pool admission under
//! concurrent acquire load (max-size enforcement, backpressure).
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

use std::sync::atomic::Ordering;

use common::{PoolTestResource, ResidentTestResource, register_pool, test_config, test_ctx};
use nebula_core::{ExecutionId, resource_key};
use nebula_resource::{
    AcquireOptions, Manager, Pooled, RegistrationSpec, Resident, ResidentConfig, ResourceContext,
    ScopeLevel, ShutdownConfig, SlotIdentity, TopologyTag, error::ErrorKind, guard::ResourceGuard,
};

// ---------------------------------------------------------------------------
// Pool concurrency scenarios
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_concurrent_acquire_respects_max_size() {
    let resource = PoolTestResource::new();
    let max_size = 3;
    let config = nebula_resource::topology::pooled::config::Config {
        max_size,
        create_timeout: std::time::Duration::from_millis(200),
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire max_size handles concurrently — all should succeed.
    let mut handles = Vec::new();
    for _ in 0..max_size {
        let handle = mgr
            .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire within max_size should succeed");
        handles.push(handle);
    }
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        u64::from(max_size),
    );

    // One more acquire should time out (pool full, short timeout via deadline).
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(100));
    let result = mgr.acquire_pooled::<PoolTestResource>(&ctx, &opts).await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(handles);
}

#[tokio::test]
async fn pool_backpressure_when_full() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        create_timeout: std::time::Duration::from_millis(200),
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire the single slot.
    let _held = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");

    // Short deadline — should get backpressure quickly.
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(50));
    let result = mgr.acquire_pooled::<PoolTestResource>(&ctx, &opts).await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(_held);
}

// ---------------------------------------------------------------------------
// Scope-aware lookup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_scope_exact_match() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let org_id = nebula_core::OrgId::new();
    let scope = ScopeLevel::Organization(org_id);
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: scope.clone(),
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // Acquire with the same org scope.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org_id),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire with matching scope should succeed");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn manager_scope_fallback_to_global() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    // Register at Global scope.
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // Acquire with Organization scope — should fall back to Global.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(nebula_core::OrgId::new()),
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should fall back to Global");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn manager_scope_mismatch_not_found() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    // Register at Organization(org_id) — no Global fallback.
    let org_id = nebula_core::OrgId::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Organization(org_id),
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // Acquire with a different org scope — no match, no Global fallback.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(nebula_core::OrgId::new()),
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let result = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected NotFound error for mismatched scope"),
    };
    assert_eq!(*err.kind(), ErrorKind::NotFound);
}

// ---------------------------------------------------------------------------
// Multiple resources coexist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_multiple_resources_coexist() {
    let manager = Manager::new();

    // Register a pool resource.
    let pool_resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: pool_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("pool registration should succeed");

    // Register a resident resource.
    let resident_resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resident_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("resident registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));
    assert!(manager.contains(&resource_key!("test-resident")));
    assert_eq!(manager.keys().len(), 2);

    // Acquire each independently.
    let ctx = test_ctx();
    let pool_handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire should succeed");

    let resident_handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("resident acquire should succeed");

    assert_eq!(pool_handle.topology_tag(), TopologyTag::Pool);
    assert_eq!(resident_handle.topology_tag(), TopologyTag::Resident);
    assert_eq!(pool_resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(resident_resource.create_counter.load(Ordering::Relaxed), 1);

    drop(pool_handle);
    drop(resident_handle);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}
