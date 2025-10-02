//! Caching Strategies Example
//!
//! This example demonstrates different caching behaviors and strategies:
//! - Cache hits vs misses
//! - Token expiration and refresh
//! - Manual cache invalidation
//! - Performance benefits of caching
//! - Cache statistics and monitoring
//!
//! This example uses MockTokenCache which provides statistics tracking.
//! In production, you would use RedisTokenCache with similar behavior.

use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache, TestCredentialFactory};
use nebula_credential::traits::{StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
use std::sync::Arc;
use std::time::SystemTime;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║     Nebula Credential - Caching Strategies Example      ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup: Create CredentialManager with cache
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setup: Creating CredentialManager with cache");
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let lock = MockLock::new();
    let registry = Arc::new(CredentialRegistry::new());

    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache.clone() as Arc<dyn TokenCache>)
        .with_lock(lock)
        .with_registry(registry)
        .build()?;

    println!("   ✓ CredentialManager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // Strategy 1: First Request (Cache Miss)
    // ═══════════════════════════════════════════════════════════════
    println!("🔍 Strategy 1: First Request (Cache Miss)");
    println!("   Creating credential...");

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "api-key-12345",
                "should_fail": false
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", cred_id);
    println!("   Requesting token for the first time...");

    let start = SystemTime::now();
    let token1 = manager.get_token(&cred_id).await?;
    let elapsed = start.elapsed()?;

    println!("   ✓ Token retrieved in {:?}", elapsed);
    println!("      ├─ This was a CACHE MISS (token not in cache)");
    println!("      ├─ Token was created/refreshed from state store");
    println!("      └─ Token cached for future requests");

    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}", stats.hits, stats.misses, stats.puts);
    println!("      └─ Miss rate: {:.1}%\n", (1.0 - cache.hit_rate()) * 100.0);

    // ═══════════════════════════════════════════════════════════════
    // Strategy 2: Subsequent Requests (Cache Hit)
    // ═══════════════════════════════════════════════════════════════
    println!("⚡ Strategy 2: Subsequent Requests (Cache Hit)");
    println!("   Requesting same token 5 times...");

    for i in 1..=5 {
        let start = SystemTime::now();
        let token = manager.get_token(&cred_id).await?;
        let elapsed = start.elapsed()?;

        println!("   Request #{}: {:?} (same token: {})",
            i, elapsed, token.token.expose() == token1.token.expose());
    }

    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}", stats.hits, stats.misses, stats.puts);
    println!("      └─ Hit rate: {:.1}%\n", cache.hit_rate() * 100.0);

    // ═══════════════════════════════════════════════════════════════
    // Strategy 3: Manual Cache Invalidation
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Strategy 3: Manual Cache Invalidation");
    println!("   Scenario: Force token refresh by clearing cache");

    println!("   ├─ Current cache stats: hits={}, misses={}", stats.hits, stats.misses);
    println!("   ├─ Invalidating cache for credential...");

    cache.del(&cred_id.to_string()).await?;

    println!("   └─ Requesting token (will trigger refresh)...");
    let start = SystemTime::now();
    let token2 = manager.get_token(&cred_id).await?;
    let elapsed = start.elapsed()?;

    println!("   ✓ Token refreshed in {:?}", elapsed);
    println!("      ├─ This was a CACHE MISS (cache was invalidated)");
    println!("      └─ New token: {} (different: {})",
        &token2.token.expose()[..10],
        token1.token.expose() != token2.token.expose());

    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}\n", stats.hits, stats.misses, stats.puts);

    // ═══════════════════════════════════════════════════════════════
    // Strategy 4: Multiple Credentials
    // ═══════════════════════════════════════════════════════════════
    println!("🔑 Strategy 4: Multiple Credentials (Cache Isolation)");
    println!("   Creating 3 additional credentials...");

    let mut cred_ids = vec![cred_id.clone()];
    for i in 2..=4 {
        let id = manager
            .create_credential(
                "test_credential",
                json!({
                    "value": format!("api-key-{}", i),
                    "should_fail": false
                }),
            )
            .await?;
        cred_ids.push(id.clone());
        println!("   ✓ Created credential {}: {}", i, id);
    }

    println!("\n   Getting tokens for all credentials...");
    for (i, id) in cred_ids.iter().enumerate() {
        let token = manager.get_token(id).await?;
        println!("   Credential {}: {} chars", i + 1, token.token.expose().len());
    }

    let stats = cache.stats();
    println!("\n   📊 Cache stats: hits={}, misses={}, puts={}", stats.hits, stats.misses, stats.puts);
    println!("      ├─ Each credential has independent cache entry");
    println!("      └─ Cache hit rate: {:.1}%\n", cache.hit_rate() * 100.0);

    // ═══════════════════════════════════════════════════════════════
    // Strategy 5: Full Cache Clear
    // ═══════════════════════════════════════════════════════════════
    println!("🗑️  Strategy 5: Full Cache Clear");
    println!("   Clearing entire cache...");

    cache.clear().await?;
    let stats_before = cache.stats();
    println!("   ✓ Cache cleared (size=0)");

    println!("   Requesting tokens for all {} credentials...", cred_ids.len());
    for id in &cred_ids {
        manager.get_token(id).await?;
    }

    let stats_after = cache.stats();
    println!("\n   📊 Before clear: hits={}, misses={}", stats_before.hits, stats_before.misses);
    println!("   📊 After clear:  hits={}, misses={} (all miss!)", stats_after.hits, stats_after.misses);
    println!("      └─ All tokens had to be refreshed from store\n");

    // ═══════════════════════════════════════════════════════════════
    // Strategy 6: Performance Comparison
    // ═══════════════════════════════════════════════════════════════
    println!("📊 Strategy 6: Performance Comparison");

    // Warm up cache
    for id in &cred_ids {
        manager.get_token(id).await?;
    }

    // Test cache hit performance
    println!("   Testing 100 requests with CACHE HITS...");
    let start = SystemTime::now();
    for _ in 0..100 {
        for id in &cred_ids {
            manager.get_token(id).await?;
        }
    }
    let cache_hit_time = start.elapsed()?;
    println!("   ✓ 400 cached requests: {:?}", cache_hit_time);

    // Test cache miss performance
    println!("\n   Testing 100 requests with CACHE MISSES...");
    let start = SystemTime::now();
    for _ in 0..100 {
        cache.clear().await?;
        for id in &cred_ids {
            manager.get_token(id).await?;
        }
    }
    let cache_miss_time = start.elapsed()?;
    println!("   ✓ 400 uncached requests: {:?}", cache_miss_time);

    let speedup = cache_miss_time.as_micros() as f64 / cache_hit_time.as_micros() as f64;
    println!("\n   ⚡ Performance Impact:");
    println!("      ├─ Cache hits: {:?} per request", cache_hit_time / 400);
    println!("      ├─ Cache misses: {:?} per request", cache_miss_time / 400);
    println!("      └─ Speedup: {:.1}x faster with caching", speedup);

    // ═══════════════════════════════════════════════════════════════
    // Final Statistics
    // ═══════════════════════════════════════════════════════════════
    let final_stats = cache.stats();
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                Final Cache Statistics                   ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ Total Hits:        {:>6}                               ║", final_stats.hits);
    println!("║ Total Misses:      {:>6}                               ║", final_stats.misses);
    println!("║ Total Puts:        {:>6}                               ║", final_stats.puts);
    println!("║ Hit Rate:          {:>5.1}%                              ║", cache.hit_rate() * 100.0);
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Key Takeaways:");
    println!("   • Cache dramatically improves performance ({:.1}x speedup)", speedup);
    println!("   • Each credential has independent cache entry");
    println!("   • Manual invalidation forces token refresh");
    println!("   • Cache stats help monitor system health");
    println!("   • Production: Use RedisTokenCache for distributed caching");

    println!("\n💡 Next Steps:");
    println!("   • See examples/distributed_lock.rs for concurrency control");
    println!("   • See examples/basic_usage.rs for complete workflow");

    Ok(())
}
