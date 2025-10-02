//! Basic Credential Management Example
//!
//! This example demonstrates the complete credential lifecycle:
//! - Setting up a CredentialManager with in-memory components
//! - Registering a credential type (TestCredential)
//! - Creating a new credential
//! - Retrieving an access token
//! - Token caching behavior
//! - Refreshing credentials
//! - Deleting credentials
//!
//! This example uses test mocks for simplicity. In production, you would use:
//! - PostgresStateStore instead of MockStateStore
//! - RedisTokenCache instead of MockTokenCache
//! - RedisLock instead of MockLock

use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache, TestCredentialFactory};
use nebula_credential::traits::{StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║     Nebula Credential - Basic Usage Example             ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Step 1: Create the components for CredentialManager
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Step 1: Setting up CredentialManager components");
    println!("   ├─ Creating in-memory state store...");
    let store = Arc::new(MockStateStore::new());

    println!("   ├─ Creating in-memory token cache...");
    let cache = Arc::new(MockTokenCache::new());

    println!("   ├─ Creating distributed lock...");
    let lock = MockLock::new();

    println!("   └─ Creating credential registry...");
    let registry = Arc::new(CredentialRegistry::new());
    println!("   ✓ All components created\n");

    // ═══════════════════════════════════════════════════════════════
    // Step 2: Register credential types
    // ═══════════════════════════════════════════════════════════════
    println!("🔧 Step 2: Registering credential types");
    println!("   └─ Registering 'test_credential' factory...");
    registry.register(Arc::new(TestCredentialFactory::new()));
    println!("   ✓ Factory registered\n");

    // ═══════════════════════════════════════════════════════════════
    // Step 3: Build the CredentialManager
    // ═══════════════════════════════════════════════════════════════
    println!("🏗️  Step 3: Building CredentialManager");
    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache.clone() as Arc<dyn TokenCache>)
        .with_lock(lock)
        .with_registry(registry)
        .build()?;
    println!("   ✓ CredentialManager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // Step 4: Create a credential
    // ═══════════════════════════════════════════════════════════════
    println!("➕ Step 4: Creating a new credential");
    println!("   ├─ Type: test_credential");
    println!("   └─ Input: {{\"value\": \"my-secret-api-key\", \"should_fail\": false}}");

    let credential_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "my-secret-api-key",
                "should_fail": false
            }),
        )
        .await?;

    println!("   ✓ Credential created with ID: {}\n", credential_id);

    // ═══════════════════════════════════════════════════════════════
    // Step 5: Get an access token (first time - will be cached)
    // ═══════════════════════════════════════════════════════════════
    println!("🔑 Step 5: Getting access token (first request)");
    let token1 = manager.get_token(&credential_id).await?;

    println!("   ✓ Token retrieved:");
    println!("      ├─ Type: {:?}", token1.token_type);
    println!("      ├─ Expired: {}", token1.is_expired());
    println!("      └─ Token value: {} (REDACTED)", token1.token.expose().chars().take(5).collect::<String>());

    // Check cache statistics
    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}\n", stats.hits, stats.misses, stats.puts);

    // ═══════════════════════════════════════════════════════════════
    // Step 6: Get the same token again (should hit cache)
    // ═══════════════════════════════════════════════════════════════
    println!("🔑 Step 6: Getting access token (second request - from cache)");
    let token2 = manager.get_token(&credential_id).await?;

    println!("   ✓ Token retrieved from cache:");
    println!("      └─ Same token: {}", token1.token.expose() == token2.token.expose());

    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}", stats.hits, stats.misses, stats.puts);
    println!("      └─ Cache hit rate: {:.1}%\n", cache.hit_rate() * 100.0);

    // ═══════════════════════════════════════════════════════════════
    // Step 7: Clear cache and get token again (will refresh)
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Step 7: Testing token refresh");
    println!("   ├─ Clearing cache...");
    cache.clear().await?;

    println!("   └─ Requesting token (will trigger refresh)...");
    let token3 = manager.get_token(&credential_id).await?;

    println!("   ✓ Token refreshed:");
    println!("      └─ Different from cached: {}", token1.token.expose() != token3.token.expose());

    let stats = cache.stats();
    println!("   📊 Cache stats: hits={}, misses={}, puts={}\n", stats.hits, stats.misses, stats.puts);

    // ═══════════════════════════════════════════════════════════════
    // Step 8: List all credentials
    // ═══════════════════════════════════════════════════════════════
    println!("📋 Step 8: Listing all credentials");
    let all_creds = manager.list_credentials().await?;
    println!("   ✓ Found {} credential(s):", all_creds.len());
    for cred in &all_creds {
        println!("      └─ {}", cred);
    }
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Step 9: Delete the credential
    // ═══════════════════════════════════════════════════════════════
    println!("🗑️  Step 9: Deleting credential");
    manager.delete_credential(&credential_id).await?;
    println!("   ✓ Credential deleted: {}\n", credential_id);

    // Verify deletion
    let all_creds = manager.list_credentials().await?;
    println!("   📋 Remaining credentials: {}", all_creds.len());

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                    Summary                               ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ Created CredentialManager with in-memory components   ║");
    println!("║ ✓ Registered TestCredential factory                     ║");
    println!("║ ✓ Created credential and retrieved tokens               ║");
    println!("║ ✓ Demonstrated caching behavior                         ║");
    println!("║ ✓ Tested token refresh                                  ║");
    println!("║ ✓ Listed and deleted credentials                        ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Next Steps:");
    println!("   • See examples/custom_credential.rs for custom implementations");
    println!("   • See examples/caching_strategies.rs for cache optimization");
    println!("   • See examples/distributed_lock.rs for concurrency control");

    Ok(())
}
