//! Pool exhaustion and recovery tests

use std::sync::Arc;
use std::time::Duration;

use nebula_resource::ResourceContext;
use nebula_resource::core::error::ResourceError;
use nebula_resource::core::resource::{
    ResourceId, ResourceInstanceMetadata, TypedResourceInstance,
};
use nebula_resource::core::traits::PoolConfig;
use nebula_resource::pool::{PoolStrategy, ResourcePool};

async fn create_test_instance()
-> Result<TypedResourceInstance<String>, nebula_resource::ResourceError> {
    let metadata = ResourceInstanceMetadata {
        instance_id: uuid::Uuid::new_v4(),
        resource_id: ResourceId::new("test", "1.0"),
        state: nebula_resource::LifecycleState::Ready,
        context: ResourceContext::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        ),
        created_at: chrono::Utc::now(),
        last_accessed_at: None,
        tags: std::collections::HashMap::new(),
    };

    Ok(TypedResourceInstance::new(
        Arc::new("test_resource".to_string()),
        metadata,
    ))
}

#[tokio::test]
async fn pool_exhaustion_returns_error() {
    let config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(600),
        max_lifetime: Duration::from_secs(3600),
        validation_interval: Duration::from_secs(30),
    };
    let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

    // Acquire 2 resources (should succeed)
    let r1 = pool.acquire().await.expect("first acquire should succeed");
    let r2 = pool.acquire().await.expect("second acquire should succeed");

    assert_eq!(pool.stats().active_count, 2);

    // Third acquire should fail with PoolExhausted
    let result = pool.acquire().await;
    assert!(result.is_err(), "third acquire should fail");

    let err = result.unwrap_err();
    assert!(
        matches!(err, ResourceError::PoolExhausted { max_size: 2, .. }),
        "expected PoolExhausted, got: {:?}",
        err
    );

    // Release one resource
    let id1 = r1.instance_id();
    drop(r1);
    pool.release(id1).await.unwrap();

    // Now acquire should succeed again
    let r3 = pool
        .acquire()
        .await
        .expect("acquire after release should succeed");

    assert_eq!(pool.stats().active_count, 2);

    // Cleanup
    drop(r2);
    drop(r3);
}

#[tokio::test]
async fn pool_exhaustion_stats_track_failures() {
    let config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(600),
        max_lifetime: Duration::from_secs(3600),
        validation_interval: Duration::from_secs(30),
    };
    let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

    let r1 = pool.acquire().await.unwrap();

    // Multiple failed attempts
    for _ in 0..3 {
        let _ = pool.acquire().await;
    }

    let stats = pool.stats();
    assert_eq!(stats.failed_acquisitions, 3);
    assert_eq!(stats.total_acquisitions, 4); // 1 success + 3 failures

    drop(r1);
}

#[tokio::test]
async fn pool_drop_returns_resource_for_reuse() {
    let config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(600),
        max_lifetime: Duration::from_secs(3600),
        validation_interval: Duration::from_secs(30),
    };
    let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

    // Acquire and drop (Drop sends resource back via channel)
    let r1 = pool.acquire().await.unwrap();
    drop(r1);

    // Next acquire triggers drain_returned, should reuse the dropped resource
    let r2 = pool.acquire().await.expect("should reuse dropped resource");

    let stats = pool.stats();
    // Only 1 resource should have been created (the first one, reused after drop)
    assert_eq!(stats.resources_created, 1);
    assert_eq!(stats.total_acquisitions, 2);

    drop(r2);
}

#[tokio::test]
async fn pool_exhausted_error_is_retryable() {
    let config = PoolConfig {
        min_size: 0,
        max_size: 1,
        ..PoolConfig::default()
    };
    let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

    let _r1 = pool.acquire().await.unwrap();
    let err = pool.acquire().await.unwrap_err();

    assert!(err.is_retryable(), "PoolExhausted should be retryable");
}
