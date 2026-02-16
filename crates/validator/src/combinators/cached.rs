//! CACHED combinator - caches validation results
//!
//! The CACHED combinator memoizes validation results to avoid redundant work.
//! Useful for expensive validators that may be called multiple times with
//! the same input.

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
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
pub struct Cached<V>
where
    V: Validate,
{
    pub(crate) validator: V,
    pub(crate) cache: Arc<moka::sync::Cache<u64, CachedResult>>,
}

/// Cached validation result (Arc-wrapped for cheap cloning).
type CachedResult = Arc<Result<(), ValidationError>>;

/// Default cache capacity (1000 entries)
const DEFAULT_CACHE_CAPACITY: u64 = 1000;

impl<V> Cached<V>
where
    V: Validate,
{
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

impl<V> Validate for Cached<V>
where
    V: Validate,
    V::Input: Hash,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        // Compute hash of input
        let hash = compute_hash(input);

        // Try to get from cache (lock-free!)
        if let Some(cached_result) = self.cache.get(&hash) {
            // Cache hit! Return cloned result from Arc
            return (*cached_result).clone();
        }

        // Cache miss - perform validation
        let result = self.validator.validate(input);

        // Store result in cache (Arc-wrapped for cheap cloning)
        self.cache.insert(hash, Arc::new(result.clone()));

        result
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("Cached({})", inner_meta.name).into(),
            description: Some(format!("Cached {}", inner_meta.name).into()),
            complexity: ValidationComplexity::Constant, // O(1) after first call
            cacheable: false,                           // Already cached!
            estimated_time: None,                       // Depends on cache hit/miss
            tags: {
                let mut tags = inner_meta.tags;
                tags.push(Cow::Borrowed("combinator"));
                tags.push("cached".into());
                tags.push("performance".into());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Computes a hash of the input for cache lookups.
fn compute_hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Creates a CACHED combinator from a validator.
pub fn cached<V>(validator: V) -> Cached<V>
where
    V: Validate,
    V::Input: Hash,
{
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

    impl Validate for CountingValidator {
        type Input = str;

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
    fn test_cached_metadata() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator { min: 5, call_count });

        let meta = validator.metadata();
        assert!(meta.name.contains("Cached"));
        assert_eq!(meta.complexity, ValidationComplexity::Constant);
        assert!(!meta.cacheable); // Already cached
        assert!(meta.tags.contains(&"cached".into()));
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
}
