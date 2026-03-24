//! Pool swap integration contract for config reload.
//!
//! Covers RSC-T012: old pool drains while new pool activates cleanly.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, Manager, PoolAcquire, PoolSizing, WorkflowId};

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct VersionedResource {
    version: &'static str,
    cleanup_count: Arc<AtomicU32>,
}

impl VersionedResource {
    fn new(version: &'static str, cleanup_count: Arc<AtomicU32>) -> Self {
        Self {
            version,
            cleanup_count,
        }
    }
}

impl Resource for VersionedResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("pool-swap")
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(self.version.to_string())
    }

    async fn destroy(&self, _instance: Self::Instance) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

#[tokio::test(flavor = "multi_thread")]
async fn config_reload_swaps_to_new_pool_while_old_drains() {
    let cleanup_count = Arc::new(AtomicU32::new(0));

    let manager = Manager::new();
    manager
        .register(
            VersionedResource::new("v1", Arc::clone(&cleanup_count)),
            TestConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 4,
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_secs(1),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("initial register");

    let key = resource_key!("pool-swap");

    // Hold a guard from the old pool so old pool cannot be fully drained yet.
    let old_guard = manager.acquire(&key, &ctx()).await.expect("old acquire");
    let old_instance = old_guard
        .as_any()
        .downcast_ref::<String>()
        .expect("downcast old instance");
    assert_eq!(old_instance, "v1");

    // Swap config/resource implementation.
    manager
        .reload_config(
            VersionedResource::new("v2", Arc::clone(&cleanup_count)),
            TestConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 4,
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_secs(1),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .expect("reload succeeds");

    // New acquires must come from new pool immediately.
    let new_guard = manager.acquire(&key, &ctx()).await.expect("new acquire");
    let new_instance = new_guard
        .as_any()
        .downcast_ref::<String>()
        .expect("downcast new instance");
    assert_eq!(new_instance, "v2");
    drop(new_guard);

    let cleanup_before = cleanup_count.load(Ordering::SeqCst);
    drop(old_guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        cleanup_count.load(Ordering::SeqCst) > cleanup_before,
        "dropping old guard should cleanup through drained old pool"
    );

    let final_guard = manager.acquire(&key, &ctx()).await.expect("final acquire");
    let final_instance = final_guard
        .as_any()
        .downcast_ref::<String>()
        .expect("downcast final instance");
    assert_eq!(final_instance, "v2");
}


