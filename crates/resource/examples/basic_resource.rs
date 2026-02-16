// Minimal resource example: an in-memory cache.
//
// Shows how to implement the Resource trait with a simple HashMap cache,
// then acquire and use it through a Pool.

use std::collections::HashMap;
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// -- Configuration ----------------------------------------------------------

/// Configuration for the in-memory cache resource.
#[derive(Debug, Clone)]
struct CacheConfig {
    /// Maximum number of entries the cache will hold.
    max_entries: usize,
}

impl Config for CacheConfig {
    fn validate(&self) -> Result<()> {
        if self.max_entries == 0 {
            return Err(Error::configuration("max_entries must be > 0"));
        }
        Ok(())
    }
}

// -- Resource ---------------------------------------------------------------

/// A minimal resource that produces HashMap-based caches.
struct InMemoryCache;

impl Resource for InMemoryCache {
    type Config = CacheConfig;
    type Instance = HashMap<String, String>;

    fn id(&self) -> &str {
        "in-memory-cache"
    }

    /// Create a new empty cache.
    async fn create(
        &self,
        config: &CacheConfig,
        _ctx: &Context,
    ) -> Result<HashMap<String, String>> {
        Ok(HashMap::with_capacity(config.max_entries))
    }

    /// A cache is always valid unless it has grown too large.
    async fn is_valid(&self, instance: &HashMap<String, String>) -> Result<bool> {
        // In a real resource you might check connectivity, staleness, etc.
        Ok(instance.len() < 10_000)
    }

    /// Clean up: nothing to do for an in-memory HashMap.
    async fn cleanup(&self, _instance: HashMap<String, String>) -> Result<()> {
        // In a real resource you would close connections, flush buffers, etc.
        Ok(())
    }
}

// -- Main -------------------------------------------------------------------

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("=== Basic Resource Example: InMemoryCache ===\n");

    // 1. Create pool configuration.
    let pool_config = PoolConfig {
        min_size: 1,
        max_size: 4,
        acquire_timeout: Duration::from_secs(5),
        ..Default::default()
    };

    // 2. Build the pool.
    let pool = Pool::new(
        InMemoryCache,
        CacheConfig { max_entries: 1024 },
        pool_config,
    )?;
    println!("Pool created (min=1, max=4)");

    // 3. Acquire a cache instance.
    let ctx = Context::new(Scope::Global, "demo-wf", "demo-ex");
    let mut cache = pool.acquire(&ctx).await?;
    println!("Cache acquired");

    // 4. Use it.
    cache.insert("greeting".into(), "hello, nebula!".into());
    println!("Cached: greeting -> {}", cache.get("greeting").unwrap());

    // 5. Drop the guard to return the cache to the pool.
    drop(cache);
    tokio::time::sleep(Duration::from_millis(20)).await;
    println!("Cache returned to pool (stats: {:?})", pool.stats());

    // 6. Shut down cleanly.
    pool.shutdown().await?;
    println!("Pool shut down");

    Ok(())
}
