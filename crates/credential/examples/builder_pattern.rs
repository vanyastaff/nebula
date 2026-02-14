//! Builder pattern usage example
//!
//! Demonstrates US5: Builder Pattern Configuration
//! - Fluent API for manager construction
//! - Compile-time type safety
//! - Optional configuration

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Credential Manager: Builder Pattern Example ===\n");

    // 1. Minimal configuration (only storage required)
    println!("1. Creating manager with minimal configuration...");
    let storage = Arc::new(MockStorageProvider::new());
    let manager_minimal = CredentialManager::builder()
        .storage(storage.clone())
        .build();

    println!("   ✓ Manager created with defaults");
    if manager_minimal.cache_stats().is_none() {
        println!("   ✓ Cache disabled by default");
    }
    println!();

    // 2. Full configuration with all options
    println!("2. Creating manager with full configuration...");
    let manager_full = CredentialManager::builder()
        .storage(storage.clone())
        .cache_ttl(Duration::from_secs(600)) // 10 minute cache
        .cache_max_size(5000) // 5000 entry capacity
        .build();

    println!("   ✓ Manager created with custom cache settings");
    if let Some(stats) = manager_full.cache_stats() {
        println!(
            "   ✓ Cache enabled with {} entry capacity",
            stats.max_capacity
        );
    }
    println!();

    // 3. Using CacheConfig struct
    println!("3. Creating manager with CacheConfig struct...");
    let cache_config = CacheConfig {
        enabled: true,
        ttl: Some(Duration::from_secs(300)),
        idle_timeout: Some(Duration::from_secs(120)),
        max_capacity: 2000,
        eviction_strategy: EvictionStrategy::Lru,
    };

    let manager_config = CredentialManager::builder()
        .storage(storage.clone())
        .cache_config(cache_config)
        .build();

    println!("   ✓ Manager created with CacheConfig struct");
    if let Some(stats) = manager_config.cache_stats() {
        println!("   ✓ Cache: {} max entries", stats.max_capacity);
    }
    println!();

    // 4. Demonstrate method chaining
    println!("4. Demonstrating fluent method chaining...");
    let manager_chained = CredentialManager::builder()
        .storage(storage)
        .cache_ttl(Duration::from_secs(300))
        .cache_max_size(1000)
        .build();

    println!("   ✓ All methods chain fluently");
    println!("   ✓ Builder provides clean, readable API\n");

    // 5. Use the manager
    println!("5. Using the configured manager...");
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let context = CredentialContext::new("user-1");
    let id = CredentialId::new("example-credential")?;
    let data = encrypt(&key, b"my-secret-data")?;

    manager_chained
        .store(&id, data, CredentialMetadata::new(), &context)
        .await?;
    println!("   ✓ Stored credential using builder-configured manager");

    let retrieved = manager_chained.retrieve(&id, &context).await?;
    if retrieved.is_some() {
        println!("   ✓ Retrieved credential successfully\n");
    }

    println!("=== Builder pattern example completed! ===");
    println!("\nKey takeaways:");
    println!("  • Builder pattern enforces required parameters (storage)");
    println!("  • Fluent API makes configuration readable");
    println!("  • Sensible defaults for optional settings");
    println!("  • Type-safe construction prevents invalid states");

    Ok(())
}
