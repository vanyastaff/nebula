//! T046: Manager scope enforcement integration tests.
//!
//! These tests focus on complex multi-resource scenarios that go beyond
//! the single-resource scope isolation tests in `scope_isolation.rs`.
//! They verify that scope enforcement is correct when multiple resources
//! with different scopes coexist in the same Manager.

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

fn pool_cfg() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 4,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Multi-tenant isolation: two tenants each with their own resources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_tenants_each_access_only_own_resources() {
    let mgr = Manager::new();

    // Register "db-A" under tenant A, "db-B" under tenant B
    mgr.register_scoped(
        NamedResource { name: "db-A" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "db-B" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("B"),
    )
    .unwrap();

    let ctx_a = Context::new(Scope::tenant("A"), "wf1", "ex1");
    let ctx_b = Context::new(Scope::tenant("B"), "wf1", "ex1");

    // Tenant A can access db-A, not db-B
    let g = mgr.acquire("db-A", &ctx_a).await.unwrap();
    assert_eq!(
        g.as_any().downcast_ref::<String>().unwrap(),
        "db-A-instance"
    );

    let err = mgr.acquire("db-B", &ctx_a).await.unwrap_err();
    assert!(err.to_string().contains("Scope mismatch"));

    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Tenant B can access db-B, not db-A
    let g = mgr.acquire("db-B", &ctx_b).await.unwrap();
    assert_eq!(
        g.as_any().downcast_ref::<String>().unwrap(),
        "db-B-instance"
    );

    let err = mgr.acquire("db-A", &ctx_b).await.unwrap_err();
    assert!(err.to_string().contains("Scope mismatch"));
}

// ---------------------------------------------------------------------------
// Mixed global + tenant: global resource accessible from both tenants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn global_resource_shared_across_tenants() {
    let mgr = Manager::new();

    // One global resource and two tenant-scoped resources
    mgr.register(NamedResource { name: "metrics" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-A" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-B" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("B"),
    )
    .unwrap();

    let ctx_a = Context::new(Scope::tenant("A"), "wf1", "ex1");
    let ctx_b = Context::new(Scope::tenant("B"), "wf1", "ex1");

    // Both tenants can access global "metrics"
    let g1 = mgr.acquire("metrics", &ctx_a).await.unwrap();
    drop(g1);
    tokio::time::sleep(Duration::from_millis(30)).await;

    let g2 = mgr.acquire("metrics", &ctx_b).await.unwrap();
    drop(g2);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Each tenant only sees their own cache
    mgr.acquire("cache-A", &ctx_a).await.unwrap();
    assert!(mgr.acquire("cache-A", &ctx_b).await.is_err());
    assert!(mgr.acquire("cache-B", &ctx_a).await.is_err());
}

// ---------------------------------------------------------------------------
// Workflow-scoped resource accessible from execution within that workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workflow_resource_accessible_from_child_execution() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "wf-cache" },
        TestConfig,
        pool_cfg(),
        Scope::workflow("wf1"),
    )
    .unwrap();

    // Execution inside wf1 can access it
    let exec_ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", None),
        "wf1",
        "ex1",
    );
    let g = mgr.acquire("wf-cache", &exec_ctx).await.unwrap();
    assert!(g.as_any().downcast_ref::<String>().is_some());
    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Execution inside wf2 cannot
    let other_exec_ctx = Context::new(
        Scope::execution_in_workflow("ex2", "wf2", None),
        "wf2",
        "ex2",
    );
    assert!(mgr.acquire("wf-cache", &other_exec_ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Workflow-scoped resource NOT accessible from different workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workflow_resource_denied_from_different_workflow() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "wf1-db" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf2-db" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf2", "A"),
    )
    .unwrap();

    // Context for wf1 execution
    let wf1_ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string())),
        "wf1",
        "ex1",
    );

    // Context for wf2 execution
    let wf2_ctx = Context::new(
        Scope::execution_in_workflow("ex2", "wf2", Some("A".to_string())),
        "wf2",
        "ex2",
    );

    // wf1 context can access wf1-db, not wf2-db
    mgr.acquire("wf1-db", &wf1_ctx).await.unwrap();
    assert!(mgr.acquire("wf2-db", &wf1_ctx).await.is_err());

    // wf2 context can access wf2-db, not wf1-db
    mgr.acquire("wf2-db", &wf2_ctx).await.unwrap();
    assert!(mgr.acquire("wf1-db", &wf2_ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Tenant-scoped resource accessible from deeply nested action scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenant_resource_accessible_from_nested_action() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "tenant-db" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    // Action deep inside tenant A's hierarchy
    let action_ctx = Context::new(
        Scope::action_in_execution(
            "act1",
            "ex1",
            Some("wf1".to_string()),
            Some("A".to_string()),
        ),
        "wf1",
        "ex1",
    );
    let g = mgr.acquire("tenant-db", &action_ctx).await.unwrap();
    assert!(g.as_any().downcast_ref::<String>().is_some());
    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Action in tenant B's hierarchy cannot access tenant A's resource
    let action_ctx_b = Context::new(
        Scope::action_in_execution(
            "act2",
            "ex2",
            Some("wf2".to_string()),
            Some("B".to_string()),
        ),
        "wf2",
        "ex2",
    );
    assert!(mgr.acquire("tenant-db", &action_ctx_b).await.is_err());
}

// ---------------------------------------------------------------------------
// Multiple resources at different scope levels: full isolation matrix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_isolation_matrix_across_scope_levels() {
    let mgr = Manager::new();

    // Register resources at various scope levels
    mgr.register(NamedResource { name: "global-r" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.register_scoped(
        NamedResource { name: "tenant-r" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf-r" },
        TestConfig,
        pool_cfg(),
        Scope::workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource {
            name: "execution-r",
        },
        TestConfig,
        pool_cfg(),
        Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string())),
    )
    .unwrap();

    // Context at execution level inside tenant A / wf1 / ex1
    let exec_ctx = Context::new(
        Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string())),
        "wf1",
        "ex1",
    );

    // This context should be able to access: global, tenant-A, wf1, ex1
    mgr.acquire("global-r", &exec_ctx).await.unwrap();
    mgr.acquire("tenant-r", &exec_ctx).await.unwrap();
    mgr.acquire("wf-r", &exec_ctx).await.unwrap();
    mgr.acquire("execution-r", &exec_ctx).await.unwrap();

    // Context at tenant level (broader than wf/execution scope)
    let tenant_ctx = Context::new(Scope::tenant("A"), "wf1", "ex1");

    // Tenant context can access global and tenant, but NOT workflow or execution
    mgr.acquire("global-r", &tenant_ctx).await.unwrap();
    mgr.acquire("tenant-r", &tenant_ctx).await.unwrap();
    assert!(mgr.acquire("wf-r", &tenant_ctx).await.is_err());
    assert!(mgr.acquire("execution-r", &tenant_ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Concurrent multi-tenant acquire: both tenants can acquire simultaneously
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_multi_tenant_acquire() {
    let mgr = std::sync::Arc::new(Manager::new());

    mgr.register_scoped(
        NamedResource { name: "pool-A" },
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
        Scope::tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "pool-B" },
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
        Scope::tenant("B"),
    )
    .unwrap();

    let mgr_a = mgr.clone();
    let mgr_b = mgr.clone();

    let handle_a = tokio::spawn(async move {
        let ctx = Context::new(Scope::tenant("A"), "wf", "ex");
        let g1 = mgr_a.acquire("pool-A", &ctx).await.unwrap();
        let g2 = mgr_a.acquire("pool-A", &ctx).await.unwrap();
        // Cannot access pool-B
        assert!(mgr_a.acquire("pool-B", &ctx).await.is_err());
        drop(g1);
        drop(g2);
    });

    let handle_b = tokio::spawn(async move {
        let ctx = Context::new(Scope::tenant("B"), "wf", "ex");
        let g1 = mgr_b.acquire("pool-B", &ctx).await.unwrap();
        let g2 = mgr_b.acquire("pool-B", &ctx).await.unwrap();
        // Cannot access pool-A
        assert!(mgr_b.acquire("pool-A", &ctx).await.is_err());
        drop(g1);
        drop(g2);
    });

    handle_a.await.unwrap();
    handle_b.await.unwrap();
}
