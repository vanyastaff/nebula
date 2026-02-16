//! T065-T068: Hook system integration tests.
//!
//! T065: Hooks execute in priority order.
//! T066: Before-hook returning Cancel stops the operation.
//! T067: HookFilter scopes hooks to specific resources.
//! T068: After-hook errors don't affect the operation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::hooks::{HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook};
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

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ---------------------------------------------------------------------------
// Hook implementations for testing
// ---------------------------------------------------------------------------

/// Hook that records its invocation order into a shared vector.
struct OrderTracker {
    order: Arc<parking_lot::Mutex<Vec<String>>>,
    name: String,
    prio: u32,
}

impl ResourceHook for OrderTracker {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.prio
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Acquire]
    }

    fn before<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async {
            self.order.lock().push(format!("before:{}", self.name));
            HookResult::Continue
        })
    }

    fn after<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
        _success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {
            self.order.lock().push(format!("after:{}", self.name));
        })
    }
}

/// Hook that cancels the operation.
struct CancelHook {
    reason: String,
}

impl ResourceHook for CancelHook {
    fn name(&self) -> &str {
        "canceller"
    }

    fn priority(&self) -> u32 {
        10
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Acquire]
    }

    fn before<'a>(
        &'a self,
        _event: &'a HookEvent,
        resource_id: &'a str,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async move {
            HookResult::Cancel(Error::Unavailable {
                resource_id: resource_id.to_string(),
                reason: self.reason.clone(),
                retryable: false,
            })
        })
    }

    fn after<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
        _success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Hook with a specific resource filter.
struct FilteredHook {
    call_count: AtomicU32,
    filter: HookFilter,
    hook_name: String,
}

impl ResourceHook for FilteredHook {
    fn name(&self) -> &str {
        &self.hook_name
    }

    fn priority(&self) -> u32 {
        50
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Acquire]
    }

    fn filter(&self) -> HookFilter {
        self.filter.clone()
    }

    fn before<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        })
    }

    fn after<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
        _success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Hook that always panics in after (simulating an error).
/// Since after-hooks cannot return errors, we use a counter to verify
/// execution and test that the hook system calls all after-hooks
/// without propagating failures.
struct FailingAfterHook {
    before_count: AtomicU32,
    after_count: AtomicU32,
}

impl ResourceHook for FailingAfterHook {
    fn name(&self) -> &str {
        "failing-after"
    }

    fn priority(&self) -> u32 {
        50
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Acquire]
    }

    fn before<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async {
            self.before_count.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        })
    }

    fn after<'a>(
        &'a self,
        _event: &'a HookEvent,
        _resource_id: &'a str,
        _ctx: &'a Context,
        _success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {
            self.after_count.fetch_add(1, Ordering::SeqCst);
            // The after-hook signature returns (), so it cannot propagate
            // errors. This test verifies the contract: after-hooks are
            // called but cannot affect the outcome.
        })
    }
}

// ---------------------------------------------------------------------------
// T065: Hooks execute in priority order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hooks_execute_in_priority_order_registry() {
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let registry = HookRegistry::new();

    // Register hooks with different priorities (out of order)
    registry.register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "high".into(),
        prio: 200,
    }));
    registry.register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "low".into(),
        prio: 10,
    }));
    registry.register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "mid".into(),
        prio: 100,
    }));

    let test_ctx = ctx();

    // Run before-hooks
    registry
        .run_before(&HookEvent::Acquire, "test", &test_ctx)
        .await
        .unwrap();

    // Run after-hooks
    registry
        .run_after(&HookEvent::Acquire, "test", &test_ctx, true)
        .await;

    let recorded = order.lock().clone();
    assert_eq!(
        recorded,
        vec![
            "before:low",
            "before:mid",
            "before:high",
            "after:low",
            "after:mid",
            "after:high",
        ],
        "hooks should execute in priority order (low number first)"
    );
}

