// Reference Resource integration test.
//
// Defines a complete Resource implementation exercising every lifecycle
// method to serve as a template for resource authors.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Reference Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ReferenceConfig {
    prefix: String,
    initial_value: u64,
}

impl Config for ReferenceConfig {
    fn validate(&self) -> Result<()> {
        if self.prefix.is_empty() {
            return Err(Error::configuration("prefix must not be empty"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Reference Instance
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ReferenceInstance {
    id: String,
    counter: u64,
    valid: bool,
}

// ---------------------------------------------------------------------------
// Reference Resource
// ---------------------------------------------------------------------------

struct ReferenceResource {
    create_count: AtomicU64,
    recycle_count: AtomicU64,
    cleanup_called: AtomicBool,
}

impl ReferenceResource {
    fn new() -> Self {
        Self {
            create_count: AtomicU64::new(0),
            recycle_count: AtomicU64::new(0),
            cleanup_called: AtomicBool::new(false),
        }
    }
}

impl Resource for ReferenceResource {
    type Config = ReferenceConfig;
    type Instance = ReferenceInstance;

    fn id(&self) -> &str {
        "reference-resource"
    }

    async fn create(&self, config: &ReferenceConfig, _ctx: &Context) -> Result<ReferenceInstance> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(ReferenceInstance {
            id: format!("{}-{n}", config.prefix),
            counter: config.initial_value,
            valid: true,
        })
    }

    async fn is_valid(&self, instance: &ReferenceInstance) -> Result<bool> {
        Ok(instance.valid)
    }

    async fn recycle(&self, instance: &mut ReferenceInstance) -> Result<()> {
        self.recycle_count.fetch_add(1, Ordering::SeqCst);
        // Reset counter on recycle to simulate connection state reset.
        instance.counter = 0;
        Ok(())
    }

    async fn cleanup(&self, _instance: ReferenceInstance) -> Result<()> {
        self.cleanup_called.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn dependencies(&self) -> Vec<&str> {
        vec!["config-store"]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn ctx() -> Context {
    Context::new(Scope::Global, "test-wf", "test-ex")
}

fn config() -> ReferenceConfig {
    ReferenceConfig {
        prefix: "ref".into(),
        initial_value: 42,
    }
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

#[tokio::test]
async fn create_produces_valid_instance() {
    let resource = ReferenceResource::new();
    let instance = resource.create(&config(), &ctx()).await.unwrap();

    assert_eq!(instance.id, "ref-0");
    assert_eq!(instance.counter, 42);
    assert!(instance.valid);
    assert_eq!(resource.create_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn is_valid_reflects_instance_state() {
    let resource = ReferenceResource::new();

    let valid_instance = ReferenceInstance {
        id: "test".into(),
        counter: 0,
        valid: true,
    };
    assert!(resource.is_valid(&valid_instance).await.unwrap());

    let invalid_instance = ReferenceInstance {
        id: "test".into(),
        counter: 0,
        valid: false,
    };
    assert!(!resource.is_valid(&invalid_instance).await.unwrap());
}

#[tokio::test]
async fn recycle_resets_instance_state() {
    let resource = ReferenceResource::new();
    let mut instance = resource.create(&config(), &ctx()).await.unwrap();

    assert_eq!(instance.counter, 42);

    resource.recycle(&mut instance).await.unwrap();

    assert_eq!(instance.counter, 0, "recycle should reset counter");
    assert_eq!(resource.recycle_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cleanup_is_called_on_shutdown() {
    let cleanup_called = Arc::new(AtomicBool::new(false));

    // We cannot share the Arc<ReferenceResource> with Pool directly since
    // Pool takes ownership. Instead, build a wrapper that tracks cleanup.
    struct CleanupTracker {
        inner: ReferenceResource,
        called: Arc<AtomicBool>,
    }

    impl Resource for CleanupTracker {
        type Config = ReferenceConfig;
        type Instance = ReferenceInstance;

        fn id(&self) -> &str {
            self.inner.id()
        }

        async fn create(
            &self,
            config: &ReferenceConfig,
            ctx: &Context,
        ) -> Result<ReferenceInstance> {
            self.inner.create(config, ctx).await
        }

        async fn is_valid(&self, instance: &ReferenceInstance) -> Result<bool> {
            self.inner.is_valid(instance).await
        }

        async fn recycle(&self, instance: &mut ReferenceInstance) -> Result<()> {
            self.inner.recycle(instance).await
        }

        async fn cleanup(&self, instance: ReferenceInstance) -> Result<()> {
            self.called.store(true, Ordering::SeqCst);
            self.inner.cleanup(instance).await
        }
    }

    let tracker = CleanupTracker {
        inner: ReferenceResource::new(),
        called: Arc::clone(&cleanup_called),
    };

    let pool = Pool::new(tracker, config(), pool_config()).unwrap();

    // Acquire and return so there is an idle instance to clean up.
    {
        let _guard = pool.acquire(&ctx()).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    pool.shutdown().await.unwrap();

    assert!(
        cleanup_called.load(Ordering::SeqCst),
        "cleanup should be called during pool shutdown"
    );
}

#[tokio::test]
async fn dependencies_are_reported() {
    let resource = ReferenceResource::new();
    let deps = resource.dependencies();

    assert_eq!(deps, vec!["config-store"]);
}

#[tokio::test]
async fn config_validation_rejects_empty_prefix() {
    let bad_config = ReferenceConfig {
        prefix: String::new(),
        initial_value: 0,
    };
    let result = bad_config.validate();
    assert!(result.is_err());
}

#[tokio::test]
async fn full_lifecycle_through_pool() {
    let pool = Pool::new(ReferenceResource::new(), config(), pool_config()).unwrap();

    // Acquire
    let guard = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(guard.id, "ref-0");
    assert_eq!(guard.counter, 42);

    // Release (drop guard)
    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Re-acquire: should get recycled instance with counter reset to 0.
    let guard2 = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(
        guard2.counter, 0,
        "recycled instance should have counter reset"
    );

    drop(guard2);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Shutdown
    pool.shutdown().await.unwrap();
    let stats = pool.stats();
    assert_eq!(stats.idle, 0);
}

#[tokio::test]
async fn multiple_creates_get_unique_ids() {
    let resource = ReferenceResource::new();
    let i0 = resource.create(&config(), &ctx()).await.unwrap();
    let i1 = resource.create(&config(), &ctx()).await.unwrap();
    let i2 = resource.create(&config(), &ctx()).await.unwrap();

    assert_eq!(i0.id, "ref-0");
    assert_eq!(i1.id, "ref-1");
    assert_eq!(i2.id, "ref-2");
}
