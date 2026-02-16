//! Manager double-register tests.
//!
//! Verifies that registering a resource with the same ID twice replaces
//! the pool correctly. Also checks that guards from the old pool do not
//! panic when dropped after replacement.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::manager::Manager;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TaggedConfig {
    tag: String,
}

impl Config for TaggedConfig {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// Resource that embeds the config tag into each instance.
struct TaggedResource {
    create_count: Arc<AtomicU32>,
}

impl TaggedResource {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Resource for TaggedResource {
    type Config = TaggedConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "db"
    }

    async fn create(&self, config: &TaggedConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("{}-{n}", config.tag))
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Test 4a: double_register_replaces_pool
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn double_register_replaces_pool() {
    let mgr = Manager::new();

    // First registration with tag "first"
    mgr.register(
        TaggedResource::new(),
        TaggedConfig {
            tag: "first".into(),
        },
        pool_config(),
    )
    .unwrap();

    // Acquire from first pool
    let guard = mgr.acquire("db", &ctx()).await.unwrap();
    let instance = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert!(
        instance.starts_with("first-"),
        "should be from first pool, got: {instance}"
    );
    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Re-register with tag "second" (same resource ID "db")
    mgr.register(
        TaggedResource::new(),
        TaggedConfig {
            tag: "second".into(),
        },
        pool_config(),
    )
    .unwrap();

    // Acquire from new pool
    let guard = mgr.acquire("db", &ctx()).await.unwrap();
    let instance = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert!(
        instance.starts_with("second-"),
        "should be from second pool after re-register, got: {instance}"
    );
}

// ---------------------------------------------------------------------------
// Test 4b: double_register_with_active_guards
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn double_register_with_active_guards() {
    let mgr = Manager::new();

    // Register first pool
    mgr.register(
        TaggedResource::new(),
        TaggedConfig { tag: "old".into() },
        pool_config(),
    )
    .unwrap();

    // Acquire a guard from the first pool and HOLD it
    let old_guard = mgr.acquire("db", &ctx()).await.unwrap();
    let old_instance = old_guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast")
        .clone();
    assert!(old_instance.starts_with("old-"));

    // Re-register with new config (replaces pool in registry)
    mgr.register(
        TaggedResource::new(),
        TaggedConfig { tag: "new".into() },
        pool_config(),
    )
    .unwrap();

    // Drop old guard: the old pool still exists via Arc in the guard's
    // drop callback. This should NOT panic.
    drop(old_guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Acquire from the new pool
    let new_guard = mgr.acquire("db", &ctx()).await.unwrap();
    let new_instance = new_guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert!(
        new_instance.starts_with("new-"),
        "should be from new pool, got: {new_instance}"
    );
}
