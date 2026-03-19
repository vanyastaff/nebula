//! Circuit breaker integration tests for pool create/recycle operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_core::{resource_key, ResourceKey};
use nebula_resilience::CircuitBreakerConfig;
use nebula_resilience::retryable::Retryable;
use nebula_resource::context::Context;
use nebula_resource::error::Error;
use nebula_resource::events::{EventBus, ResourceEvent};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, PoolAcquire, PoolResiliencePolicy, PoolSizing, WorkflowId};

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

struct FailingCreateResource {
    create_calls: AtomicU32,
    fail_until: u32,
}

impl FailingCreateResource {
    fn new(fail_until: u32) -> Self {
        Self {
            create_calls: AtomicU32::new(0),
            fail_until,
        }
    }
}

impl Resource for FailingCreateResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("cb-create")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<String> {
        let call = self.create_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call <= self.fail_until {
            return Err(Error::Initialization {
                resource_key: resource_key!("cb-create"),
                reason: format!("intentional create failure #{call}"),
                source: None,
            });
        }
        Ok(format!("instance-{call}"))
    }
}

#[tokio::test]
async fn create_breaker_opens_and_reports_retryability() {
    let breaker_cfg = CircuitBreakerConfig { min_operations: 5, half_open_max_ops: 1, ..Default::default() };
    let pool = Pool::new(
        FailingCreateResource::new(1000),
        TestConfig,
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 1 },
            acquire: PoolAcquire { timeout: Duration::from_millis(200), ..Default::default() },
            resilience: PoolResiliencePolicy { create_breaker: Some(breaker_cfg), ..Default::default() },
            ..Default::default()
        },
    )
    .expect("pool created");

    for _ in 0..5 {
        let err = pool.acquire(&ctx()).await.expect_err("create should fail");
        assert!(matches!(err, Error::Initialization { .. }));
    }

    let err = pool
        .acquire(&ctx())
        .await
        .expect_err("breaker should be open on subsequent attempt");
    assert!(matches!(
        err,
        Error::CircuitBreakerOpen {
            operation: "create",
            ..
        }
    ));
    assert!(err.is_retryable());
    assert_eq!(Retryable::is_retryable(&err), err.is_retryable());
    assert_eq!(
        Retryable::retry_delay(&err),
        err.retry_after().unwrap_or(Duration::from_millis(100))
    );
}

#[tokio::test]
async fn create_breaker_half_open_probe_then_close_emits_events() {
    let breaker_cfg = CircuitBreakerConfig { min_operations: 5, half_open_max_ops: 1, ..Default::default() };
    let bus = Arc::new(EventBus::new(128));
    let mut sub = bus.subscribe();

    let pool = Pool::with_event_bus(
        FailingCreateResource::new(5),
        TestConfig,
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 1 },
            acquire: PoolAcquire { timeout: Duration::from_millis(200), ..Default::default() },
            resilience: PoolResiliencePolicy { create_breaker: Some(breaker_cfg), ..Default::default() },
            ..Default::default()
        },
        Some(Arc::clone(&bus)),
    )
    .expect("pool created");

    for _ in 0..5 {
        let _ = pool.acquire(&ctx()).await;
    }

    let _ = pool.acquire(&ctx()).await;

    tokio::time::sleep(Duration::from_secs(31)).await;
    let (_g, _) = pool
        .acquire(&ctx())
        .await
        .expect("half-open probe should succeed and close breaker");

    let mut saw_open = false;
    let mut saw_closed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let maybe_event = tokio::time::timeout(Duration::from_millis(200), sub.recv()).await;
        let Ok(Some(event)) = maybe_event else {
            continue;
        };
        match event {
            ResourceEvent::CircuitBreakerOpen { operation, .. } if operation == "create" => {
                saw_open = true;
            }
            ResourceEvent::CircuitBreakerClosed { operation, .. } if operation == "create" => {
                saw_closed = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_open, "expected create breaker open event");
    assert!(saw_closed, "expected create breaker closed event");
}

struct RecycleCountingResource {
    recycle_calls: Arc<AtomicU32>,
}

impl RecycleCountingResource {
    fn new(recycle_calls: Arc<AtomicU32>) -> Self {
        Self { recycle_calls }
    }
}

impl Resource for RecycleCountingResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("cb-recycle")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<String> {
        Ok("instance".to_string())
    }

    async fn recycle(&self, _instance: &mut String) -> nebula_resource::Result<()> {
        self.recycle_calls.fetch_add(1, Ordering::SeqCst);
        Err(Error::Internal {
            resource_key: resource_key!("cb-recycle"),
            message: "recycle failed".to_string(),
            source: None,
        })
    }
}

#[tokio::test]
async fn recycle_breaker_open_skips_recycle_call() {
    let breaker_cfg = CircuitBreakerConfig { min_operations: 1, failure_threshold: 1, ..Default::default() };
    let recycle_calls = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        RecycleCountingResource::new(Arc::clone(&recycle_calls)),
        TestConfig,
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 1 },
            acquire: PoolAcquire { timeout: Duration::from_millis(500), ..Default::default() },
            resilience: PoolResiliencePolicy { recycle_breaker: Some(breaker_cfg), ..Default::default() },
            ..Default::default()
        },
    )
    .expect("pool created");

    {
        let (_g, _) = pool
            .acquire(&ctx())
            .await
            .expect("first acquire should work");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    {
        let (_g, _) = pool
            .acquire(&ctx())
            .await
            .expect("second acquire should work");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        recycle_calls.load(Ordering::SeqCst),
        1,
        "second release should skip recycle when breaker is open"
    );
}

#[tokio::test]
async fn create_and_recycle_breakers_are_independent() {
    // Open the recycle breaker (min_operations=1 → opens after the very first
    // failed recycle). The create_breaker is absent, so the create path is
    // never gated. A subsequent acquire must still succeed.
    let recycle_cfg = CircuitBreakerConfig { min_operations: 1, failure_threshold: 1, ..Default::default() };
    let recycle_calls = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        RecycleCountingResource::new(Arc::clone(&recycle_calls)),
        TestConfig,
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 2 },
            acquire: PoolAcquire { timeout: Duration::from_millis(500), ..Default::default() },
            resilience: PoolResiliencePolicy {
                recycle_breaker: Some(recycle_cfg), // create_breaker intentionally absent — create path must stay open.
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("pool created");

    // First acquire + release: triggers a recycle failure, opening the recycle breaker.
    {
        let (_g, _) = pool.acquire(&ctx()).await.expect("first acquire ok");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Recycle breaker is now open. The create breaker is absent, so a fresh
    // acquire — which must create a new instance — must succeed unconditionally.
    let result = pool.acquire(&ctx()).await;
    assert!(
        result.is_ok(),
        "create path unaffected by open recycle breaker; got: {:?}",
        result.err()
    );
}