#[tokio::test]
async fn hooks_execute_in_priority_order_via_manager() {
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mgr = Manager::new();

    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.hooks().register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "second".into(),
        prio: 50,
    }));
    mgr.hooks().register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "first".into(),
        prio: 10,
    }));

    let _guard = mgr.acquire("db", &ctx()).await.unwrap();

    let recorded = order.lock().clone();
    // Before-hooks run in priority order, then after-hooks in priority order
    assert_eq!(recorded[0], "before:first");
    assert_eq!(recorded[1], "before:second");
    assert_eq!(recorded[2], "after:first");
    assert_eq!(recorded[3], "after:second");
}

// ---------------------------------------------------------------------------
// T066: Before-hook returning Cancel stops the operation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn before_hook_cancel_stops_operation_registry() {
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let registry = HookRegistry::new();

    // Cancel hook at priority 10
    registry.register(Arc::new(CancelHook {
        reason: "blocked by policy".into(),
    }));

    // Tracker hook at priority 100 (higher, should NOT run)
    registry.register(Arc::new(OrderTracker {
        order: order.clone(),
        name: "tracker".into(),
        prio: 100,
    }));

    let test_ctx = ctx();
    let result = registry
        .run_before(&HookEvent::Acquire, "test-res", &test_ctx)
        .await;

    assert!(result.is_err(), "cancel hook should cause error");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("blocked by policy"),
        "error should contain the cancel reason, got: {err}"
    );

    let recorded = order.lock().clone();
    assert!(
        recorded.is_empty(),
        "tracker hook should not have run after cancel, got: {recorded:?}"
    );
}

#[tokio::test]
async fn before_hook_cancel_stops_manager_acquire() {
    let mgr = Manager::new();
    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();

    mgr.hooks().register(Arc::new(CancelHook {
        reason: "rate limited".into(),
    }));

    let result = mgr.acquire("db", &ctx()).await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("rate limited"),
        "manager acquire should propagate cancel hook error"
    );
}

// ---------------------------------------------------------------------------
// T067: HookFilter scopes hooks to specific resources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_filter_limits_to_specific_resource_registry() {
    let registry = HookRegistry::new();

    let targeted_hook = Arc::new(FilteredHook {
        call_count: AtomicU32::new(0),
        filter: HookFilter::Resource("db".to_string()),
        hook_name: "db-only".into(),
    });
    registry.register(Arc::clone(&targeted_hook) as Arc<dyn ResourceHook>);

    let test_ctx = ctx();

    // Run for "db" -- should trigger
    registry
        .run_before(&HookEvent::Acquire, "db", &test_ctx)
        .await
        .unwrap();
    assert_eq!(
        targeted_hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should run for matching resource"
    );

    // Run for "cache" -- should NOT trigger
    registry
        .run_before(&HookEvent::Acquire, "cache", &test_ctx)
        .await
        .unwrap();
    assert_eq!(
        targeted_hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should not run for non-matching resource"
    );
}

#[tokio::test]
async fn hook_filter_prefix_matches_multiple_resources() {
    let registry = HookRegistry::new();

    let prefix_hook = Arc::new(FilteredHook {
        call_count: AtomicU32::new(0),
        filter: HookFilter::Prefix("db-".to_string()),
        hook_name: "db-prefix".into(),
    });
    registry.register(Arc::clone(&prefix_hook) as Arc<dyn ResourceHook>);

    let test_ctx = ctx();

    // "db-primary" matches prefix "db-"
    registry
        .run_before(&HookEvent::Acquire, "db-primary", &test_ctx)
        .await
        .unwrap();
    assert_eq!(prefix_hook.call_count.load(Ordering::SeqCst), 1);

    // "db-replica" also matches
    registry
        .run_before(&HookEvent::Acquire, "db-replica", &test_ctx)
        .await
        .unwrap();
    assert_eq!(prefix_hook.call_count.load(Ordering::SeqCst), 2);

    // "cache-primary" does NOT match
    registry
        .run_before(&HookEvent::Acquire, "cache-primary", &test_ctx)
        .await
        .unwrap();
    assert_eq!(
        prefix_hook.call_count.load(Ordering::SeqCst),
        2,
        "non-matching prefix should not trigger hook"
    );
}

