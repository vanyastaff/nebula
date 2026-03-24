//! Verifies scope-aware dependency injection and multi-scope pool selection.
//!
//! T049: When the same logical resource is registered under multiple scopes,
//! `Manager::acquire` must select the most-specific scope-compatible pool.

use std::time::Duration;

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, PoolAcquire, PoolSizing, WorkflowId};

// ---------------------------------------------------------------------------
// LabeledResource — creates instances with a label so we can tell which
// pool was selected.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct LabeledConfig {
    label: String,
}
impl Config for LabeledConfig {}

#[derive(Debug)]
pub struct LabeledInstance {
    pub label: String,
}

#[derive(Debug, Clone)]
struct LabeledResource;

impl Resource for LabeledResource {
    type Config = LabeledConfig;
    type Instance = LabeledInstance;

    fn key(&self) -> ResourceKey {
        resource_key!("logger")
    }

    async fn create(&self, cfg: &LabeledConfig, _ctx: &Context) -> Result<LabeledInstance> {
        Ok(LabeledInstance {
            label: cfg.label.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pool_cfg() -> PoolConfig {
    PoolConfig {
        sizing: PoolSizing {
            min_size: 0,
            max_size: 4,
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(5),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn tenant_ctx(tenant: &str) -> Context {
    Context::new(
        Scope::try_tenant(tenant).unwrap(),
        WorkflowId::new(),
        ExecutionId::new(),
    )
}

fn global_ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When a resource is registered under both Global and Tenant("acme") scopes,
/// acquiring with a Tenant("acme") context must return the tenant-specific pool.
#[tokio::test]
async fn tenant_specific_pool_preferred_over_global() {
    let mgr = Manager::new();

    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "global".into(),
        },
        pool_cfg(),
        Scope::Global,
    )
    .unwrap();

    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "tenant-acme".into(),
        },
        pool_cfg(),
        Scope::try_tenant("acme").unwrap(),
    )
    .unwrap();

    // Tenant context → tenant-specific instance.
    let guard = mgr
        .acquire(&resource_key!("logger"), &tenant_ctx("acme"))
        .await
        .unwrap();
    let inst = guard.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(
        inst.label, "tenant-acme",
        "tenant context must use tenant-scoped pool"
    );

    // Global context → global instance.
    let guard2 = mgr
        .acquire(&resource_key!("logger"), &global_ctx())
        .await
        .unwrap();
    let inst2 = guard2.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(inst2.label, "global", "global context must use global pool");
}

/// When only a Global pool exists, any narrower scope (e.g. Tenant) falls back to it.
#[tokio::test]
async fn falls_back_to_global_when_no_tenant_specific() {
    let mgr = Manager::new();

    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "global".into(),
        },
        pool_cfg(),
        Scope::Global,
    )
    .unwrap();

    // Tenant context, but only Global is registered → should succeed with global.
    let guard = mgr
        .acquire(&resource_key!("logger"), &tenant_ctx("acme"))
        .await
        .unwrap();
    let inst = guard.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(
        inst.label, "global",
        "falls back to global when no tenant pool exists"
    );
}

/// Two different tenant pools must not interfere with each other.
#[tokio::test]
async fn exact_scope_match_is_preferred_over_other_tenant() {
    let mgr = Manager::new();

    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "global".into(),
        },
        pool_cfg(),
        Scope::Global,
    )
    .unwrap();
    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "tenant-acme".into(),
        },
        pool_cfg(),
        Scope::try_tenant("acme").unwrap(),
    )
    .unwrap();
    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "tenant-other".into(),
        },
        pool_cfg(),
        Scope::try_tenant("other").unwrap(),
    )
    .unwrap();

    let guard = mgr
        .acquire(&resource_key!("logger"), &tenant_ctx("acme"))
        .await
        .unwrap();
    let inst = guard.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(
        inst.label, "tenant-acme",
        "acme context picks acme pool, not other-tenant pool"
    );

    let guard2 = mgr
        .acquire(&resource_key!("logger"), &tenant_ctx("other"))
        .await
        .unwrap();
    let inst2 = guard2.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(
        inst2.label, "tenant-other",
        "other context picks other pool, not acme pool"
    );
}

/// `deregister_scoped` removes only the specified scope; others remain accessible.
#[tokio::test]
async fn deregister_scoped_only_removes_that_scope() {
    let mgr = Manager::new();

    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "global".into(),
        },
        pool_cfg(),
        Scope::Global,
    )
    .unwrap();
    mgr.register_scoped(
        LabeledResource,
        LabeledConfig {
            label: "tenant-acme".into(),
        },
        pool_cfg(),
        Scope::try_tenant("acme").unwrap(),
    )
    .unwrap();

    // Remove the global one.
    mgr.deregister_scoped(&resource_key!("logger"), &Scope::Global)
        .await;

    // Tenant pool still accessible.
    let guard = mgr
        .acquire(&resource_key!("logger"), &tenant_ctx("acme"))
        .await
        .unwrap();
    let inst = guard.as_any().downcast_ref::<LabeledInstance>().unwrap();
    assert_eq!(
        inst.label, "tenant-acme",
        "tenant pool survives global deregister"
    );

    // Global pool is gone — acquire with global ctx fails.
    let result = mgr.acquire(&resource_key!("logger"), &global_ctx()).await;
    assert!(
        result.is_err(),
        "global pool should be gone after deregister_scoped"
    );
}


