//! Stress tests for shutdown behavior under in-flight load.
//!
//! Covers RSC-T008.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, Manager, PoolAcquire, PoolSizing, ShutdownConfig, WorkflowId};

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct StressResource {
    created: Arc<AtomicUsize>,
    cleaned: Arc<AtomicUsize>,
}

impl StressResource {
    fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let created = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        (
            Self {
                created: Arc::clone(&created),
                cleaned: Arc::clone(&cleaned),
            },
            created,
            cleaned,
        )
    }
}

impl Resource for StressResource {
    type Config = TestConfig;
    type Instance = usize;

    fn key(&self) -> ResourceKey {
        resource_key!("shutdown-stress")
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(self.created.fetch_add(1, Ordering::SeqCst))
    }

    async fn destroy(&self, _instance: Self::Instance) -> Result<()> {
        self.cleaned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

#[expect(
    clippy::excessive_nesting,
    reason = "tokio::spawn inside loop with match in shutdown stress test naturally requires this depth"
)]
#[tokio::test(flavor = "multi_thread")]
async fn manager_shutdown_phased_completes_under_inflight_load() {
    let (resource, created, cleaned) = StressResource::new();
    let manager = Arc::new(Manager::new());
    manager
        .register(
            resource,
            TestConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 8,
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_millis(100),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("resource registered");

    let key = resource_key!("shutdown-stress");
    let stop = Arc::new(AtomicBool::new(false));

    let mut workers = Vec::new();
    for _ in 0..24 {
        let manager = Arc::clone(&manager);
        let key = key.clone();
        let stop = Arc::clone(&stop);
        workers.push(tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                match manager.acquire(&key, &ctx()).await {
                    Ok(guard) => {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        drop(guard);
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            }
        }));
    }

    tokio::time::sleep(Duration::from_millis(120)).await;

    manager
        .shutdown_phased(ShutdownConfig {
            drain_timeout: Duration::from_millis(250),
            cleanup_timeout: Duration::from_millis(250),
            terminate_timeout: Duration::from_millis(100),
        })
        .await
        .expect("shutdown must complete under load");

    stop.store(true, Ordering::SeqCst);
    for worker in workers {
        worker.await.expect("worker task should not panic");
    }

    let err = manager
        .acquire(&key, &ctx())
        .await
        .expect_err("acquire after shutdown must fail");
    assert!(!err.is_retryable());

    tokio::time::sleep(Duration::from_millis(50)).await;
    let created_count = created.load(Ordering::SeqCst);
    let cleaned_count = cleaned.load(Ordering::SeqCst);
    assert!(
        created_count > 0,
        "stress run should create at least one instance"
    );
    assert!(
        cleaned_count > 0,
        "shutdown should clean at least one instance"
    );
    assert!(
        cleaned_count <= created_count,
        "cleanup count cannot exceed created count ({cleaned_count} > {created_count})"
    );
}

#[expect(
    clippy::excessive_nesting,
    reason = "tokio::spawn inside loop with match in pool shutdown test naturally requires this depth"
)]
#[tokio::test(flavor = "multi_thread")]
async fn pool_shutdown_does_not_hang_with_concurrent_acquires() {
    let (resource, _created, cleaned) = StressResource::new();
    let pool = Arc::new(
        Pool::new(
            resource,
            TestConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 4,
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_millis(60),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("pool created"),
    );

    let mut workers = Vec::new();
    for _ in 0..16 {
        let pool = Arc::clone(&pool);
        workers.push(tokio::spawn(async move {
            for _ in 0..40 {
                match pool.acquire(&ctx()).await {
                    Ok((guard, _)) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        drop(guard);
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    tokio::time::timeout(Duration::from_secs(2), pool.shutdown())
        .await
        .expect("shutdown should not hang")
        .expect("shutdown should succeed");

    for worker in workers {
        worker.await.expect("worker task should not panic");
    }

    let stats = pool.stats();
    assert_eq!(stats.active, 0, "no active instances after shutdown");
    assert_eq!(stats.idle, 0, "no idle instances after shutdown");
    assert!(
        cleaned.load(Ordering::SeqCst) > 0,
        "shutdown should perform cleanup"
    );
}
