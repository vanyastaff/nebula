//! Tests that `Pool::retain` predicate receives `&InstanceMetadata` instead of
//! raw `(Instant, Instant)` arguments.

use nebula_core::{resource_key, ResourceKey};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

// ---------------------------------------------------------------------------
// Minimal test resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize, Default)]
struct DummyConfig;

impl Config for DummyConfig {}

#[derive(Debug, Default)]
struct DummyResource;

impl Resource for DummyResource {
    type Config = DummyConfig;
    type Instance = ();

    fn key(&self) -> ResourceKey {
        resource_key!("dummy-retain")
    }

    async fn create(&self, _config: &DummyConfig, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retain_predicate_receives_instance_metadata() {
    let pool = Pool::new(DummyResource, DummyConfig, PoolConfig::default()).unwrap();
    let ctx = ctx();

    // First cycle: acquire two fresh instances and return them.
    // At this point the idle entries have acquire_count == 0 (Entry::new).
    let g1 = pool.acquire(&ctx).await.unwrap();
    let g2 = pool.acquire(&ctx).await.unwrap();
    drop(g1);
    drop(g2);
    tokio::task::yield_now().await;

    // Second cycle: re-acquire and re-release so acquire_count becomes 1.
    let g3 = pool.acquire(&ctx).await.unwrap();
    let g4 = pool.acquire(&ctx).await.unwrap();
    drop(g3);
    drop(g4);
    tokio::task::yield_now().await;

    // Both entries should have acquire_count == 1 now — none should be evicted.
    let evicted = pool.retain(|_inst, meta| meta.acquire_count >= 1).await;
    assert_eq!(evicted, 0, "all entries survive retain with acquire_count >= 1");

    // Retain nothing — both should be evicted.
    let evicted = pool.retain(|_inst, _meta| false).await;
    assert_eq!(evicted, 2, "both entries evicted when predicate returns false");
}
