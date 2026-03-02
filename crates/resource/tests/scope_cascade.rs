//! T047: Parent scope shutdown cascades to child-scoped resources.
//!
//! Verifies:
//! 1. Resources registered at different scopes (tenant, workflow, execution)
//!    are cleaned up when the parent scope is shut down.
//! 2. Resources under a different parent scope are NOT affected.
//! 3. Dependency ordering is respected during scope shutdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::metadata::ResourceMetadata;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;
impl Config for TestConfig {}

struct NamedResource {
    name: &'static str,
}

impl Resource for NamedResource {
    type Config = TestConfig;
    type Instance = String;
    type Deps = ();

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from(self.name).expect("valid"))
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }
}

/// Resource that tracks cleanup calls via an atomic counter.
struct TrackingResource {
    name: &'static str,
    cleanup_count: Arc<AtomicU32>,
}

impl Resource for TrackingResource {
    type Config = TestConfig;
    type Instance = String;
    type Deps = ();

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from(self.name).expect("valid"))
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }

    async fn cleanup(&self, _instance: String) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Resource that records its cleanup order into a shared vector.
struct OrderedResource {
    name: &'static str,
    order: Arc<parking_lot::Mutex<Vec<String>>>,
    deps: Vec<&'static str>,
}

impl Resource for OrderedResource {
    type Config = TestConfig;
    type Instance = String;
    type Deps = ();

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from(self.name).expect("valid"))
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }

    async fn cleanup(&self, _instance: String) -> Result<()> {
        self.order.lock().push(self.name.to_string());
        Ok(())
    }

    fn dependencies(&self) -> Vec<ResourceKey> {
        self.deps
            .iter()
            .map(|&name| ResourceKey::try_from(name).expect("valid"))
            .collect()
    }
}

fn pool_cfg() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 4,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// T047: Tenant shutdown cascades to child workflow and execution resources
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn tenant_shutdown_cascades_to_child_scopes() {
    let mgr = Manager::new();

    // Register resources at three levels under tenant "A"
    mgr.register_scoped(
        NamedResource { name: "tenant-db" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf-cache" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "exec-temp" },
        TestConfig,
        pool_cfg(),
        Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string())),
    )
    .unwrap();

    // Verify all three are acquirable before shutdown
    let ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string())),
        WorkflowId::new(),
        ExecutionId::new(),
    );

    let tenant_db_key = ResourceKey::try_from("tenant-db").expect("valid resource key");
    let wf_cache_key = ResourceKey::try_from("wf-cache").expect("valid resource key");
    let exec_temp_key = ResourceKey::try_from("exec-temp").expect("valid resource key");

    let g1 = mgr.acquire(&tenant_db_key, &ctx).await.unwrap();
    let g2 = mgr.acquire(&wf_cache_key, &ctx).await.unwrap();
    let g3 = mgr.acquire(&exec_temp_key, &ctx).await.unwrap();
    drop(g1);
    drop(g2);
    drop(g3);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Shut down tenant "A" scope -- should cascade to all child scopes
    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    // All three resources should now be gone
    let err1 = mgr.acquire(&tenant_db_key, &ctx).await;
    let err2 = mgr.acquire(&wf_cache_key, &ctx).await;
    let err3 = mgr.acquire(&exec_temp_key, &ctx).await;

    assert!(
        err1.is_err(),
        "tenant-db should be unavailable after scope shutdown"
    );
    assert!(
        err2.is_err(),
        "wf-cache should be unavailable after scope shutdown"
    );
    assert!(
        err3.is_err(),
        "exec-temp should be unavailable after scope shutdown"
    );
}

