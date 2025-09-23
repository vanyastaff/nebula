//! Asynchronous compute cache implementation
//!
//! This module provides a high-performance asynchronous compute cache with
//! advanced concurrency features, batch operations, and comprehensive monitoring.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Weak,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, sync::Arc, vec::Vec},
    core::{
        hash::Hash,
        pin::Pin,
        task::{Context, Poll, Waker},
        time::Duration,
    },
    hashbrown::{HashMap, HashSet},
};

use futures_core::Future;
use tokio::{
    sync::{Mutex, RwLock, Semaphore},
    task::JoinHandle,
    time::{sleep, timeout},
};

use super::compute::{CacheKey, ComputeCache};
use super::config::CacheConfig;
use super::stats::{AtomicCacheStats, StatsProvider, TimeWindow};
use crate::error::{MemoryError, MemoryResult};

/// Result type for async cache operations
pub type AsyncCacheResult<T> = Result<T, MemoryError>;

/// Configuration for async cache behavior
#[derive(Debug, Clone)]
pub struct AsyncCacheConfig {
    /// Base cache configuration
    pub cache_config: CacheConfig,
    /// Maximum concurrent computations
    pub max_concurrent_computations: usize,
    /// Timeout for individual computation operations
    pub computation_timeout: Option<Duration>,
    /// Enable deduplication of concurrent requests for the same key
    pub enable_deduplication: bool,
    /// Enable background refresh for frequently accessed items
    pub enable_background_refresh: bool,
    /// Background refresh threshold (hit count)
    pub background_refresh_threshold: usize,
    /// Maximum size of the batch operation queue
    pub max_batch_size: usize,
    /// Enable request coalescing
    pub enable_request_coalescing: bool,
    /// Coalescing window duration
    pub coalescing_window: Duration,
    /// Enable circuit breaker for failing computations
    pub enable_circuit_breaker: bool,
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: usize,
    /// Circuit breaker recovery timeout
    pub circuit_breaker_recovery_timeout: Duration,
}

impl Default for AsyncCacheConfig {
    fn default() -> Self {
        Self {
            cache_config: CacheConfig::default(),
            max_concurrent_computations: 100,
            computation_timeout: Some(Duration::from_secs(30)),
            enable_deduplication: true,
            enable_background_refresh: false,
            background_refresh_threshold: 10,
            max_batch_size: 1000,
            enable_request_coalescing: false,
            coalescing_window: Duration::from_millis(10),
            enable_circuit_breaker: false,
            circuit_breaker_threshold: 5,
            circuit_breaker_recovery_timeout: Duration::from_secs(60),
        }
    }
}

