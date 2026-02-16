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

use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

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

    fn id(&self) -> &str {
        self.name
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

    fn id(&self) -> &str {
        self.name
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

    fn id(&self) -> &str {
        self.name
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }

    async fn cleanup(&self, _instance: String) -> Result<()> {
        self.order.lock().push(self.name.to_string());
        Ok(())
    }

    fn dependencies(&self) -> Vec<&str> {
        self.deps.clone()
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

#[tokio::test]
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
        "wf1",
        "ex1",
    );
    let g1 = mgr.acquire("tenant-db", &ctx).await.unwrap();
    let g2 = mgr.acquire("wf-cache", &ctx).await.unwrap();
    let g3 = mgr.acquire("exec-temp", &ctx).await.unwrap();
    drop(g1);
    drop(g2);
    drop(g3);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Shut down tenant "A" scope -- should cascade to all child scopes
    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    // All three resources should now be gone
    let err1 = mgr.acquire("tenant-db", &ctx).await;
    let err2 = mgr.acquire("wf-cache", &ctx).await;
    let err3 = mgr.acquire("exec-temp", &ctx).await;

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

#[tokio::test]
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

    // Tenant B resources should still be accessible
    let ctx_b = Context::new(Scope::tenant("B"), "wf1", "ex1");
    let g1 = mgr.acquire("db-B", &ctx_b).await;
    assert!(
        g1.is_ok(),
        "db-B should still be accessible after tenant A shutdown"
    );
    drop(g1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ctx_b_wf = Context::new(Scope::workflow_in_tenant("wf1", "B"), "wf1", "ex1");
    let g2 = mgr.acquire("cache-B", &ctx_b_wf).await;
    assert!(
        g2.is_ok(),
        "cache-B should still be accessible after tenant A shutdown"
    );
    drop(g2);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Tenant A resources should be gone
    let ctx_a = Context::new(Scope::tenant("A"), "wf1", "ex1");
    assert!(mgr.acquire("db-A", &ctx_a).await.is_err());
    assert!(mgr.acquire("cache-A", &ctx_a).await.is_err());
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

    let ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", Some("X".to_string())),
        "wf1",
        "ex1",
    );
    assert!(mgr.acquire("global-r", &ctx).await.is_err());
    assert!(mgr.acquire("tenant-r", &ctx).await.is_err());
    assert!(mgr.acquire("wf-r", &ctx).await.is_err());
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

    // wf2 resource should still work
    let ctx_wf2 = Context::new(Scope::workflow_in_tenant("wf2", "A"), "wf2", "ex1");
    assert!(mgr.acquire("cache-wf2", &ctx_wf2).await.is_ok());

    // wf1 resource should be gone
    let ctx_wf1 = Context::new(Scope::workflow_in_tenant("wf1", "A"), "wf1", "ex1");
    assert!(mgr.acquire("cache-wf1", &ctx_wf1).await.is_err());
}

// ---------------------------------------------------------------------------
// Dependency ordering: dependents shut down before their dependencies
// ---------------------------------------------------------------------------

#[tokio::test]
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
    let ctx = Context::new(Scope::tenant("A"), "wf", "ex");
    for name in &["db", "cache", "app"] {
        let g = mgr.acquire(name, &ctx).await.unwrap();
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

    assert!(mgr.acquire("app", &ctx).await.is_err());
    assert!(mgr.acquire("cache", &ctx).await.is_err());
    assert!(mgr.acquire("db", &ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Cleanup callbacks are invoked during scope shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
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
    let ctx = Context::new(Scope::tenant("A"), "wf", "ex");
    {
        let _g = mgr.acquire("tracked-db", &ctx).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    mgr.shutdown_scope(&Scope::tenant("A")).await.unwrap();

    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "cleanup should have been called during scope shutdown"
    );
}
