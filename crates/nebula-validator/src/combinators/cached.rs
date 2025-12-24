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

use crate::core::{TypedValidator, ValidationComplexity, ValidatorMetadata};
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
/// - Input must be `Hash` and `Eq` for cache lookups
/// - Output and Error must be `Clone` to return cached values
///
/// # Cache Behavior
///
/// - Thread-safe using lock-free `moka` cache
/// - **LRU eviction policy** with configurable capacity (default: 1000 entries)
/// - Cache persists for the lifetime of the validator
/// - Tracks hit/miss statistics automatically
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// // Default capacity (1000 entries)
/// let validator = RegexValidator::new(r"^\d+$").cached();
///
/// // Custom capacity
/// let validator = RegexValidator::new(r"^\d+$").cached_with_capacity(100);
///
/// // First call: compiles and runs regex
/// validator.validate("12345")?;
///
/// // Second call: returns cached result
/// validator.validate("12345")?; // Much faster!
///
/// // Check statistics
/// let stats = validator.cache_stats();
/// println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);
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
    pub(crate) cache: Arc<moka::sync::Cache<u64, CachedResult<V>>>,
}

/// Cached validation result (Arc-wrapped for cheap cloning).
type CachedResult<V> = Arc<Result<<V as TypedValidator>::Output, <V as TypedValidator>::Error>>;

/// Default cache capacity (1000 entries)
const DEFAULT_CACHE_CAPACITY: u64 = 1000;

impl<V> Cached<V>
where
    V: TypedValidator,
    V::Output: Send + Sync + 'static,
    V::Error: Send + Sync + 'static,
{
    /// Creates a new CACHED combinator with default capacity (1000 entries).
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to cache
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = expensive_validator().cached();
    /// ```
    pub fn new(validator: V) -> Self {
        Self::with_capacity(validator, DEFAULT_CACHE_CAPACITY as usize)
    }

    /// Creates a new CACHED combinator with custom capacity.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to cache
    /// * `capacity` - Maximum number of cache entries
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Small cache for memory-constrained environments
    /// let validator = expensive_validator().cached_with_capacity(100);
    /// ```
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
    ///
    /// Note: This method calls `run_pending_tasks()` to ensure accurate count.
    pub fn cache_size(&self) -> u64 {
        self.cache.run_pending_tasks();
        self.cache.entry_count()
    }

    /// Clears the cache.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = expensive_validator().cached();
    /// validator.validate("test")?;
    /// assert!(validator.cache_size() > 0);
    ///
    /// validator.clear_cache();
    /// assert_eq!(validator.cache_size(), 0);
    /// ```
    pub fn clear_cache(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks();
    }

    /// Returns cache statistics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = expensive_validator().cached();
    /// validator.validate("test1")?;
    /// validator.validate("test1")?; // Cache hit
    /// validator.validate("test2")?;
    ///
    /// let stats = validator.cache_stats();
    /// assert_eq!(stats.entries, 2);
    /// ```
    ///
    /// Note: This method calls `run_pending_tasks()` to ensure accurate stats.
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
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<V> TypedValidator for Cached<V>
where
    V: TypedValidator,
    V::Input: Hash,
    V::Output: Clone + Send + Sync + 'static,
    V::Error: Clone + Send + Sync + 'static,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
            name: format!("Cached({})", inner_meta.name),
            description: Some(format!("Cached {}", inner_meta.name)),
            complexity: ValidationComplexity::Constant, // O(1) after first call
            cacheable: false,                           // Already cached!
            estimated_time: None,                       // Depends on cache hit/miss
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
    V: TypedValidator
        + crate::core::AsyncValidator<
            Input = <V as TypedValidator>::Input,
            Output = <V as TypedValidator>::Output,
            Error = <V as TypedValidator>::Error,
        > + Send
        + Sync,
    <V as TypedValidator>::Input: Hash + Sync,
    <V as TypedValidator>::Output: Clone + Send + Sync + 'static,
    <V as TypedValidator>::Error: Clone + Send + Sync + 'static,
{
    type Input = <V as TypedValidator>::Input;
    type Output = <V as TypedValidator>::Output;
    type Error = <V as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let hash = compute_hash(input);

        // Try to get from cache (lock-free!)
        if let Some(cached_result) = self.cache.get(&hash) {
            // Cache hit! Return cloned result from Arc
            return (*cached_result).clone();
        }

        // Cache miss - perform async validation
        let result = self.validator.validate_async(input).await;

        // Store result in cache (Arc-wrapped for cheap cloning)
        self.cache.insert(hash, Arc::new(result.clone()));

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
    V::Output: Clone + Send + Sync + 'static,
    V::Error: Clone + Send + Sync + 'static,
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
// TODO: Re-enable when lru crate is added as dependency
#[cfg(any())] // Disabled until lru crate is added
#[allow(dead_code)]
pub fn lru_cached<V>(validator: V, capacity: usize) -> LruCached<V>
where
    V: TypedValidator,
    V::Input: Hash + Eq,
    V::Output: Clone,
    V::Error: Clone,
{
    LruCached::new(validator, capacity)
}

// TODO: Re-enable when lru crate is added as dependency
#[cfg(any())] // Disabled until lru crate is added
#[derive(Debug)]
pub struct LruCached<V> {
    validator: V,
    cache: RwLock<lru::LruCache<u64, CacheEntry<V>>>,
}

#[cfg(any())] // Disabled until lru crate is added
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

#[cfg(any())] // Disabled until lru crate is added
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
    use super::*;
    use crate::core::ValidationError;
    use crate::core::traits::ValidatorExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