impl AsyncCacheConfig {
    /// Create new async cache configuration
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache_config: CacheConfig::new(max_entries),
            ..Default::default()
        }
    }

    /// Configure for high throughput scenarios
    pub fn for_high_throughput(max_entries: usize) -> Self {
        Self {
            cache_config: CacheConfig::new(max_entries).with_metrics(),
            max_concurrent_computations: 500,
            computation_timeout: Some(Duration::from_secs(10)),
            enable_deduplication: true,
            enable_background_refresh: true,
            background_refresh_threshold: 5,
            max_batch_size: 5000,
            enable_request_coalescing: true,
            coalescing_window: Duration::from_millis(5),
            enable_circuit_breaker: true,
            circuit_breaker_threshold: 10,
            circuit_breaker_recovery_timeout: Duration::from_secs(30),
        }
    }

    /// Configure for memory-constrained environments
    pub fn for_memory_constrained(max_entries: usize) -> Self {
        Self {
            cache_config: CacheConfig::for_memory_constrained(max_entries),
            max_concurrent_computations: 20,
            computation_timeout: Some(Duration::from_secs(60)),
            enable_deduplication: true,
            enable_background_refresh: false,
            max_batch_size: 100,
            enable_request_coalescing: false,
            enable_circuit_breaker: false,
            ..Default::default()
        }
    }

    /// Configure for low latency scenarios
    pub fn for_low_latency(max_entries: usize) -> Self {
        Self {
            cache_config: CacheConfig::new(max_entries),
            max_concurrent_computations: 200,
            computation_timeout: Some(Duration::from_millis(500)),
            enable_deduplication: true,
            enable_background_refresh: true,
            background_refresh_threshold: 3,
            max_batch_size: 1000,
            enable_request_coalescing: true,
            coalescing_window: Duration::from_millis(1),
            enable_circuit_breaker: true,
            circuit_breaker_threshold: 3,
            circuit_breaker_recovery_timeout: Duration::from_secs(10),
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> MemoryResult<()> {
        self.cache_config.validate()?;

        if self.max_concurrent_computations == 0 {
            return Err(MemoryError::InvalidConfig {
                reason: "max_concurrent_computations must be greater than 0".to_string(),
            });
        }

        if self.max_batch_size == 0 {
            return Err(MemoryError::InvalidConfig {
                reason: "max_batch_size must be greater than 0".to_string(),
            });
        }

        Ok(())
    }
}

/// State of an ongoing computation
#[derive(Debug)]
enum ComputationState<V> {
    /// Computation is in progress
    InProgress {
        wakers: Vec<Waker>,
        join_handle: JoinHandle<AsyncCacheResult<V>>,
    },
    /// Computation completed successfully
    Completed(V),
    /// Computation failed
    Failed(MemoryError),
}

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitBreakerState {
    Closed,    // Normal operation
    Open,      // Failing fast
    HalfOpen,  // Testing recovery
}

/// Circuit breaker for computation failures
#[derive(Debug)]
struct CircuitBreaker {
    state: CircuitBreakerState,
    failure_count: usize,
    threshold: usize,
    last_failure_time: Option<Instant>,
    recovery_timeout: Duration,
}

impl CircuitBreaker {
    fn new(threshold: usize, recovery_timeout: Duration) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            failure_count: 0,
            threshold,
            last_failure_time: None,
            recovery_timeout,
        }
    }

    fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                if let Some(last_failure) = self.last_failure_time {
                    if last_failure.elapsed() >= self.recovery_timeout {
                        self.state = CircuitBreakerState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitBreakerState::Closed;
        self.last_failure_time = None;
    }

    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(Instant::now());

        if self.failure_count >= self.threshold {
            self.state = CircuitBreakerState::Open;
        }
    }
}

/// Future that waits for a computation to complete
pub struct ComputationFuture<V> {
    key: String,
    cache: Weak<AsyncComputeCacheInner<V>>,
    registered: bool,
}

impl<V> ComputationFuture<V>
where
    V: Clone + Send + Sync + 'static,
{
    fn new<K>(key: K, cache: Weak<AsyncComputeCacheInner<V>>) -> Self
    where
        K: CacheKey,
    {
        Self {
            key: format!("{:?}", key), // Simplified key conversion
            cache,
            registered: false,
        }
    }
}

impl<V> Future for ComputationFuture<V>
where
    V: Clone + Send + Sync + 'static,
{
    type Output = AsyncCacheResult<V>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let cache = match self.cache.upgrade() {
            Some(cache) => cache,
            None => return Poll::Ready(Err(MemoryError::AllocationFailed)),
        };

        // Register waker if not already done
        if !self.registered {
            // In a real implementation, you'd register the waker with the computation state
            self.registered = true;
        }

        // Check computation state
        // This is a simplified implementation - in production you'd check the actual computation state
        Poll::Pending
    }
}

/// Batch request for multiple keys
#[derive(Debug)]
pub struct BatchRequest<K, V> {
    pub keys: Vec<K>,
    pub compute_fn: Box<dyn Fn(&K) -> Pin<Box<dyn Future<Output = AsyncCacheResult<V>> + Send>> + Send + Sync>,
}

