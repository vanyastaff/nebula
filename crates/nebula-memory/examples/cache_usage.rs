//! Cache usage example - demonstrates multi-level caching with eviction policies

use nebula_memory::cache::{ComputeCache, CacheConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Cache Usage Example ===\n");

    // Example 1: Basic Cache Usage
    println!("1. Basic Cache Example:");
    {
        let config = CacheConfig::default();
        let mut cache: ComputeCache<String, String> = ComputeCache::new(config);

        // Insert items
        cache.insert("key1".to_string(), "value1".to_string());
        cache.insert("key2".to_string(), "value2".to_string());
        cache.insert("key3".to_string(), "value3".to_string());

        println!("   Inserted: key1, key2, key3");

        // Access key1 (makes it most recently used)
        let _ = cache.get(&"key1".to_string());
        println!("   Accessed: key1 (now most recent)");

        // Insert key4 - should evict key2 (least recently used)
        cache.insert("key4".to_string(), "value4".to_string());
        println!("   Inserted: key4 (evicted key2 - least recent)");

        // Check what's in cache
        println!("   Cache contains key1: {}", cache.contains(&"key1".to_string()));
        println!("   Cache contains key2: {}", cache.contains(&"key2".to_string())); // false
        println!("   Cache contains key3: {}", cache.contains(&"key3".to_string()));
        println!("   Cache contains key4: {}", cache.contains(&"key4".to_string()));
    }

    println!();

    // Example 2: Cache with capacity
    println!("2. Cache with Capacity Limit:");
    {
        let config = CacheConfig {
            capacity: 3,
            ..Default::default()
        };
        let mut cache: ComputeCache<String, i32> = ComputeCache::new(config);

        cache.insert("a".to_string(), 1);
        cache.insert("b".to_string(), 2);
        cache.insert("c".to_string(), 3);

        println!("   Inserted: a, b, c");

        // Access 'a' multiple times (doesn't affect FIFO order)
        let _ = cache.get(&"a".to_string());
        let _ = cache.get(&"a".to_string());

        // Insert 'd' - should evict 'a' (first in)
        cache.insert("d".to_string(), 4);
        println!("   Inserted: d (evicted 'a' - first in, regardless of access)");

        println!("   Cache contains 'a': {}", cache.contains(&"a".to_string())); // false
        println!("   Cache contains 'b': {}", cache.contains(&"b".to_string()));
        println!("   Cache contains 'c': {}", cache.contains(&"c".to_string()));
        println!("   Cache contains 'd': {}", cache.contains(&"d".to_string()));
    }

    println!();

    // Example 3: Cache operations
    println!("3. Cache Operations:");
    {
        let config = CacheConfig::default();
        let mut cache: ComputeCache<String, i32> = ComputeCache::new(config);

        cache.insert("x".to_string(), 100);
        cache.insert("y".to_string(), 200);
        cache.insert("z".to_string(), 300);

        println!("   Inserted: x, y, z");

        // Insert 'w' - will evict random item
        cache.insert("w".to_string(), 400);
        println!("   Inserted: w (evicted random item)");

        // Check size
        println!("   Cache size: {}", cache.len());
    }

    println!();

    // Example 4: Cache with different value types
    println!("4. Cache with Complex Values:");
    {
        #[derive(Debug, Clone)]
        struct User {
            id: u64,
            name: String,
            email: String,
        }

        let config = CacheConfig {
            capacity: 100,
            ..Default::default()
        };
        let mut user_cache: ComputeCache<u64, User> = ComputeCache::new(config);

        let user1 = User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        };

        let user2 = User {
            id: 2,
            name: "Bob".to_string(),
            email: "bob@example.com".to_string(),
        };

        user_cache.insert(1, user1.clone());
        user_cache.insert(2, user2.clone());

        if let Some(user) = user_cache.get(&1) {
            println!("   Found user: {} ({})", user.name, user.email);
        }

        println!("   User cache size: {}", user_cache.len());
    }

    println!();

    // Example 5: Cache statistics
    #[cfg(feature = "stats")]
    {
        println!("5. Cache Statistics:");
        let config = CacheConfig {
            capacity: 10,
            ..Default::default()
        };
        let mut cache: ComputeCache<i32, i32> = ComputeCache::new(config);

        // Perform operations
        for i in 0..5 {
            cache.insert(i, i * 10);
        }

        // Some hits
        let _ = cache.get(&1);
        let _ = cache.get(&2);
        let _ = cache.get(&1);

        // Some misses
        let _ = cache.get(&100);
        let _ = cache.get(&200);

        println!("   Total operations: ~10");
        println!("   Cache size: {}", cache.len());
        println!("   Capacity: {}", cache.capacity());
    }

    println!("\n=== Cache example completed successfully! ===");
    Ok(())
}
