//! T046: Manager scope enforcement integration tests.
//!
//! These tests focus on complex multi-resource scenarios that go beyond
//! the single-resource scope isolation tests in `scope_isolation.rs`.
//! They verify that scope enforcement is correct when multiple resources
//! with different scopes coexist in the same Manager.

use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::{ExecutionId, WorkflowId};

mod scope_helpers;
use scope_helpers::*;



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
    fn key(&self) -> ResourceKey {
        ResourceKey::try_from(self.name).expect("valid")
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

#[tokio::test(start_paused = true)]
async fn two_tenants_each_access_only_own_resources() {
    let mgr = Manager::new();

    // Register "db-A" under tenant A, "db-B" under tenant B
    mgr.register_scoped(
        NamedResource { name: "db-A" },
        TestConfig,
        pool_cfg(),
        scope_tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "db-B" },
        TestConfig,
        pool_cfg(),
        scope_tenant("B"),
    )
    .unwrap();

    let ctx_a = Context::new(scope_tenant("A"), WorkflowId::new(), ExecutionId::new());
    let ctx_b = Context::new(scope_tenant("B"), WorkflowId::new(), ExecutionId::new());

    let key_a = ResourceKey::try_from("db-A").expect("valid resource key");
    let key_b = ResourceKey::try_from("db-B").expect("valid resource key");

    // Tenant A can access db-A, not db-B
    let g = mgr.acquire(&key_a, &ctx_a).await.unwrap();
    assert_eq!(
        g.as_any().downcast_ref::<String>().unwrap(),
        "db-A-instance"
    );

    let err = mgr.acquire(&key_b, &ctx_a).await.unwrap_err();
    assert!(err.to_string().contains("Scope mismatch"));

    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Tenant B can access db-B, not db-A
    let g = mgr.acquire(&key_b, &ctx_b).await.unwrap();
    assert_eq!(
        g.as_any().downcast_ref::<String>().unwrap(),
        "db-B-instance"
    );

    let err = mgr.acquire(&key_a, &ctx_b).await.unwrap_err();
    assert!(err.to_string().contains("Scope mismatch"));
}

// ---------------------------------------------------------------------------
// Mixed global + tenant: global resource accessible from both tenants
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn global_resource_shared_across_tenants() {
    let mgr = Manager::new();

    // One global resource and two tenant-scoped resources
    mgr.register(NamedResource { name: "metrics" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-A" },
        TestConfig,
        pool_cfg(),
        scope_tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "cache-B" },
        TestConfig,
        pool_cfg(),
        scope_tenant("B"),
    )
    .unwrap();

    let ctx_a = Context::new(scope_tenant("A"), WorkflowId::new(), ExecutionId::new());
    let ctx_b = Context::new(scope_tenant("B"), WorkflowId::new(), ExecutionId::new());

    let metrics_key = ResourceKey::try_from("metrics").expect("valid resource key");
    let cache_a_key = ResourceKey::try_from("cache-A").expect("valid resource key");
    let cache_b_key = ResourceKey::try_from("cache-B").expect("valid resource key");

    // Both tenants can access global "metrics"
    let g1 = mgr.acquire(&metrics_key, &ctx_a).await.unwrap();
    drop(g1);
    tokio::time::sleep(Duration::from_millis(30)).await;

    let g2 = mgr.acquire(&metrics_key, &ctx_b).await.unwrap();
    drop(g2);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Each tenant only sees their own cache
    mgr.acquire(&cache_a_key, &ctx_a).await.unwrap();
    assert!(mgr.acquire(&cache_a_key, &ctx_b).await.is_err());
    assert!(mgr.acquire(&cache_b_key, &ctx_a).await.is_err());
}

// ---------------------------------------------------------------------------
// Workflow-scoped resource accessible from execution within that workflow
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn workflow_resource_accessible_from_child_execution() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "wf-cache" },
        TestConfig,
        pool_cfg(),
        scope_workflow("wf1"),
    )
    .unwrap();

    let wf_cache_key = ResourceKey::try_from("wf-cache").expect("valid resource key");

    // Execution inside wf1 can access it
    let exec_ctx = Context::new(
        scope_execution_in_workflow("ex1", "wf1", None),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    let g = mgr.acquire(&wf_cache_key, &exec_ctx).await.unwrap();
    assert!(g.as_any().downcast_ref::<String>().is_some());
    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Execution inside wf2 cannot
    let other_exec_ctx = Context::new(
        scope_execution_in_workflow("ex2", "wf2", None),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    assert!(mgr.acquire(&wf_cache_key, &other_exec_ctx).await.is_err());
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
        scope_workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf2-db" },
        TestConfig,
        pool_cfg(),
        scope_workflow_in_tenant("wf2", "A"),
    )
    .unwrap();

    // Context for wf1 execution
    let wf1_ctx = Context::new(
        scope_execution_in_workflow("ex1", "wf1", Some("A".to_string())),
        WorkflowId::new(),
        ExecutionId::new(),
    );

    // Context for wf2 execution
    let wf2_ctx = Context::new(
        scope_execution_in_workflow("ex2", "wf2", Some("A".to_string())),
        WorkflowId::new(),
        ExecutionId::new(),
    );

    let wf1_db_key = ResourceKey::try_from("wf1-db").expect("valid resource key");
    let wf2_db_key = ResourceKey::try_from("wf2-db").expect("valid resource key");

    // wf1 context can access wf1-db, not wf2-db
    mgr.acquire(&wf1_db_key, &wf1_ctx).await.unwrap();
    assert!(mgr.acquire(&wf2_db_key, &wf1_ctx).await.is_err());

    // wf2 context can access wf2-db, not wf1-db
    mgr.acquire(&wf2_db_key, &wf2_ctx).await.unwrap();
    assert!(mgr.acquire(&wf1_db_key, &wf2_ctx).await.is_err());
}