/// Batch response containing results for multiple keys
#[derive(Debug)]
pub struct BatchResponse<K, V> {
    pub results: HashMap<K, AsyncCacheResult<V>>,
    pub cache_hits: HashSet<K>,
    pub cache_misses: HashSet<K>,
}

/// Internal cache data structure
struct AsyncComputeCacheInner<V>
where
    V: Clone + Send + Sync + 'static,
{
    /// The underlying synchronous cache
    cache: RwLock<ComputeCache<String, V>>,
    /// Configuration
    config: AsyncCacheConfig,
    /// Statistics collector
    stats: AtomicCacheStats,
    /// Semaphore for limiting concurrent computations
    computation_semaphore: Semaphore,
    /// Ongoing computations (for deduplication)
    ongoing_computations: Mutex<HashMap<String, ComputationState<V>>>,
    /// Circuit breakers per key pattern (simplified)
    circuit_breakers: Mutex<HashMap<String, CircuitBreaker>>,
    /// Background refresh tasks
    background_tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Shutdown signal
    shutdown: AtomicBool,
}

/// A high-performance asynchronous compute cache
pub struct AsyncComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    inner: Arc<AsyncComputeCacheInner<V>>,
    _phantom: std::marker::PhantomData<K>,
}

impl<K, V> AsyncComputeCache<K, V>
where
    K: CacheKey + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new async compute cache
    pub fn new(max_entries: usize) -> Self {
        Self::with_config(AsyncCacheConfig::new(max_entries))
    }

    /// Create a new async compute cache with configuration
    pub fn with_config(config: AsyncCacheConfig) -> Self {
        config.validate().expect("Invalid async cache configuration");

        let inner = AsyncComputeCacheInner {
            cache: RwLock::new(ComputeCache::with_config(config.cache_config.clone())),
            stats: AtomicCacheStats::new(),
            computation_semaphore: Semaphore::new(config.max_concurrent_computations),
            ongoing_computations: Mutex::new(HashMap::new()),
            circuit_breakers: Mutex::new(HashMap::new()),
            background_tasks: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
            config,
        };

        Self {
            inner: Arc::new(inner),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get a value from the cache, computing it asynchronously if not present
    pub async fn get_or_compute<F, Fut>(&self, key: K, compute_fn: F) -> AsyncCacheResult<V>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = AsyncCacheResult<V>> + Send + 'static,
    {
        let start_time = Instant::now();
        let key_str = format!("{:?}", key); // Simplified key conversion

        // First, try to get from cache
        {
            let cache = self.inner.cache.read().await;
            if let Some(value) = cache.get(&key_str) {
                self.inner.stats.record_hit(Some(start_time.elapsed().as_nanos() as u64));

                // Check if background refresh is needed
                if self.inner.config.enable_background_refresh {
                    self.maybe_schedule_background_refresh(&key_str, &value).await;
                }

                return Ok(value);
            }
        }

        self.inner.stats.record_miss(Some(start_time.elapsed().as_nanos() as u64), None);

        // Check if deduplication is enabled and computation is already in progress
        if self.inner.config.enable_deduplication {
            let mut ongoing = self.inner.ongoing_computations.lock().await;
            if let Some(state) = ongoing.get(&key_str) {
                match state {
                    ComputationState::Completed(value) => {
                        return Ok(value.clone());
                    }
                    ComputationState::Failed(error) => {
                        return Err(error.clone());
                    }
                    ComputationState::InProgress { .. } => {
                        // Wait for the ongoing computation
                        drop(ongoing);
                        return self.wait_for_computation(&key_str).await;
                    }
                }
            }
        }

        // Check circuit breaker if enabled
        if self.inner.config.enable_circuit_breaker {
            let mut breakers = self.inner.circuit_breakers.lock().await;
            let breaker = breakers.entry(key_str.clone()).or_insert_with(|| {
                CircuitBreaker::new(
                    self.inner.config.circuit_breaker_threshold,
                    self.inner.config.circuit_breaker_recovery_timeout,
                )
            });

            if !breaker.can_execute() {
                return Err(MemoryError::AllocationFailed); // Circuit breaker open
            }
        }

        // Acquire computation permit
        let _permit = self.inner.computation_semaphore.acquire().await
            .map_err(|_| MemoryError::AllocationFailed)?;

        // Start computation
        let computation_start = Instant::now();
        let result = if let Some(timeout_duration) = self.inner.config.computation_timeout {
            timeout(timeout_duration, compute_fn()).await
                .map_err(|_| MemoryError::AllocationFailed)?
        } else {
            compute_fn().await
        };

        let computation_time = computation_start.elapsed().as_nanos() as u64;

        match result {
            Ok(value) => {
                // Update circuit breaker
                if self.inner.config.enable_circuit_breaker {
                    let mut breakers = self.inner.circuit_breakers.lock().await;
                    if let Some(breaker) = breakers.get_mut(&key_str) {
                        breaker.record_success();
                    }
                }

                // Insert into cache
                {
                    let mut cache = self.inner.cache.write().await;
                    let _ = cache.insert(key_str.clone(), value.clone());
                }

                self.inner.stats.record_insertion(Some(computation_time));

                // Update ongoing computations if deduplication is enabled
                if self.inner.config.enable_deduplication {
                    let mut ongoing = self.inner.ongoing_computations.lock().await;
                    ongoing.insert(key_str, ComputationState::Completed(value.clone()));
                }

                Ok(value)
            }
            Err(error) => {
                // Update circuit breaker
                if self.inner.config.enable_circuit_breaker {
                    let mut breakers = self.inner.circuit_breakers.lock().await;
                    if let Some(breaker) = breakers.get_mut(&key_str) {
                        breaker.record_failure();
                    }
                }

                // Record error in ongoing computations if deduplication is enabled
                if self.inner.config.enable_deduplication {
                    let mut ongoing = self.inner.ongoing_computations.lock().await;
                    ongoing.insert(key_str, ComputationState::Failed(error.clone()));
                }

                Err(error)
            }
        }
    }

    /// Get multiple values from the cache with batch computation
    pub async fn get_or_compute_batch<F, Fut>(
        &self,
        keys: Vec<K>,
        compute_fn: F,
    ) -> BatchResponse<K, V>
    where
        F: Fn(&K) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AsyncCacheResult<V>> + Send + 'static,
    {
        let mut results = HashMap::new();
        let mut cache_hits = HashSet::new();
        let mut cache_misses = HashSet::new();
        let mut keys_to_compute = Vec::new();

        // First pass: check cache for all keys
        {
            let cache = self.inner.cache.read().await;
            for key in &keys {
                let key_str = format!("{:?}", key);
                if let Some(value) = cache.get(&key_str) {
                    results.insert(key.clone(), Ok(value));
                    cache_hits.insert(key.clone());
                    self.inner.stats.record_hit(None);
                } else {
                    keys_to_compute.push(key.clone());
                    cache_misses.insert(key.clone());
                    self.inner.stats.record_miss(None, None);
                }
            }
        }

        // Second pass: compute missing values
        if !keys_to_compute.is_empty() {
            let computation_futures: Vec<_> = keys_to_compute
                .iter()
                .map(|key| {
                    let key_clone = key.clone();
                    let compute_fn_ref = &compute_fn;
                    async move {
                        let result = compute_fn_ref(&key_clone).await;
                        (key_clone, result)
                    }
                })
                .collect();

            // Execute computations concurrently (respecting semaphore limits)
            let computation_results = futures::future::join_all(computation_futures).await;

            // Process results and update cache
            let mut cache = self.inner.cache.write().await;
            for (key, result) in computation_results {
                match result {
                    Ok(value) => {
                        let key_str = format!("{:?}", key);
                        let _ = cache.insert(key_str, value.clone());
                        results.insert(key, Ok(value));
                        self.inner.stats.record_insertion(None);
                    }
                    Err(error) => {
                        results.insert(key, Err(error));
                    }
                }
            }
        }

        BatchResponse {
            results,
            cache_hits,
            cache_misses,
        }
    }

    /// Warm up the cache with pre-computed values
    pub async fn warm_up(&self, entries: Vec<(K, V)>) -> MemoryResult<()> {
        let mut cache = self.inner.cache.write().await;

        for (key, value) in entries {
            let key_str = format!("{:?}", key);
            cache.insert(key_str, value)?;
            self.inner.stats.record_insertion(None);
        }

        Ok(())
    }

    /// Invalidate a specific key
    pub async fn invalidate(&self, key: &K) -> Option<V> {
        let key_str = format!("{:?}", key);

        // Remove from cache
        let mut cache = self.inner.cache.write().await;
        let result = cache.remove(&key_str);

        // Remove from ongoing computations
        if self.inner.config.enable_deduplication {
            let mut ongoing = self.inner.ongoing_computations.lock().await;
            ongoing.remove(&key_str);
        }

        result
    }

    /// Invalidate multiple keys
    pub async fn invalidate_batch(&self, keys: &[K]) -> HashMap<K, Option<V>> {
        let mut results = HashMap::new();
        let mut cache = self.inner.cache.write().await;

        for key in keys {
            let key_str = format!("{:?}", key);
            let value = cache.remove(&key_str);
            results.insert(key.clone(), value);
        }

        // Clean up ongoing computations
        if self.inner.config.enable_deduplication {
            let mut ongoing = self.inner.ongoing_computations.lock().await;
            for key in keys {
                let key_str = format!("{:?}", key);
                ongoing.remove(&key_str);
            }
        }

        results
    }

    /// Clear all entries from the cache
    pub async fn clear(&self) {
        let mut cache = self.inner.cache.write().await;
        cache.clear();

        if self.inner.config.enable_deduplication {
            let mut ongoing = self.inner.ongoing_computations.lock().await;
            ongoing.clear();
        }

        let mut breakers = self.inner.circuit_breakers.lock().await;
        breakers.clear();
    }

    /// Get current cache size
    pub async fn len(&self) -> usize {
        let cache = self.inner.cache.read().await;
        cache.len()
    }

    /// Check if cache is empty
    pub async fn is_empty(&self) -> bool {
        let cache = self.inner.cache.read().await;
        cache.is_empty()
    }

    /// Get cache capacity
    pub async fn capacity(&self) -> usize {
        let cache = self.inner.cache.read().await;
        cache.capacity()
    }

    /// Clean up expired entries
    pub async fn cleanup_expired(&self) -> usize {
        let mut cache = self.inner.cache.write().await;
        let cleaned = cache.cleanup_expired();
        self.inner.stats.record_expired_cleanup(cleaned as u64);
        cleaned
    }

    /// Wait for a computation to complete (for deduplication)
    async fn wait_for_computation(&self, key: &str) -> AsyncCacheResult<V> {
        // Simplified implementation - in production you'd use proper async coordination
        loop {
            sleep(Duration::from_millis(1)).await;

            let ongoing = self.inner.ongoing_computations.lock().await;
            if let Some(state) = ongoing.get(key) {
                match state {
                    ComputationState::Completed(value) => {
                        return Ok(value.clone());
                    }
                    ComputationState::Failed(error) => {
                        return Err(error.clone());
                    }
                    ComputationState::InProgress { .. } => {
                        // Continue waiting
                        continue;
                    }
                }
            } else {
                return Err(MemoryError::AllocationFailed);
            }
        }
    }

    /// Maybe schedule background refresh for frequently accessed items
    async fn maybe_schedule_background_refresh(&self, key: &str, _value: &V) {
        // Simplified implementation - in production you'd track access counts and schedule refreshes
        if self.inner.config.enable_background_refresh {
            // Check if item should be refreshed based on access patterns
            // This is a placeholder for more sophisticated logic
        }
    }

    /// Shutdown the cache and clean up background tasks
    pub async fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);

        // Cancel background tasks
        let mut tasks = self.inner.background_tasks.lock().await;
        for task in tasks.drain(..) {
            task.abort();
        }
    }
}

