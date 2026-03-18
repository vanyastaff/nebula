//! CACHED combinator - caches validation results
//!
//! The CACHED combinator memoizes validation results to avoid redundant work.
//! Useful for expensive validators that may be called multiple times with
//! the same input.

use crate::foundation::{Validate, ValidationError};
use std::hash::{BuildHasher, Hash};
use std::sync::Arc;

// ============================================================================
// CACHED COMBINATOR
// ============================================================================

/// Caches validation results for improved performance.
///
/// This combinator stores validation results in memory and returns
/// cached results for inputs it has seen before.
///
/// # Type Parameters
///
/// * `V` - Inner validator type
///
/// # Requirements
///
/// - Input must be `Hash` for cache lookups
///
/// # Cache Behavior
///
/// - Thread-safe using lock-free `moka` cache
/// - **LRU eviction policy** with configurable capacity (default: 1000 entries)
/// - Cache persists for the lifetime of the validator
/// - Uses double-hashing `(u64, u64)` keys with independently-seeded SipHash
///   for ~1/2^128 collision probability
pub struct Cached<V> {
    pub(crate) validator: V,
    pub(crate) cache: Arc<moka::sync::Cache<(u64, u64), CachedResult>>,
}

/// Cached validation result (Arc-wrapped for cheap cloning).
type CachedResult = Arc<Result<(), ValidationError>>;

/// Default cache capacity (1000 entries)
const DEFAULT_CACHE_CAPACITY: u64 = 1000;

impl<V> Cached<V> {
    /// Creates a new CACHED combinator with default capacity (1000 entries).
    pub fn new(validator: V) -> Self {
        Self::with_capacity(validator, DEFAULT_CACHE_CAPACITY as usize)
    }

    /// Creates a new CACHED combinator with custom capacity.
    pub fn with_capacity(validator: V, capacity: usize) -> Self {
        Self {
            validator,
            cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(capacity as u64)
                    .build(),
            ),
        }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns the number of cached entries.
    pub fn cache_size(&self) -> u64 {
        self.cache.run_pending_tasks();
        self.cache.entry_count()
    }

    /// Clears the cache.
    pub fn clear_cache(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks();
    }

    /// Returns cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.run_pending_tasks();
        CacheStats {
            entries: self.cache.entry_count(),
            capacity: self.cache.policy().max_capacity().unwrap_or(0),
            weighted_size: self.cache.weighted_size(),
        }
    }

    /// Returns the cache capacity.
    pub fn capacity(&self) -> u64 {
        self.cache.policy().max_capacity().unwrap_or(0)
    }
}

/// Statistics about the cache.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// The number of entries currently stored in the cache.
    pub entries: u64,
    /// The maximum number of entries the cache can hold.
    pub capacity: u64,
    /// The weighted size of the cache (for caches with entry weighting).
    pub weighted_size: u64,
}

impl CacheStats {
    /// Returns the cache utilization as a percentage (0.0 to 1.0).
    #[must_use]
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            0.0
        } else {
            self.entries as f64 / self.capacity as f64
        }
    }
}

// ============================================================================
// VALIDATOR IMPLEMENTATION
// ============================================================================

impl<T: Hash + ?Sized, V> Validate<T> for Cached<V>
where
    V: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        let key = compute_double_hash(input);

        // Try to get from cache (lock-free!)
        if let Some(cached_result) = self.cache.get(&key) {
            return (*cached_result).clone();
        }

        // Cache miss - perform validation
        let result = self.validator.validate(input);

        // Store result in cache (Arc-wrapped for cheap cloning)
        self.cache.insert(key, Arc::new(result.clone()));

        result
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Two `RandomState` instances with distinct internal seeds.
/// `RandomState::new()` generates fresh random keys each time, so the two
/// hashers are statistically independent — a collision on one does not
/// imply a collision on the other, giving ~1/2^128 collision probability.
static HASHER_A: std::sync::LazyLock<std::collections::hash_map::RandomState> =
    std::sync::LazyLock::new(std::collections::hash_map::RandomState::new);
static HASHER_B: std::sync::LazyLock<std::collections::hash_map::RandomState> =
    std::sync::LazyLock::new(std::collections::hash_map::RandomState::new);

/// Computes two independent hashes of the input as a `(u64, u64)` cache key.
///
/// Each hash uses a separately-seeded `RandomState` (SipHash with random keys).
/// Because the two hashers have independent keys, collision probability is
/// ~1/2^128 — a genuine double-hash, not a seed-prepend trick.
fn compute_double_hash<T: Hash + ?Sized>(value: &T) -> (u64, u64) {
    (HASHER_A.hash_one(value), HASHER_B.hash_one(value))
}

/// Creates a CACHED combinator from a validator.
pub fn cached<V>(validator: V) -> Cached<V> {
    Cached::new(validator)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingValidator {
        min: usize,
        call_count: Arc<AtomicUsize>,
    }

    impl Validate<str> for CountingValidator {
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);

            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_cached_first_call() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        assert!(validator.validate("hello").is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cached_second_call() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        validator.validate("hello").unwrap();
        validator.validate("hello").unwrap();

        // Should only call inner validator once
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cached_different_inputs() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        validator.validate("hello").unwrap();
        validator.validate("world").unwrap();

        // Different inputs, should call twice
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cached_errors() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("hi").is_err());

        // Errors are cached too
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cache_size() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator { min: 5, call_count });

        assert_eq!(validator.cache_size(), 0);

        validator.validate("hello").unwrap();
        assert_eq!(validator.cache_size(), 1);

        validator.validate("world").unwrap();
        assert_eq!(validator.cache_size(), 2);
    }

    #[test]
    fn test_clear_cache() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        validator.validate("hello").unwrap();
        assert_eq!(validator.cache_size(), 1);

        validator.clear_cache();
        assert_eq!(validator.cache_size(), 0);

        // After clear, should call validator again
        validator.validate("hello").unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cache_stats() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator { min: 5, call_count });

        validator.validate("hello").unwrap();
        validator.validate("world").unwrap();

        let stats = validator.cache_stats();
        assert_eq!(stats.entries, 2);
        assert!(stats.weighted_size > 0);
    }

    #[test]
    fn test_cached_helper() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = cached(CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        });

        validator.validate("hello").unwrap();
        validator.validate("hello").unwrap();

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cached_preserves_validation_semantics() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let raw = CountingValidator {
            min: 5,
            call_count: call_count.clone(),
        };
        let wrapped = Cached::new(CountingValidator { min: 5, call_count });

        let raw_ok = raw.validate("hello");
        let cached_ok = wrapped.validate("hello");
        assert_eq!(raw_ok.is_ok(), cached_ok.is_ok());

        let raw_err = raw.validate("hi").unwrap_err();
        let cached_err = wrapped.validate("hi").unwrap_err();
        assert_eq!(raw_err.code, cached_err.code);
    }

    #[test]
    fn test_double_hash_differs() {
        // Verify the two hash components are different for the same input
        let (h1, h2) = compute_double_hash("hello");
        assert_ne!(h1, h2, "double hash components should differ");
    }
}