#[tokio::test]
async fn hook_filter_via_manager_only_fires_for_matching_resource() {
    let mgr = Manager::new();

    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();
    mgr.register(NamedResource { name: "cache" }, TestConfig, pool_cfg())
        .unwrap();

    let db_hook = Arc::new(FilteredHook {
        call_count: AtomicU32::new(0),
        filter: HookFilter::Resource("db".to_string()),
        hook_name: "db-only".into(),
    });
    mgr.hooks()
        .register(Arc::clone(&db_hook) as Arc<dyn ResourceHook>);

    // Acquire "db" -- hook fires
    let g1 = mgr.acquire("db", &ctx()).await.unwrap();
    drop(g1);
    tokio::time::sleep(Duration::from_millis(30)).await;

    assert_eq!(
        db_hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should fire for db"
    );

    // Acquire "cache" -- hook does NOT fire
    let g2 = mgr.acquire("cache", &ctx()).await.unwrap();
    drop(g2);
    tokio::time::sleep(Duration::from_millis(30)).await;

    assert_eq!(
        db_hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should not fire for cache"
    );
}

// ---------------------------------------------------------------------------
// T068: After-hook errors don't affect the operation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn after_hook_does_not_affect_operation_registry() {
    let registry = HookRegistry::new();

    let hook = Arc::new(FailingAfterHook {
        before_count: AtomicU32::new(0),
        after_count: AtomicU32::new(0),
    });
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let test_ctx = ctx();

    // Before-hook should pass
    let result = registry
        .run_before(&HookEvent::Acquire, "test", &test_ctx)
        .await;
    assert!(result.is_ok());
    assert_eq!(hook.before_count.load(Ordering::SeqCst), 1);

    // After-hook runs but cannot propagate errors
    registry
        .run_after(&HookEvent::Acquire, "test", &test_ctx, true)
        .await;
    assert_eq!(
        hook.after_count.load(Ordering::SeqCst),
        1,
        "after-hook should have been called"
    );
    // The fact that we reach this point proves after-hooks don't fail the caller
}

#[tokio::test]
async fn after_hook_does_not_affect_manager_acquire() {
    let mgr = Manager::new();

    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();

    let hook = Arc::new(FailingAfterHook {
        before_count: AtomicU32::new(0),
        after_count: AtomicU32::new(0),
    });
    mgr.hooks()
        .register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    // Acquire should succeed despite the after-hook
    let guard = mgr.acquire("db", &ctx()).await;
    assert!(
        guard.is_ok(),
        "acquire should succeed despite after-hook, got: {:?}",
        guard.err()
    );
    assert_eq!(hook.before_count.load(Ordering::SeqCst), 1);
    assert_eq!(hook.after_count.load(Ordering::SeqCst), 1);

    // Verify the returned guard has the right value
    let val = guard
        .unwrap()
        .as_any()
        .downcast_ref::<String>()
        .unwrap()
        .clone();
    assert_eq!(val, "db-instance");
}

// ---------------------------------------------------------------------------
// Hook event filtering: hooks only fire for subscribed events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_only_fires_for_subscribed_events() {
    let registry = HookRegistry::new();

    // This hook only subscribes to Acquire
    let hook = Arc::new(FilteredHook {
        call_count: AtomicU32::new(0),
        filter: HookFilter::All,
        hook_name: "acquire-only".into(),
    });
    registry.register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    let test_ctx = ctx();

    // Acquire event -- should fire
    registry
        .run_before(&HookEvent::Acquire, "test", &test_ctx)
        .await
        .unwrap();
    assert_eq!(hook.call_count.load(Ordering::SeqCst), 1);

    // Release event -- should NOT fire (hook only subscribes to Acquire)
    registry
        .run_before(&HookEvent::Release, "test", &test_ctx)
        .await
        .unwrap();
    assert_eq!(
        hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should not fire for unsubscribed Release event"
    );

    // Create event -- should NOT fire
    registry
        .run_before(&HookEvent::Create, "test", &test_ctx)
        .await
        .unwrap();
    assert_eq!(
        hook.call_count.load(Ordering::SeqCst),
        1,
        "hook should not fire for unsubscribed Create event"
    );
}