// ---------------------------------------------------------------------------
// Tenant-scoped resource accessible from deeply nested action scope
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn tenant_resource_accessible_from_nested_action() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "tenant-db" },
        TestConfig,
        pool_cfg(),
        scope_tenant("A"),
    )
    .unwrap();

    // Action deep inside tenant A's hierarchy
    let action_ctx = Context::new(
        scope_action_in_execution(
            "act1",
            "ex1",
            Some("wf1".to_string()),
            Some("A".to_string()),
        ),
        WorkflowId::new(),
        ExecutionId::new(),
    );

    let tenant_db_key = ResourceKey::try_from("tenant-db").expect("valid resource key");

    let g = mgr.acquire(&tenant_db_key, &action_ctx).await.unwrap();
    assert!(g.as_any().downcast_ref::<String>().is_some());
    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Action in tenant B's hierarchy cannot access tenant A's resource
    let action_ctx_b = Context::new(
        scope_action_in_execution(
            "act2",
            "ex2",
            Some("wf2".to_string()),
            Some("B".to_string()),
        ),
        WorkflowId::new(),
        ExecutionId::new(),
    );
    assert!(mgr.acquire(&tenant_db_key, &action_ctx_b).await.is_err());
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
        scope_tenant("A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource { name: "wf-r" },
        TestConfig,
        pool_cfg(),
        scope_workflow_in_tenant("wf1", "A"),
    )
    .unwrap();

    mgr.register_scoped(
        NamedResource {
            name: "execution-r",
        },
        TestConfig,
        pool_cfg(),
        scope_execution_in_workflow("ex1", "wf1", Some("A".to_string())),
    )
    .unwrap();

    // Context at execution level inside tenant A / wf1 / ex1
    let exec_ctx = Context::new(
        scope_execution_in_workflow("ex1", "wf1", Some("A".to_string())),
        WorkflowId::new(),
        ExecutionId::new(),
    );

    let global_key = ResourceKey::try_from("global-r").expect("valid resource key");
    let tenant_key = ResourceKey::try_from("tenant-r").expect("valid resource key");
    let wf_key = ResourceKey::try_from("wf-r").expect("valid resource key");
    let execution_key = ResourceKey::try_from("execution-r").expect("valid resource key");

    // This context should be able to access: global, tenant-A, wf1, ex1
    mgr.acquire(&global_key, &exec_ctx).await.unwrap();
    mgr.acquire(&tenant_key, &exec_ctx).await.unwrap();
    mgr.acquire(&wf_key, &exec_ctx).await.unwrap();
    mgr.acquire(&execution_key, &exec_ctx).await.unwrap();

    // Context at tenant level (broader than wf/execution scope)
    let tenant_ctx = Context::new(scope_tenant("A"), WorkflowId::new(), ExecutionId::new());

    // Tenant context can access global and tenant, but NOT workflow or execution
    mgr.acquire(&global_key, &tenant_ctx).await.unwrap();
    mgr.acquire(&tenant_key, &tenant_ctx).await.unwrap();
    assert!(mgr.acquire(&wf_key, &tenant_ctx).await.is_err());
    assert!(mgr.acquire(&execution_key, &tenant_ctx).await.is_err());
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
        scope_tenant("A"),
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
        scope_tenant("B"),
    )
    .unwrap();

    let mgr_a = mgr.clone();
    let mgr_b = mgr.clone();

    let handle_a = tokio::spawn(async move {
        let ctx = Context::new(scope_tenant("A"), WorkflowId::new(), ExecutionId::new());
        let key_a = ResourceKey::try_from("pool-A").expect("valid");
        let key_b = ResourceKey::try_from("pool-B").expect("valid");
        let g1 = mgr_a.acquire(&key_a, &ctx).await.unwrap();
        let g2 = mgr_a.acquire(&key_a, &ctx).await.unwrap();
        // Cannot access pool-B
        assert!(mgr_a.acquire(&key_b, &ctx).await.is_err());
        drop(g1);
        drop(g2);
    });

    let handle_b = tokio::spawn(async move {
        let ctx = Context::new(scope_tenant("B"), WorkflowId::new(), ExecutionId::new());
        let key_a = ResourceKey::try_from("pool-A").expect("valid");
        let key_b = ResourceKey::try_from("pool-B").expect("valid");
        let g1 = mgr_b.acquire(&key_b, &ctx).await.unwrap();
        let g2 = mgr_b.acquire(&key_b, &ctx).await.unwrap();
        // Cannot access pool-A
        assert!(mgr_b.acquire(&key_a, &ctx).await.is_err());
        drop(g1);
        drop(g2);
    });

    handle_a.await.unwrap();
    handle_b.await.unwrap();
}
