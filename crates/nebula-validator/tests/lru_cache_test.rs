use nebula_validator::core::{TypedValidator, ValidatorExt};
use nebula_validator::validators::string::min_length;

#[test]
fn test_lru_cache_basic() {
    // Create a validator with small capacity
    let validator = min_length(3).cached_with_capacity(2);

    // First validations - all misses
    assert!(validator.validate("hello").is_ok());
    assert!(validator.validate("world").is_ok());
    assert!(validator.validate("test").is_ok());

    let stats = validator.cache_stats();
    assert_eq!(stats.entries, 2); // Only 2 entries fit (LRU evicted "hello")
    assert_eq!(stats.capacity, 2);
    assert_eq!(stats.misses, 3); // 3 misses
    assert_eq!(stats.hits, 0); // No hits yet
}

#[test]
fn test_lru_cache_hits() {
    let validator = min_length(5).cached_with_capacity(10);

    // First call - miss
    assert!(validator.validate("hello").is_ok());

    // Second call - hit!
    assert!(validator.validate("hello").is_ok());

    let stats = validator.cache_stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hit_rate(), 0.5); // 50% hit rate
}

#[test]
fn test_lru_eviction() {
    // Create cache with capacity of 3
    let validator = min_length(1).cached_with_capacity(3);

    // Fill cache
    validator.validate("a").unwrap();
    validator.validate("b").unwrap();
    validator.validate("c").unwrap();

    assert_eq!(validator.cache_size(), 3);

    // Add one more - should evict "a" (least recently used)
    validator.validate("d").unwrap();
    assert_eq!(validator.cache_size(), 3); // Still 3

    // Access "a" again - should be a miss (was evicted)
    validator.validate("a").unwrap();

    let stats = validator.cache_stats();
    assert_eq!(stats.hits, 0); // "a" was evicted, so it's a miss
    assert_eq!(stats.misses, 5); // a, b, c, d, a again
}

#[test]
fn test_lru_ordering() {
    let validator = min_length(1).cached_with_capacity(2);

    // Add a, b
    validator.validate("a").unwrap();
    validator.validate("b").unwrap();

    // Access "a" again (moves it to front)
    validator.validate("a").unwrap();

    // Add "c" - should evict "b" (not "a" because we just accessed it)
    validator.validate("c").unwrap();

    // Access "b" - should be a miss (was evicted)
    validator.validate("b").unwrap();

    let stats = validator.cache_stats();
    assert_eq!(stats.entries, 2);
    // We have: 1 hit (second access to "a"), rest are misses
    assert_eq!(stats.hits, 1);
}

#[test]
fn test_cache_clear() {
    let validator = min_length(1).cached();

    validator.validate("test").unwrap();
    assert_eq!(validator.cache_size(), 1);

    validator.clear_cache();
    assert_eq!(validator.cache_size(), 0);

    let stats = validator.cache_stats();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 0); // Stats reset too
}

#[test]
fn test_cache_utilization() {
    let validator = min_length(1).cached_with_capacity(10);

    // Add 3 entries
    validator.validate("a").unwrap();
    validator.validate("b").unwrap();
    validator.validate("c").unwrap();

    let stats = validator.cache_stats();
    assert_eq!(stats.utilization(), 0.3); // 3 out of 10 = 30%
}

#[test]
fn test_default_capacity() {
    let validator = min_length(1).cached();
    assert_eq!(validator.capacity(), 1000); // Default capacity
}
