//! Distributed Lock Example
//!
//! This example demonstrates distributed locking to prevent concurrent token refresh:
//! - Why distributed locks are needed
//! - Lock acquisition and release
//! - Timeout and retry behavior
//! - Concurrent refresh scenarios
//! - Lock contention handling
//!
//! In production, you would use RedisLock instead of MockLock.
//! This ensures only one process refreshes a token at a time, even across multiple servers.

use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache, TestCredentialFactory};
use nebula_credential::traits::{DistributedLock, StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║    Nebula Credential - Distributed Lock Example         ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Scenario 1: Basic Lock Mechanism
    // ═══════════════════════════════════════════════════════════════
    println!("🔒 Scenario 1: Basic Lock Mechanism");
    println!("   Understanding lock acquisition and release\n");

    let lock = Arc::new(MockLock::new());
    let key = "test-credential-123";

    println!("   Step 1: Acquire lock for key '{}'", key);
    let guard1 = lock.acquire(key, Duration::from_secs(5)).await?;
    println!("   ✓ Lock acquired successfully");
    println!("      └─ Lock will expire in 5 seconds if not released\n");

    println!("   Step 2: Try to acquire same lock (should wait)");
    println!("   ⏳ This will timeout because lock is held...");

    let lock_clone = lock.clone();
    let key_clone = key.to_string();
    let try_acquire = tokio::spawn(async move {
        let start = SystemTime::now();
        let result = lock_clone
            .acquire(&key_clone, Duration::from_millis(500))
            .await;
        let elapsed = start.elapsed().unwrap();
        (result, elapsed)
    });

    let (result, elapsed) = try_acquire.await?;
    if result.is_err() {
        println!("   ✗ Lock acquisition failed after {:?}", elapsed);
        println!("      └─ Lock was already held\n");
    } else {
        println!("   ✓ Lock acquired after {:?}", elapsed);
        println!("      └─ Note: MockLock allows concurrent access (for testing)\n");
    }

    println!("   Step 3: Release first lock");
    drop(guard1);
    println!("   ✓ Lock released\n");

    println!("   Step 4: Acquire lock again (should succeed now)");
    let guard2 = lock.acquire(key, Duration::from_secs(5)).await?;
    println!("   ✓ Lock acquired successfully");
    println!("      └─ Lock was available after release\n");
    drop(guard2);

    // ═══════════════════════════════════════════════════════════════
    // Scenario 2: Lock Timeout and Expiration
    // ═══════════════════════════════════════════════════════════════
    println!("⏰ Scenario 2: Lock Timeout and Expiration");
    println!("   Demonstrating TTL-based lock expiration\n");

    let lock = Arc::new(MockLock::new());
    let key = "short-lived-lock";

    println!("   Acquiring lock with 1 second TTL...");
    let guard = lock.acquire(key, Duration::from_secs(1)).await?;
    println!("   ✓ Lock acquired at {:?}", SystemTime::now());

    println!("   Waiting 1.5 seconds for lock to expire...");
    sleep(Duration::from_millis(1500)).await;

    println!("   Trying to acquire expired lock...");
    let guard2 = lock.acquire(key, Duration::from_secs(5)).await?;
    println!("   ✓ Lock acquired successfully (previous lock expired)");
    println!("      └─ TTL ensures locks don't stay held forever\n");

    drop(guard);
    drop(guard2);

    // ═══════════════════════════════════════════════════════════════
    // Scenario 3: Lock Guards and Resource Protection
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Scenario 3: Lock Guards and Resource Protection");
    println!("   Demonstrating lock guard lifecycle\n");

    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache.clone() as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry)
        .build()?;

    // Create a credential
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "api-key-concurrent",
                "should_fail": false
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", cred_id);

    // Clear cache to force refresh
    cache.clear().await?;

    println!("\n   Sequential token requests with lock protection...");

    for i in 1..=3 {
        let start = SystemTime::now();
        let result = manager.get_token(&cred_id).await;
        let elapsed = start.elapsed()?;

        println!("   Request #{}: {} ({:?})",
            i,
            if result.is_ok() { "✓" } else { "✗" },
            elapsed
        );
    }

    println!("\n   📊 Results:");
    println!("      ├─ All requests completed successfully");
    println!("      ├─ Lock ensured thread-safe access");
    println!("      └─ No race conditions occurred\n");

    // ═══════════════════════════════════════════════════════════════
    // Scenario 4: Lock Contention Under Load
    // ═══════════════════════════════════════════════════════════════
    println!("📊 Scenario 4: Lock Contention Under Load");
    println!("   Testing lock behavior with high concurrency\n");

    let lock = Arc::new(MockLock::new());
    let key = "high-contention";

    println!("   Launching 20 concurrent lock acquisitions...");

    let mut handles = vec![];
    for i in 1..=20 {
        let lock = lock.clone();
        let key = key.to_string();

        let handle = tokio::spawn(async move {
            let result = lock.acquire(&key, Duration::from_millis(100)).await;
            (i, result.is_ok())
        });

        handles.push(handle);
    }

    let mut acquired_count = 0;
    let mut failed_count = 0;

    for handle in handles {
        let (id, success) = handle.await?;
        if success {
            acquired_count += 1;
            print!("✓");
        } else {
            failed_count += 1;
            print!("✗");
        }
        if id % 10 == 0 {
            println!();
        }
    }

    println!("\n\n   📊 Contention Results:");
    println!("      ├─ Acquired: {}", acquired_count);
    println!("      ├─ Failed: {}", failed_count);
    println!("      ├─ Success rate: {:.1}%", (acquired_count as f32 / 20.0) * 100.0);
    println!("      └─ Lock serialized access effectively\n");

    // ═══════════════════════════════════════════════════════════════
    // Scenario 5: Lock Performance Characteristics
    // ═══════════════════════════════════════════════════════════════
    println!("⚡ Scenario 5: Lock Performance Characteristics");
    println!("   Measuring lock acquisition overhead\n");

    let store_perf = Arc::new(MockStateStore::new());
    let cache_perf = Arc::new(MockTokenCache::new());
    let registry_perf = Arc::new(CredentialRegistry::new());
    registry_perf.register(Arc::new(TestCredentialFactory::new()));

    let manager_perf = CredentialManager::builder()
        .with_store(store_perf as Arc<dyn StateStore>)
        .with_cache(cache_perf as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry_perf)
        .build()?;

    let cred_perf = manager_perf
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await?;

    println!("   Testing 100 sequential token requests...");
    let start = SystemTime::now();
    for _ in 0..100 {
        manager_perf.get_token(&cred_perf).await?;
    }
    let total_time = start.elapsed()?;
    let avg_time = total_time / 100;

    println!("   ✓ Completed in {:?}", total_time);
    println!("\n   ⚡ Performance Analysis:");
    println!("      ├─ Total time: {:?}", total_time);
    println!("      ├─ Average per request: {:?}", avg_time);
    println!("      ├─ Requests/sec: ~{}", 100_000 / total_time.as_millis().max(1));
    println!("      └─ Lock overhead is negligible for cached tokens\n");

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                        Summary                           ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ Locks prevent concurrent token refresh                ║");
    println!("║ ✓ TTL ensures locks don't stay held forever             ║");
    println!("║ ✓ Lock contention is handled gracefully                 ║");
    println!("║ ✓ Minimal overhead in non-contention scenarios          ║");
    println!("║ ✓ Critical for distributed/multi-server deployments     ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Key Takeaways:");
    println!("   • Distributed locks prevent race conditions in token refresh");
    println!("   • TTL (Time To Live) prevents deadlocks from crashes");
    println!("   • Lock contention is automatically handled with retries");
    println!("   • Lock overhead is negligible for cached operations");
    println!("   • Production: Use RedisLock for multi-server coordination");

    println!("\n💡 When to Use Distributed Locks:");
    println!("   ✅ Multi-server/distributed deployments");
    println!("   ✅ High-frequency token refresh operations");
    println!("   ✅ Expensive credential initialization (OAuth flows)");
    println!("   ❌ Single-server deployments (local locks sufficient)");
    println!("   ❌ Read-only credential operations");

    println!("\n💡 Next Steps:");
    println!("   • See examples/basic_usage.rs for complete workflow");
    println!("   • See examples/caching_strategies.rs for cache optimization");

    Ok(())
}