// ---------------------------------------------------------------------------
// Shutdown of one tenant does NOT affect resources under a different tenant
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn scope_shutdown_does_not_affect_other_tenants() {
    let mgr = Manager::new();

    // Tenant A resources
    mgr.register_scoped(
        NamedResource { name: "db-A" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-A" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    // Tenant B resources
    mgr.register_scoped(
        NamedResource { name: "db-B" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("B"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-B" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "B"),
    )
    .unwrap();

    // Shut down tenant A
    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    let db_b_key = ResourceKey::try_from("db-B").expect("valid resource key");
    let cache_b_key = ResourceKey::try_from("cache-B").expect("valid resource key");
    let db_a_key = ResourceKey::try_from("db-A").expect("valid resource key");
    let cache_a_key = ResourceKey::try_from("cache-A").expect("valid resource key");

    // Tenant B resources should still be accessible
    let ctx_b = Context::new(Scope::tenant("B"), WorkflowId::new(), ExecutionId::new());
    let g1 = mgr.acquire(&db_b_key, &ctx_b).await;
    assert!(
        g1.is_ok(),
        "db-B should still be accessible after tenant A shutdown"
    );
    drop(g1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ctx_b_wf = Context::new(
        Scope::workflow_in_tenant("wf1", "B"),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    let g2 = mgr.acquire(&cache_b_key, &ctx_b_wf).await;
    assert!(
        g2.is_ok(),
        "cache-B should still be accessible after tenant A shutdown"
    );
    drop(g2);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Tenant A resources should be gone
    let ctx_a = Context::new(Scope::tenant("A"), WorkflowId::new(), ExecutionId::new());
    assert!(mgr.acquire(&db_a_key, &ctx_a).await.is_err());
    assert!(mgr.acquire(&cache_a_key, &ctx_a).await.is_err());
}

// ---------------------------------------------------------------------------
// Global scope shutdown cascades to everything
// ---------------------------------------------------------------------------

#[tokio::test]
async fn global_scope_shutdown_cascades_to_all() {
    let mgr = Manager::new();

    mgr.register(NamedResource { name: "global-r" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.register_scoped(
        NamedResource { name: "tenant-r" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("X"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf-r" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "X"),
    )
    .unwrap();

    mgr.shutdown_scope(&Scope::Global).await.unwrap();

    let global_r_key = ResourceKey::try_from("global-r").expect("valid resource key");
    let tenant_r_key = ResourceKey::try_from("tenant-r").expect("valid resource key");
    let wf_r_key = ResourceKey::try_from("wf-r").expect("valid resource key");

    let ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", Some("X".to_string())),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    assert!(mgr.acquire(&global_r_key, &ctx).await.is_err());
    assert!(mgr.acquire(&tenant_r_key, &ctx).await.is_err());
    assert!(mgr.acquire(&wf_r_key, &ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Workflow scope shutdown only affects that workflow, not sibling workflows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workflow_scope_shutdown_does_not_affect_siblings() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "cache-wf1" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-wf2" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf2", "A"),
    )
    .unwrap();

    // Shut down wf1 only
    mgr.shutdown_scope(&Scope::workflow_in_tenant("wf1", "A"))
        .await
        .unwrap();

    let cache_wf2_key = ResourceKey::try_from("cache-wf2").expect("valid resource key");
    let cache_wf1_key = ResourceKey::try_from("cache-wf1").expect("valid resource key");

    // wf2 resource should still work
    let ctx_wf2 = Context::new(
        Scope::workflow_in_tenant("wf2", "A"),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    assert!(mgr.acquire(&cache_wf2_key, &ctx_wf2).await.is_ok());

    // wf1 resource should be gone
    let ctx_wf1 = Context::new(
        Scope::workflow_in_tenant("wf1", "A"),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    assert!(mgr.acquire(&cache_wf1_key, &ctx_wf1).await.is_err());
}

// ---------------------------------------------------------------------------
// Dependency ordering: dependents shut down before their dependencies
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn shutdown_scope_follows_dependency_ordering() {
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mgr = Manager::new();

    // "app" depends on "cache" depends on "db"
    // All under the same tenant.
    // On shutdown, expected reverse topo order: app, cache, db
    mgr.register_scoped(
        OrderedResource {
            name: "db",
            order: order.clone(),
            deps: vec![],
        },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        OrderedResource {
            name: "cache",
            order: order.clone(),
            deps: vec!["db"],
        },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        OrderedResource {
            name: "app",
            order: order.clone(),
            deps: vec!["cache"],
        },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    // Acquire and release each to populate idle instances for cleanup.
    // Use a generous sleep to ensure the spawned return tasks complete.
    let ctx = Context::new(Scope::tenant("A"), WorkflowId::new(), ExecutionId::new());
    for name in &["db", "cache", "app"] {
        let key = ResourceKey::try_from(*name).expect("valid");
        let g = mgr.acquire(&key, &ctx).await.unwrap();
        drop(g);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    let cleanup_order = order.lock().clone();
    // All three pools had an idle instance, so all three must have been
    // cleaned up. Fail loudly if that didn't happen.
    assert_eq!(
        cleanup_order.len(),
        3,
        "expected 3 cleanup callbacks, got {}: {cleanup_order:?}",
        cleanup_order.len()
    );
    // "app" depends on "cache" depends on "db".
    // Reverse topo: app before cache, cache before db.
    let app_pos = cleanup_order.iter().position(|s| s == "app").unwrap();
    let cache_pos = cleanup_order.iter().position(|s| s == "cache").unwrap();
    let db_pos = cleanup_order.iter().position(|s| s == "db").unwrap();
    assert!(
        app_pos < cache_pos,
        "app should shut down before cache, got order: {cleanup_order:?}"
    );
    assert!(
        cache_pos < db_pos,
        "cache should shut down before db, got order: {cleanup_order:?}"
    );

    let app_key = ResourceKey::try_from("app").expect("valid resource key");
    let cache_key = ResourceKey::try_from("cache").expect("valid resource key");
    let db_key = ResourceKey::try_from("db").expect("valid resource key");

    assert!(mgr.acquire(&app_key, &ctx).await.is_err());
    assert!(mgr.acquire(&cache_key, &ctx).await.is_err());
    assert!(mgr.acquire(&db_key, &ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Cleanup callbacks are invoked during scope shutdown
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn scope_shutdown_invokes_cleanup() {
    let cleanup_count = Arc::new(AtomicU32::new(0));
    let mgr = Manager::new();

    mgr.register_scoped(
        TrackingResource {
            name: "tracked-db",
            cleanup_count: cleanup_count.clone(),
        },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    // Acquire and release to create an idle instance
    let ctx = Context::new(Scope::tenant("A"), WorkflowId::new(), ExecutionId::new());

    let tracked_db_key = ResourceKey::try_from("tracked-db").expect("valid resource key");

    {
        let _g = mgr.acquire(&tracked_db_key, &ctx).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "cleanup should have been called during scope shutdown"
    );
}
