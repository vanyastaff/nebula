//! Integration tests for Create and Destroy lifecycle hooks.
//!
//! Verifies that when a [`HookRegistry`] is attached to a [`Pool`], the pool
//! fires [`HookEvent::Create`] before/after `Resource::create()` and
//! [`HookEvent::Destroy`] before/after `Resource::destroy()`.
//!
//! Also verifies that hooks fire through the [`Manager`] registration path.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::hooks::{HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, PoolAcquire, PoolLifetime, PoolSizing, WorkflowId};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        sizing: PoolSizing {
            min_size: 0,
            max_size: 3,
        },
        lifetime: PoolLifetime {
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            ..Default::default()
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Simple test resource
// ---------------------------------------------------------------------------

struct SimpleResource;

impl Resource for SimpleResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("simple")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("instance".to_string())
    }
}

// ---------------------------------------------------------------------------
// Counting hook — records before/after invocations per event type
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CountingHook {
    name: String,
    create_before: AtomicU32,
    create_after: AtomicU32,
    cleanup_before: AtomicU32,
    cleanup_after: AtomicU32,
    acquire_before: AtomicU32,
    acquire_after: AtomicU32,
    release_before: AtomicU32,
    release_after: AtomicU32,
}

impl CountingHook {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            create_before: AtomicU32::new(0),
            create_after: AtomicU32::new(0),
            cleanup_before: AtomicU32::new(0),
            cleanup_after: AtomicU32::new(0),
            acquire_before: AtomicU32::new(0),
            acquire_after: AtomicU32::new(0),
            release_before: AtomicU32::new(0),
            release_after: AtomicU32::new(0),
        }
    }

    fn create_before_count(&self) -> u32 {
        self.create_before.load(Ordering::SeqCst)
    }

    fn create_after_count(&self) -> u32 {
        self.create_after.load(Ordering::SeqCst)
    }

    fn cleanup_before_count(&self) -> u32 {
        self.cleanup_before.load(Ordering::SeqCst)
    }

    fn cleanup_after_count(&self) -> u32 {
        self.cleanup_after.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    fn acquire_before_count(&self) -> u32 {
        self.acquire_before.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    fn acquire_after_count(&self) -> u32 {
        self.acquire_after.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ResourceHook for CountingHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        50
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![
            HookEvent::Create,
            HookEvent::Destroy,
            HookEvent::Acquire,
            HookEvent::Release,
        ]
    }

    fn filter(&self) -> HookFilter {
        HookFilter::All
    }

    async fn before(&self, event: &HookEvent, _resource_id: &str, _ctx: &Context) -> HookResult {
        match event {
            HookEvent::Create => {
                self.create_before.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Destroy => {
                self.cleanup_before.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Acquire => {
                self.acquire_before.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Release => {
                self.release_before.fetch_add(1, Ordering::SeqCst);
            }
            // New P1 hook variants — no-op in this test fixture.
            _ => {}
        }
        HookResult::Continue
    }

    async fn after(&self, event: &HookEvent, _resource_id: &str, _ctx: &Context, _success: bool) {
        match event {
            HookEvent::Create => {
                self.create_after.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Destroy => {
                self.cleanup_after.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Acquire => {
                self.acquire_after.fetch_add(1, Ordering::SeqCst);
            }
            HookEvent::Release => {
                self.release_after.fetch_add(1, Ordering::SeqCst);
            }
            // New P1 hook variants — no-op in this test fixture.
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Cancelling hook — cancels Create operations
// ---------------------------------------------------------------------------

struct CancelCreateHook;

#[async_trait]
impl ResourceHook for CancelCreateHook {
    fn name(&self) -> &str {
        "cancel-create"
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Create]
    }

    async fn before(&self, _event: &HookEvent, _resource_id: &str, _ctx: &Context) -> HookResult {
        HookResult::Cancel(nebula_resource::error::Error::Unavailable {
            resource_key: resource_key!("simple"),
            reason: "Create cancelled by hook".to_string(),
            retryable: false,
        })
    }

    async fn after(&self, _event: &HookEvent, _resource_id: &str, _ctx: &Context, _success: bool) {}
}

// ---------------------------------------------------------------------------
// Test 1: Pool fires Create hooks on first acquire (cold pool)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_fires_create_hooks_on_first_acquire() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        SimpleResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // First acquire — no idle instances, so create() is called.
    let (guard, _) = pool.acquire(&ctx()).await.expect("acquire should succeed");
    assert_eq!(*guard, "instance");

    assert_eq!(
        hook.create_before_count(),
        1,
        "Create before-hook should fire once"
    );
    assert_eq!(
        hook.create_after_count(),
        1,
        "Create after-hook should fire once"
    );

    drop(guard);
    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 2: Pool fires Cleanup hooks on shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_fires_cleanup_hooks_on_shutdown() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        SimpleResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // Acquire and release two instances.
    {
        let (g1, _) = pool.acquire(&ctx()).await.unwrap();
        let (g2, _) = pool.acquire(&ctx()).await.unwrap();
        drop(g1);
        drop(g2);
    }
    // Allow return_instance tasks to complete.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(hook.create_before_count(), 2);
    assert_eq!(hook.create_after_count(), 2);

    // Cleanup hooks should be zero before shutdown.
    assert_eq!(hook.cleanup_before_count(), 0);
    assert_eq!(hook.cleanup_after_count(), 0);

    // Shutdown cleans up all idle instances.
    pool.shutdown().await.unwrap();

    assert_eq!(
        hook.cleanup_before_count(),
        2,
        "Cleanup before-hook should fire for each idle instance"
    );
    assert_eq!(
        hook.cleanup_after_count(),
        2,
        "Cleanup after-hook should fire for each idle instance"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Pool fires Cleanup hooks when expired entries are evicted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_fires_cleanup_hooks_on_expired_entry_eviction() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let mut cfg = pool_config();
    cfg.lifetime.idle_timeout = Duration::from_millis(30);

    let pool = Pool::with_hooks(SimpleResource, TestConfig, cfg, None, Some(registry)).unwrap();

    // Acquire, release, then wait for expiration.
    {
        let (g, _) = pool.acquire(&ctx()).await.unwrap();
        drop(g);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(hook.create_before_count(), 1);
    assert_eq!(hook.cleanup_before_count(), 0, "no cleanup yet");

    // Wait for the entry to expire (idle_timeout = 30ms).
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Next acquire will find expired entry, clean it up, and create a new one.
    let (g, _) = pool.acquire(&ctx()).await.unwrap();

    assert_eq!(
        hook.cleanup_before_count(),
        1,
        "Cleanup hook should fire for expired entry"
    );
    assert_eq!(hook.cleanup_after_count(), 1);

    // A new instance was created to replace the expired one.
    assert_eq!(
        hook.create_before_count(),
        2,
        "second Create for replacement"
    );
    assert_eq!(hook.create_after_count(), 2);

    drop(g);
    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 4: Create before-hook cancellation prevents resource creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_before_hook_cancellation_prevents_creation() {
    let counter = Arc::new(CountingHook::new("counter"));
    let canceller = Arc::new(CancelCreateHook);
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&counter) as Arc<dyn ResourceHook>);
    registry.register(Arc::clone(&canceller) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        SimpleResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // Acquire should fail because CancelCreateHook cancels the Create event.
    let result = pool.acquire(&ctx()).await;
    assert!(
        result.is_err(),
        "acquire should fail due to cancelled create"
    );

    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("cancelled by hook") || msg.contains("Create cancelled"),
        "error should indicate hook cancellation, got: {msg}"
    );

    // The Create after-hook should NOT fire because the operation was cancelled.
    // The counter hook's before fires first (priority 50 < 100), then the
    // CancelCreateHook fires and cancels. So counter sees 1 before, 0 after.
    // Actually, priority: CountingHook=50, CancelCreateHook=100 (default).
    // CountingHook runs first, CancelCreateHook cancels.
    assert_eq!(counter.create_before_count(), 1, "counter before ran first");
    // After-hook is only called on the success/failure of the actual operation,
    // not when a before-hook cancels. Since the cancel short-circuits run_before,
    // create() is never called, so run_after is never reached.

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 5: Cleanup hooks fire on scale_down
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cleanup_hooks_fire_on_scale_down() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        SimpleResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // Scale up to create idle instances.
    let created = pool.scale_up(3).await;
    assert_eq!(created, 3);

    assert_eq!(hook.create_before_count(), 3, "3 creates from scale_up");
    assert_eq!(hook.create_after_count(), 3);

    // Scale down — should fire Cleanup hooks.
    let removed = pool.scale_down(2).await;
    assert_eq!(removed, 2);

    assert_eq!(
        hook.cleanup_before_count(),
        2,
        "Cleanup hooks should fire for scaled-down instances"
    );
    assert_eq!(hook.cleanup_after_count(), 2);

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 6: Cleanup hooks fire on maintain() eviction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cleanup_hooks_fire_on_maintain_eviction() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let mut cfg = pool_config();
    cfg.lifetime.idle_timeout = Duration::from_millis(30);
    cfg.sizing.min_size = 0;

    let pool = Pool::with_hooks(SimpleResource, TestConfig, cfg, None, Some(registry)).unwrap();

    // Create some idle instances.
    let created = pool.scale_up(2).await;
    assert_eq!(created, 2);

    // Wait for idle timeout.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Run maintenance — should evict expired entries.
    pool.maintain(&ctx()).await.unwrap();

    assert!(
        hook.cleanup_before_count() >= 2,
        "Cleanup hooks should fire for evicted entries, got {}",
        hook.cleanup_before_count()
    );
    assert!(
        hook.cleanup_after_count() >= 2,
        "Cleanup after-hooks should fire, got {}",
        hook.cleanup_after_count()
    );

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 7: Cleanup hooks fire when recycle fails on return_instance
// ---------------------------------------------------------------------------

struct RecycleFailResource;

impl Resource for RecycleFailResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("recycle-fail")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("recycle-fail-inst".to_string())
    }

    async fn recycle(&self, _instance: &mut String, _meta: &nebula_resource::pool::InstanceMetadata) -> Result<()> {
        Err(nebula_resource::error::Error::Internal {
            resource_key: resource_key!("recycle-fail"),
            message: "recycle always fails".to_string(),
            source: None,
        })
    }
}

#[tokio::test]
async fn cleanup_hooks_fire_on_recycle_failure() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        RecycleFailResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // Acquire an instance.
    let (guard, _) = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(hook.create_before_count(), 1);
    assert_eq!(hook.create_after_count(), 1);

    // Drop the guard — recycle fails, so the instance is cleaned up.
    drop(guard);

    // Give the spawned return_instance task time to complete.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        hook.cleanup_before_count(),
        1,
        "Cleanup hooks should fire when recycle fails"
    );
    assert_eq!(hook.cleanup_after_count(), 1);

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 8: Hooks fire through Manager registration path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_pools_have_hooks_wired() {
    let manager = nebula_resource::Manager::new();

    let hook = Arc::new(CountingHook::new("manager-hook"));
    manager
        .hooks()
        .register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register should succeed");

    let ctx = ctx();

    // Acquire — should trigger Create hooks (cold pool) and Acquire hooks.
    let resource_key = resource_key!("simple");
    let guard = manager
        .acquire(&resource_key, &ctx)
        .await
        .expect("acquire should succeed");

    assert_eq!(
        hook.create_before_count(),
        1,
        "Create before-hook should fire through Manager"
    );
    assert_eq!(
        hook.create_after_count(),
        1,
        "Create after-hook should fire through Manager"
    );

    // Drop the guard to trigger release.
    drop(guard);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Shutdown — triggers cleanup hooks for idle instances.
    manager.shutdown().await.unwrap();

    assert!(
        hook.cleanup_before_count() >= 1,
        "Cleanup hooks should fire on Manager shutdown, got {}",
        hook.cleanup_before_count()
    );
    assert!(
        hook.cleanup_after_count() >= 1,
        "Cleanup after-hooks should fire on Manager shutdown, got {}",
        hook.cleanup_after_count()
    );
}

// ---------------------------------------------------------------------------
// Test 9: Create hooks fire for each new instance (not cached)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_hooks_fire_per_instance_creation() {
    let hook = Arc::new(CountingHook::new("counter"));
    let registry = Arc::new(HookRegistry::new());
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let pool = Pool::with_hooks(
        SimpleResource,
        TestConfig,
        pool_config(),
        None,
        Some(registry),
    )
    .unwrap();

    // Acquire 3 instances in sequence (each requires a new create).
    let (g1, _) = pool.acquire(&ctx()).await.unwrap();
    let (g2, _) = pool.acquire(&ctx()).await.unwrap();
    let (g3, _) = pool.acquire(&ctx()).await.unwrap();

    assert_eq!(hook.create_before_count(), 3, "3 creates, 3 before-hooks");
    assert_eq!(hook.create_after_count(), 3, "3 creates, 3 after-hooks");

    // Release all.
    drop(g1);
    drop(g2);
    drop(g3);
    // Give the spawned return_instance tasks time to complete
    // and push instances back into the idle queue.
    // idle_timeout is 600s so entries won't expire during this wait.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stats = pool.stats();
    assert_eq!(
        stats.idle, 3,
        "all 3 instances should be back in idle queue before re-acquire"
    );

    // Re-acquire — instances are recycled from idle, no create hooks should fire.
    let before_count = hook.create_before_count();
    let (_g, _) = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(
        hook.create_before_count(),
        before_count,
        "No Create hook when reusing idle instance"
    );

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 10: Pool without hooks still works (no panics)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_without_hooks_works_normally() {
    let pool = Pool::new(SimpleResource, TestConfig, pool_config()).unwrap();

    let (guard, _) = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(*guard, "instance");
    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    pool.shutdown().await.unwrap();
}