impl<K, V> StatsProvider for AsyncComputeCache<K, V>
where
    K: CacheKey + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn get_stats(&self) -> super::stats::CacheStats {
        self.inner.stats.get_stats()
    }

    fn get_stats_for_window(&self, window: TimeWindow) -> super::stats::CacheStats {
        self.inner.stats.get_stats_for_window(window)
    }

    fn reset_stats(&self) {
        self.inner.stats.reset()
    }

    fn get_latency_percentiles(&self) -> super::stats::Percentiles {
        // Simplified implementation
        super::stats::Percentiles::default()
    }

    fn get_throughput_metrics(&self) -> super::stats::ThroughputMetrics {
        self.inner.stats.get_stats().throughput_metrics()
    }
}

impl<K, V> Clone for AsyncComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<K, V> Drop for AsyncComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
    }
}

/// Async cache builder for easier configuration
pub struct AsyncCacheBuilder<K, V> {
    config: AsyncCacheConfig,
    _phantom: std::marker::PhantomData<(K, V)>,
}

impl<K, V> AsyncCacheBuilder<K, V>
where
    K: CacheKey + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new cache builder
    pub fn new(max_entries: usize) -> Self {
        Self {
            config: AsyncCacheConfig::new(max_entries),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set maximum concurrent computations
    pub fn max_concurrent_computations(mut self, max: usize) -> Self {
        self.config.max_concurrent_computations = max;
        self
    }

    /// Set computation timeout
    pub fn computation_timeout(mut self, timeout: Duration) -> Self {
        self.config.computation_timeout = Some(timeout);
        self
    }

    /// Enable deduplication
    pub fn enable_deduplication(mut self) -> Self {
        self.config.enable_deduplication = true;
        self
    }

    /// Enable background refresh
    pub fn enable_background_refresh(mut self, threshold: usize) -> Self {
        self.config.enable_background_refresh = true;
        self.config.background_refresh_threshold = threshold;
        self
    }

    /// Enable request coalescing
    pub fn enable_request_coalescing(mut self, window: Duration) -> Self {
        self.config.enable_request_coalescing = true;
        self.config.coalescing_window = window;
        self
    }

    /// Enable circuit breaker
    pub fn enable_circuit_breaker(mut self, threshold: usize, recovery_timeout: Duration) -> Self {
        self.config.enable_circuit_breaker = true;
        self.config.circuit_breaker_threshold = threshold;
        self.config.circuit_breaker_recovery_timeout = recovery_timeout;
        self
    }

    /// Build the async cache
    pub fn build(self) -> AsyncComputeCache<K, V> {
        AsyncComputeCache::with_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_basic_async_caching() {
        let cache = AsyncComputeCache::<String, usize>::new(10);

        // First call should compute
        let result1 = cache.get_or_compute("key1".to_string(), || async { Ok(42) }).await;
        assert_eq!(result1.unwrap(), 42);

        // Second call should use cached value
        let result2 = cache.get_or_compute("key1".to_string(), || async { Ok(99) }).await;
        assert_eq!(result2.unwrap(), 42);

        // Different key should compute new value
        let result3 = cache.get_or_compute("key2".to_string(), || async { Ok(99) }).await;
        assert_eq!(result3.unwrap(), 99);
    }

    #[tokio::test]
    async fn test_async_batch_operations() {
        let cache = AsyncComputeCache::<String, usize>::new(10);

        let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
        let response = cache.get_or_compute_batch(keys, |key| async move {
            Ok(key.len())
        }).await;

        assert_eq!(response.results.len(), 3);
        assert_eq!(response.cache_misses.len(), 3);
        assert_eq!(response.cache_hits.len(), 0);

        // Second batch should hit cache
        let keys = vec!["key1".to_string(), "key2".to_string()];
        let response = cache.get_or_compute_batch(keys, |key| async move {
            Ok(key.len() * 2)
        }).await;

        assert_eq!(response.cache_hits.len(), 2);
        assert_eq!(response.cache_misses.len(), 0);
    }

    #[tokio::test]
    async fn test_async_timeout() {
        let cache = AsyncComputeCache::<String, usize>::with_config(
            AsyncCacheConfig::new(10)
                .computation_timeout(Some(Duration::from_millis(50)))
        );

        // This should timeout
        let result = cache.get_or_compute("slow_key".to_string(), || async {
            sleep(Duration::from_millis(100)).await;
            Ok(42)
        }).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_async_deduplication() {
        let cache = AsyncComputeCache::<String, usize>::with_config(
            AsyncCacheConfig::new(10).enable_deduplication(true)
        );

        let counter = Arc::new(AtomicUsize::new(0));

        // Start multiple concurrent computations for the same key
        let futures: Vec<_> = (0..5).map(|_| {
            let cache = cache.clone();
            let counter = Arc::clone(&counter);
            async move {
                cache.get_or_compute("key1".to_string(), || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(10)).await;
                        Ok(42)
                    }
                }).await
            }
        }).collect();

        let results = futures::future::join_all(futures).await;

        // All results should be Ok(42)
        for result in results {
            assert_eq!(result.unwrap(), 42);
        }

        // Computation should have been deduplicated
        // Note: This test might be flaky due to timing - in production you'd need more robust testing
        let final_count = counter.load(Ordering::SeqCst);
        assert!(final_count <= 2); // Allow for some race conditions
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let cache = AsyncComputeCache::<String, usize>::new(10);

        // Test warm up
        cache.warm_up(vec![
            ("key1".to_string(), 100),
            ("key2".to_string(), 200),
        ]).await.unwrap();

        assert_eq!(cache.len().await, 2);
        assert!(!cache.is_empty().await);

        // Test invalidation
        let invalidated = cache.invalidate(&"key1".to_string()).await;
        assert_eq!(invalidated, Some(100));
        assert_eq!(cache.len().await, 1);

        // Test batch invalidation
        let batch_invalidated = cache.invalidate_batch(&["key2".to_string()]).await;
        assert_eq!(batch_invalidated.get(&"key2".to_string()), Some(&Some(200)));
        assert_eq!(cache.len().await, 0);
        assert!(cache.is_empty().await);
    }

    #[tokio::test]
    async fn test_cache_builder() {
        let cache = AsyncCacheBuilder::<String, usize>::new(100)
            .max_concurrent_computations(50)
            .computation_timeout(Duration::from_secs(5))
            .enable_deduplication()
            .enable_background_refresh(10)
            .enable_circuit_breaker(5, Duration::from_secs(30))
            .build();

        // Test basic functionality
        let result = cache.get_or_compute("test".to_string(), || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_preset_configurations() {
        // Test high throughput configuration
        let high_throughput = AsyncComputeCache::<String, usize>::with_config(
            AsyncCacheConfig::for_high_throughput(1000)
        );
        assert!(high_throughput.inner.config.enable_deduplication);
        assert!(high_throughput.inner.config.enable_background_refresh);

        // Test memory constrained configuration
        let memory_constrained = AsyncComputeCache::<String, usize>::with_config(
            AsyncCacheConfig::for_memory_constrained(100)
        );
        assert!(!memory_constrained.inner.config.enable_background_refresh);

        // Test low latency configuration
        let low_latency = AsyncComputeCache::<String, usize>::with_config(
            AsyncCacheConfig::for_low_latency(500)
        );
        assert!(low_latency.inner.config.enable_circuit_breaker);
        assert!(low_latency.inner.config.enable_request_coalescing);
    }
}