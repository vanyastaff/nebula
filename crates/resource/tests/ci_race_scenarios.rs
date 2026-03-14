//! CI race scenario tests for concurrent acquire/cancel/release behavior.
//!
//! Covers RSC-T011: ensure no leaked permits/instances under concurrent ops.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct RaceResource {
    create_count: Arc<AtomicUsize>,
}

impl RaceResource {
    fn new() -> (Self, Arc<AtomicUsize>) {
        let create_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                create_count: Arc::clone(&create_count),
            },
            create_count,
        )
    }
}

impl Resource for RaceResource {
    type Config = TestConfig;
    type Instance = usize;

    fn metadata(&self) -> nebula_resource::ResourceMetadata {
        nebula_resource::ResourceMetadata::from_key(
            ResourceKey::try_from("ci-race").expect("valid resource key"),
        )
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(self.create_count.fetch_add(1, Ordering::SeqCst))
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

#[expect(
    clippy::excessive_nesting,
    reason = "tokio::spawn inside concurrent test naturally requires this depth"
)]
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_cancelled_acquires_do_not_leak_permits() {
    let (resource, _create_count) = RaceResource::new();
    let max_size = 3usize;
    let pool = Arc::new(
        Pool::new(
            resource,
            TestConfig,
            PoolConfig {
                min_size: 0,
                max_size,
                acquire_timeout: Duration::from_millis(200),
                ..Default::default()
            },
        )
        .expect("pool created"),
    );

    let mut workers = Vec::new();
    for i in 0..60usize {
        let pool = Arc::clone(&pool);
        workers.push(tokio::spawn(async move {
            if i % 2 == 0 {
                if let Ok((guard, _)) = pool.acquire(&ctx()).await {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    drop(guard);
                }
            } else {
                let token = CancellationToken::new();
                let cancelled_ctx = ctx().with_cancellation(token.child_token());
                token.cancel();
                let _ = pool.acquire(&cancelled_ctx).await;
            }
        }));
    }

    for worker in workers {
        worker.await.expect("worker should not panic");
    }

    // If permits leaked, we could not hold max_size concurrent guards here.
    let mut guards = Vec::new();
    for _ in 0..max_size {
        let (guard, _) = pool
            .acquire(&ctx())
            .await
            .expect("permit should be available");
        guards.push(guard);
    }
    assert_eq!(guards.len(), max_size);
}

#[expect(
    clippy::excessive_nesting,
    reason = "tokio::spawn inside concurrent test naturally requires this depth"
)]
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_acquire_release_cycles_preserve_pool_invariants() {
    let (resource, _create_count) = RaceResource::new();
    let max_size = 5usize;
    let pool = Arc::new(
        Pool::new(
            resource,
            TestConfig,
            PoolConfig {
                min_size: 0,
                max_size,
                acquire_timeout: Duration::from_millis(250),
                ..Default::default()
            },
        )
        .expect("pool created"),
    );

    let mut workers = Vec::new();
    for _ in 0..24 {
        let pool = Arc::clone(&pool);
        workers.push(tokio::spawn(async move {
            for _ in 0..30 {
                if let Ok((guard, _)) = pool.acquire(&ctx()).await {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    drop(guard);
                }
            }
        }));
    }

    for worker in workers {
        worker.await.expect("worker should not panic");
    }

    let stats = pool.stats();
    assert!(
        stats.active + stats.idle <= max_size,
        "pool invariant violated: active({}) + idle({}) > max_size({max_size})",
        stats.active,
        stats.idle
    );
}
