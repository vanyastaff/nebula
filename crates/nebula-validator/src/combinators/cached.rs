//! CACHED combinator - caches validation results
//!
//! The CACHED combinator memoizes validation results to avoid redundant work.
//! Useful for expensive validators that may be called multiple times with
//! the same input.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = expensive_database_lookup().cached();
//!
//! // First call: performs database lookup
//! validator.validate("test@example.com")?;
//!
//! // Second call: returns cached result
//! validator.validate("test@example.com")?; // Fast!
//! ```

use crate::core::{TypedValidator, ValidatorMetadata, ValidationComplexity};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::RwLock;

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
/// - Input must be `Hash` and `Eq` for cache lookups
/// - Output and Error must be `Clone` to return cached values
///
/// # Cache Behavior
///
/// - Thread-safe using `RwLock`
/// - No automatic eviction (unbounded cache)
/// - Cache persists for the lifetime of the validator
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = RegexValidator::new(r"^\d+$").cached();
///
/// // First call: compiles and runs regex
/// validator.validate("12345")?;
///
/// // Second call: returns cached result
/// validator.validate("12345")?; // Much faster!
/// ```
///
/// # Warning
///
/// Only use caching for:
/// - Pure validators (same input → same output)
/// - Expensive operations (database, API calls, complex regex)
///
/// Do NOT cache:
/// - Validators with side effects
/// - Validators that depend on external state
/// - Cheap validators (caching overhead may be higher than validation)
pub struct Cached<V>
where
    V: TypedValidator,
{
    pub(crate) validator: V,
    pub(crate) cache: RwLock<HashMap<u64, CacheEntry<V>>>,
}

/// Cached validation result.
#[derive(Debug, Clone)]
struct CacheEntry<V>
where
    V: TypedValidator,
{
    result: Result<V::Output, V::Error>,
}

impl<V> Cached<V>
where
    V: TypedValidator,
{
    /// Creates a new CACHED combinator.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to cache
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns the number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    /// Clears the cache.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = expensive_validator().cached();
    /// validator.validate("test")?;
    /// assert_eq!(validator.cache_size(), 1);
    ///
    /// validator.clear_cache();
    /// assert_eq!(validator.cache_size(), 0);
    /// ```
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }

    /// Returns cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        let cache = self.cache.read().unwrap();
        CacheStats {
            entries: cache.len(),
            memory_estimate: std::mem::size_of_val(&*cache),
        }
    }
}

/// Statistics about the cache.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Number of entries in the cache.
    pub entries: usize,
    /// Estimated memory usage in bytes.
    pub memory_estimate: usize,
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<V> TypedValidator for Cached<V>
where
    V: TypedValidator,
    V::Input: Hash,
    V::Output: Clone,
    V::Error: Clone,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Compute hash of input
        let hash = compute_hash(input);

        // Try to read from cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(&hash) {
                // Cache hit!
                return entry.result.clone();
            }
        }

        // Cache miss - perform validation
        let result = self.validator.validate(input);

        // Store result in cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                hash,
                CacheEntry {
                    result: result.clone(),
                },
            );
        }

        result
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("Cached({})", inner_meta.name),
            description: Some(format!("Cached {}", inner_meta.name)),
            complexity: ValidationComplexity::Constant, // O(1) after first call
            cacheable: false, // Already cached!
            estimated_time: None, // Depends on cache hit/miss
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("cached".to_string());
                tags.push("performance".to_string());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

// ============================================================================
// ASYNC VALIDATOR IMPLEMENTATION
// ============================================================================

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<V> crate::core::AsyncValidator for Cached<V>
where
    V: TypedValidator + crate::core::AsyncValidator<
        Input = <V as TypedValidator>::Input,
        Output = <V as TypedValidator>::Output,
        Error = <V as TypedValidator>::Error
    > + Send + Sync,
    <V as TypedValidator>::Input: Hash + Sync,
    <V as TypedValidator>::Output: Clone + Send + Sync,
    <V as TypedValidator>::Error: Clone + Send + Sync,
{
    type Input = <V as TypedValidator>::Input;
    type Output = <V as TypedValidator>::Output;
    type Error = <V as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let hash = compute_hash(input);

        // Try cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(&hash) {
                return entry.result.clone();
            }
        }

        // Cache miss
        let result = self.validator.validate_async(input).await;

        // Store in cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                hash,
                CacheEntry {
                    result: result.clone(),
                },
            );
        }

        result
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
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
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::cached;
///
/// let validator = cached(expensive_validator());
/// ```
pub fn cached<V>(validator: V) -> Cached<V>
where
    V: TypedValidator,
    V::Input: Hash,
    V::Output: Clone,
    V::Error: Clone,
{
    Cached::new(validator)
}

// ============================================================================
// LRU CACHED VALIDATOR
// ============================================================================

/// Cached validator with LRU (Least Recently Used) eviction.
///
/// This is useful when you want bounded memory usage.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::lru_cached;
///
/// // Cache up to 100 results
/// let validator = lru_cached(expensive_validator(), 100);
/// ```
#[cfg(feature = "lru")]
pub fn lru_cached<V>(validator: V, capacity: usize) -> LruCached<V>
where
    V: TypedValidator,
    V::Input: Hash + Eq,
    V::Output: Clone,
    V::Error: Clone,
{
    LruCached::new(validator, capacity)
}

#[cfg(feature = "lru")]
#[derive(Debug)]
pub struct LruCached<V> {
    validator: V,
    cache: RwLock<lru::LruCache<u64, CacheEntry<V>>>,
}

#[cfg(feature = "lru")]
impl<V> LruCached<V>
where
    V: TypedValidator,
{
    pub fn new(validator: V, capacity: usize) -> Self {
        Self {
            validator,
            cache: RwLock::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(capacity).unwrap(),
            )),
        }
    }

    pub fn cache_size(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }
}

#[cfg(feature = "lru")]
impl<V> TypedValidator for LruCached<V>
where
    V: TypedValidator,
    V::Input: Hash + Eq,
    V::Output: Clone,
    V::Error: Clone,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let hash = compute_hash(input);

        // Try cache
        {
            let mut cache = self.cache.write().unwrap();
            if let Some(entry) = cache.get(&hash) {
                return entry.result.clone();
            }
        }

        // Cache miss
        let result = self.validator.validate(input);

        // Store in cache (may evict LRU entry)
        {
            let mut cache = self.cache.write().unwrap();
            cache.put(
                hash,
                CacheEntry {
                    result: result.clone(),
                },
            );
        }

        result
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();
        ValidatorMetadata {
            name: format!("LruCached({})", inner_meta.name),
            ..inner_meta
        }
    }
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use crate::core::traits::ValidatorExt;
    use super::*;
    use crate::core::ValidationError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingValidator {
        min: usize,
        call_count: Arc<AtomicUsize>,
    }

    impl TypedValidator for CountingValidator {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

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
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count,
        });

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
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count,
        });

        validator.validate("hello").unwrap();
        validator.validate("world").unwrap();

        let stats = validator.cache_stats();
        assert_eq!(stats.entries, 2);
        assert!(stats.memory_estimate > 0);
    }

    #[test]
    fn test_cached_metadata() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let validator = Cached::new(CountingValidator {
            min: 5,
            call_count,
        });

        let meta = validator.metadata();
        assert!(meta.name.contains("Cached"));
        assert_eq!(meta.complexity, ValidationComplexity::Constant);
        assert!(!meta.cacheable); // Already cached
        assert!(meta.tags.contains(&"cached".to_string()));
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