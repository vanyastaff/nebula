//! Pool exhaustion and recovery tests

use std::time::Duration;

use async_trait::async_trait;
use nebula_resource::context::ResourceContext;
use nebula_resource::error::{ResourceError, ResourceResult};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Resource, ResourceConfig};
use nebula_resource::scope::ResourceScope;

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;

impl ResourceConfig for TestConfig {}

struct TestResource;

#[async_trait]
impl Resource for TestResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "test-pool"
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        Ok("pooled-instance".to_string())
    }
}

fn ctx() -> ResourceContext {
    ResourceContext::new(ResourceScope::Global, "wf", "ex")
}

#[tokio::test]
async fn pool_exhaustion_returns_error() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let pool = Pool::new(TestResource, TestConfig, pool_config).unwrap();

    // Acquire 2 resources (should succeed)
    let _r1 = pool
        .acquire(&ctx())
        .await
        .expect("first acquire should succeed");
    let _r2 = pool
        .acquire(&ctx())
        .await
        .expect("second acquire should succeed");

    // Third acquire should fail with PoolExhausted
    let result = pool.acquire(&ctx()).await;
    assert!(result.is_err(), "third acquire should fail");

    let err = result.unwrap_err();
    assert!(
        matches!(err, ResourceError::PoolExhausted { max_size: 2, .. }),
        "expected PoolExhausted, got: {:?}",
        err
    );
}

#[tokio::test]
async fn pool_reuses_after_drop() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(TestResource, TestConfig, pool_config).unwrap();

    // Acquire and drop to return to pool
    {
        let _r1 = pool.acquire(&ctx()).await.unwrap();
    }
    // Give the spawn a moment to return the instance
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should be able to acquire again
    let _r2 = pool.acquire(&ctx()).await.expect("should reuse after drop");

    let stats = pool.stats();
    assert_eq!(stats.total_acquisitions, 2);
}

#[tokio::test]
async fn pool_exhausted_error_is_retryable() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let pool = Pool::new(TestResource, TestConfig, pool_config).unwrap();

    let _r1 = pool.acquire(&ctx()).await.unwrap();
    let err = pool.acquire(&ctx()).await.unwrap_err();

    assert!(err.is_retryable(), "PoolExhausted should be retryable");
}
