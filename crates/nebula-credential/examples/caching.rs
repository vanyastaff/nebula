//! Caching and performance optimization example
//!
//! Demonstrates US4: Performance Optimization with Caching
//! - Cache configuration
//! - Cache hit/miss tracking
//! - TTL-based expiration
//! - Performance benefits

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Credential Manager: Caching Example ===\n");

    // 1. Create manager with caching enabled
    println!("1. Creating manager with caching...");
    let storage = Arc::new(MockStorageProvider::new());
    let manager = CredentialManager::builder()
        .storage(storage)
        .cache_ttl(Duration::from_secs(300)) // 5 minute TTL
        .cache_max_size(1000) // Max 1000 entries
        .build();
    println!("   ✓ Manager created with cache enabled\n");

    // 2. Store a credential
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let context = CredentialContext::new("user-1");
    let id = CredentialId::new("api-key")?;
    let data = encrypt(&key, b"secret-api-key-12345")?;

    println!("2. Storing credential...");
    manager
        .store(&id, data, CredentialMetadata::new(), &context)
        .await?;
    println!("   ✓ Credential stored\n");

    // 3. First retrieve (cache miss)
    println!("3. First retrieve (cache miss)...");
    let start = Instant::now();
    manager.retrieve(&id, &context).await?;
    let first_duration = start.elapsed();
    println!("   ✓ Retrieved in {:?}", first_duration);

    if let Some(stats) = manager.cache_stats() {
        println!(
            "   Cache stats: {} hits, {} misses",
            stats.hits, stats.misses
        );
    }
    println!();

    // 4. Second retrieve (cache hit)
    println!("4. Second retrieve (cache hit)...");
    let start = Instant::now();
    manager.retrieve(&id, &context).await?;
    let second_duration = start.elapsed();
    println!("   ✓ Retrieved in {:?}", second_duration);

    if let Some(stats) = manager.cache_stats() {
        println!(
            "   Cache stats: {} hits, {} misses",
            stats.hits, stats.misses
        );
        println!("   Hit rate: {:.1}%", stats.hit_rate() * 100.0);
    }
    println!();

    // 5. Multiple retrievals to demonstrate cache performance
    println!("5. Testing cache performance (10 retrievals)...");
    let start = Instant::now();
    for _ in 0..10 {
        manager.retrieve(&id, &context).await?;
    }
    let batch_duration = start.elapsed();
    println!("   ✓ 10 retrievals completed in {:?}", batch_duration);
    println!("   ✓ Average: {:?} per retrieval", batch_duration / 10);

    if let Some(stats) = manager.cache_stats() {
        println!("   Final cache stats:");
        println!("     - Hits: {}", stats.hits);
        println!("     - Misses: {}", stats.misses);
        println!("     - Hit rate: {:.1}%", stats.hit_rate() * 100.0);
        println!(
            "     - Cached entries: {}/{}",
            stats.size, stats.max_capacity
        );
        println!("     - Utilization: {:.1}%", stats.utilization() * 100.0);
    }
    println!();

    // 6. Demonstrate cache with multiple credentials
    println!("6. Storing and retrieving multiple credentials...");
    for i in 1..=5 {
        let cred_id = CredentialId::new(&format!("cred-{}", i))?;
        let cred_data = encrypt(&key, format!("secret-{}", i).as_bytes())?;
        manager
            .store(&cred_id, cred_data, CredentialMetadata::new(), &context)
            .await?;
    }
    println!("   ✓ Stored 5 credentials");

    // Retrieve all to populate cache
    for i in 1..=5 {
        let cred_id = CredentialId::new(&format!("cred-{}", i))?;
        manager.retrieve(&cred_id, &context).await?;
    }

    if let Some(stats) = manager.cache_stats() {
        println!("   ✓ Cache now contains {} entries", stats.size);
    }
    println!();

    println!("=== Caching example completed! ===");
    println!("\nKey takeaways:");
    println!("  • Cache significantly improves retrieval performance");
    println!("  • Hit rate metrics help monitor cache effectiveness");
    println!("  • TTL ensures credentials don't stay cached indefinitely");
    println!("  • LRU eviction manages memory when cache is full");

    Ok(())
}
