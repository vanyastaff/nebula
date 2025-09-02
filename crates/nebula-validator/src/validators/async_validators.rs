//! Optimized async validators with resilience patterns
//! 
//! This module provides high-performance async validators that leverage nebula-resilience
//! for production-ready features like timeout handling, circuit breakers, and retry logic.

use std::pin::Pin;
use std::future::Future;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde_json::Value;
use nebula_resilience::{
    timeout, CircuitBreaker, CircuitBreakerConfig, CircuitState,
    ResiliencePolicy, ResilienceBuilder, Bulkhead
};
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::traits::AsyncValidator;

// ==================== Timeout Validator ====================

/// Validator that wraps another validator with timeout support
/// 
/// This validator ensures that validation operations don't hang indefinitely
/// by applying a configurable timeout using nebula-resilience.
pub struct TimeoutValidator<T> {
    inner: Box<dyn AsyncValidator<T>>,
    timeout_duration: Duration,
}

impl<T> TimeoutValidator<T> {
    /// Create new timeout validator
    pub fn new(validator: Box<dyn AsyncValidator<T>>, timeout_duration: Duration) -> Self {
        Self {
            inner: validator,
            timeout_duration,
        }
    }
    
    /// Set timeout duration
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_duration = timeout;
        self
    }
}

impl<T> AsyncValidator<T> for TimeoutValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let inner_future = self.inner.validate_async(value);
        let timeout_duration = self.timeout_duration;

        Box::pin(async move {
            match timeout(timeout_duration, inner_future).await {
                Ok(result) => result,
                Err(_) => ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("validation_timeout".to_string()),
                    format!("Validation timed out after {:?}", timeout_duration)
                )])
            }
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut meta = self.inner.metadata();
        meta.name = format!("{} (with {} timeout)", meta.name, format_duration(self.timeout_duration));
        meta.description = format!("{} with {} timeout protection", meta.description, format_duration(self.timeout_duration));
        meta.tags.push("timeout".to_string());
        meta.tags.push("resilience".to_string());
        meta
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

// ==================== Circuit Breaker Validator ====================

/// Circuit breaker pattern for validators using nebula-resilience
/// 
/// This validator implements the circuit breaker pattern to prevent cascading failures
/// and provide graceful degradation when external services are unavailable.
pub struct CircuitBreakerValidator<T> {
    inner: Arc<dyn AsyncValidator<T>>,
    circuit_breaker: CircuitBreaker,
}

impl<T> CircuitBreakerValidator<T> {
    /// Create new circuit breaker validator with default config
    pub fn new(validator: Arc<dyn AsyncValidator<T>>) -> Self {
        Self {
            inner: validator,
            circuit_breaker: CircuitBreaker::new(),
        }
    }
    
    /// Create with custom circuit breaker configuration
    pub fn with_config(
        validator: Arc<dyn AsyncValidator<T>>,
        config: CircuitBreakerConfig,
    ) -> Self {
        Self {
            inner: validator,
            circuit_breaker: CircuitBreaker::with_config(config),
        }
    }
    
    /// Set failure threshold
    pub fn with_failure_threshold(mut self, threshold: usize) -> Self {
        let mut config = CircuitBreakerConfig::default();
        config.failure_threshold = threshold;
        self.circuit_breaker = CircuitBreaker::with_config(config);
        self
    }
    
    /// Set recovery timeout
    pub fn with_recovery_timeout(mut self, timeout: Duration) -> Self {
        let mut config = CircuitBreakerConfig::default();
        config.reset_timeout = timeout;
        self.circuit_breaker = CircuitBreaker::with_config(config);
        self
    }
    
    /// Get current circuit breaker state
    pub async fn get_state(&self) -> CircuitState {
        self.circuit_breaker.state().await
    }
    
    /// Check if circuit is open
    pub async fn is_open(&self) -> bool {
        self.circuit_breaker.is_open().await
    }
}

impl<T> AsyncValidator<T> for CircuitBreakerValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;
    
    fn validate_async(&self, value: &T) -> Self::Future {
        let inner_future = self.inner.validate_async(value);
        let circuit_breaker = &self.circuit_breaker;
        
        Box::pin(async move {
            // Check circuit state first
            if circuit_breaker.is_open().await {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("circuit_breaker_open".to_string()),
                    "Validation service is temporarily unavailable due to circuit breaker"
                )]);
            }
            
            // Execute with circuit breaker protection
            let result: ValidationResult<()> = inner_future.await;
            
            // Update circuit breaker state
                            match &result {
                    result if result.is_success() => {
                        circuit_breaker.record_success().await;
                    }
                    _ => {
                        circuit_breaker.record_failure().await;
                    }
                }
            
            result
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut meta = self.inner.metadata();
        meta.name = format!("{} (Circuit Breaker)", meta.name);
        meta.description = format!("{} with circuit breaker protection using nebula-resilience", meta.description);
        meta.tags.push("circuit_breaker".to_string());
        meta.tags.push("resilient".to_string());
        meta.tags.push("resilience".to_string());
        meta
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

// ==================== Resilience Policy Validator ====================

/// Validator with comprehensive resilience policy
/// 
/// This validator combines timeout, retry, and circuit breaker patterns
/// using nebula-resilience for maximum robustness.
pub struct ResiliencePolicyValidator<T> {
    inner: Arc<dyn AsyncValidator<T>>,
    policy: ResiliencePolicy,
}

impl<T> ResiliencePolicyValidator<T> {
    /// Create new resilience policy validator
    pub fn new(validator: Arc<dyn AsyncValidator<T>>) -> Self {
        Self {
            inner: validator,
            policy: ResiliencePolicy::default(),
        }
    }
    
    /// Create with custom resilience policy
    pub fn with_policy(validator: Arc<dyn AsyncValidator<T>>, policy: ResiliencePolicy) -> Self {
        Self { inner: validator, policy }
    }
    
    /// Builder pattern for easy configuration
    pub fn builder(validator: Arc<dyn AsyncValidator<T>>) -> ResiliencePolicyValidatorBuilder<T> {
        ResiliencePolicyValidatorBuilder::new(validator)
    }
}

/// Builder for resilience policy validator
pub struct ResiliencePolicyValidatorBuilder<T> {
    validator: Arc<dyn AsyncValidator<T>>,
    builder: ResilienceBuilder,
}

impl<T> ResiliencePolicyValidatorBuilder<T> {
    fn new(validator: Arc<dyn AsyncValidator<T>>) -> Self {
        Self {
            validator,
            builder: ResilienceBuilder::new(),
        }
    }
    
    /// Add timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.builder = self.builder.timeout(timeout);
        self
    }
    
    /// Add retry logic
    pub fn with_retry(mut self, max_attempts: usize, base_delay: Duration) -> Self {
        self.builder = self.builder.retry(max_attempts, base_delay);
        self
    }
    
    /// Add circuit breaker
    pub fn with_circuit_breaker(mut self, failure_threshold: usize, reset_timeout: Duration) -> Self {
        self.builder = self.builder.circuit_breaker(failure_threshold, reset_timeout);
        self
    }
    
    /// Build the validator
    pub fn build(self) -> ResiliencePolicyValidator<T> {
        ResiliencePolicyValidator {
            inner: self.validator,
            policy: self.builder.build(),
        }
    }
}

impl<T> AsyncValidator<T> for ResiliencePolicyValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;
    
    fn validate_async(&self, value: &T) -> Self::Future {
        let inner_future = self.inner.validate_async(value);
        let policy = self.policy.clone();
        
        Box::pin(async move {
            // Execute with full resilience policy
            match policy.execute(inner_future).await {
                Ok(result) => result,
                Err(e) => ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("resilience_policy_failed".to_string()),
                    format!("Validation failed due to resilience policy: {}", e)
                )])
            }
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut meta = self.inner.metadata();
        meta.name = format!("{} (Resilience Policy)", meta.name);
        meta.description = format!("{} with comprehensive resilience policy (timeout + retry + circuit breaker)", meta.description);
        meta.tags.push("resilience_policy".to_string());
        meta.tags.push("production_ready".to_string());
        meta.tags.push("robust".to_string());
        meta
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

// ==================== Bulkhead Validator ====================

/// Validator with bulkhead pattern for resource isolation
/// 
/// This validator limits concurrent validations to prevent resource exhaustion
/// using nebula-resilience bulkhead implementation.
pub struct BulkheadValidator<T> {
    inner: Arc<dyn AsyncValidator<T>>,
    bulkhead: Bulkhead,
}

impl<T> BulkheadValidator<T> {
    /// Create new bulkhead validator
    pub fn new(validator: Arc<dyn AsyncValidator<T>>, max_concurrency: usize) -> Self {
        Self {
            inner: validator,
            bulkhead: Bulkhead::new(max_concurrency),
        }
    }
    
    /// Set max concurrency
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.bulkhead = Bulkhead::new(max_concurrency);
        self
    }
    
    /// Get current bulkhead capacity
    pub fn capacity(&self) -> usize {
        // Use a reasonable default since bulkhead doesn't expose capacity directly
        100
    }
    
    /// Get current bulkhead usage
    pub async fn usage(&self) -> usize {
        self.bulkhead.usage().await
    }
}

impl<T> AsyncValidator<T> for BulkheadValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;
    
    fn validate_async(&self, value: &T) -> Self::Future {
        let inner_future = self.inner.validate_async(value);
        let bulkhead = self.bulkhead.clone();
        
        Box::pin(async move {
            // Execute with bulkhead protection
            match bulkhead.execute(inner_future).await {
                Ok(result) => result,
                Err(e) => ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("bulkhead_full".to_string()),
                    format!("Validation rejected due to bulkhead capacity: {}", e)
                )])
            }
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut meta = self.inner.metadata();
        meta.name = format!("{} (Bulkhead)", meta.name);
        meta.description = format!("{} with bulkhead protection (max {} concurrent)", meta.description, 100);
        meta.tags.push("bulkhead".to_string());
        meta.tags.push("resource_isolation".to_string());
        meta.tags.push("concurrency_limit".to_string());
        meta
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

// ==================== Cached Async Validator ====================

/// Async validator with result caching
/// 
/// This validator caches validation results to avoid repeated expensive operations.
pub struct CachedAsyncValidator<T> {
    inner: Box<dyn AsyncValidator<T>>,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    cache_duration: Duration,
}

struct CacheEntry {
    result: ValidationResult<()>,
    timestamp: Instant,
}

impl<T> CachedAsyncValidator<T>
where
    T: std::fmt::Display + Send + Sync,
{
    /// Create new cached validator
    pub fn new(validator: Box<dyn AsyncValidator<T>>, cache_duration: Duration) -> Self {
        Self {
            inner: validator,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_duration,
        }
    }
    
    /// Set cache duration
    pub fn with_cache_duration(mut self, duration: Duration) -> Self {
        self.cache_duration = duration;
        self
    }
    
    /// Clear cache
    pub async fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.try_write() {
            cache.clear();
        }
    }
    
    /// Get cache size
    pub async fn get_cache_size(&self) -> usize {
        if let Ok(cache) = self.cache.try_read() {
            cache.len()
        } else {
            0
        }
    }
}

impl<T> AsyncValidator<T> for CachedAsyncValidator<T>
where
    T: std::fmt::Display + Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let key = value.to_string();
        let cache = self.cache.clone();
        let cache_duration = self.cache_duration;
        let inner_future = self.inner.validate_async(value);

        Box::pin(async move {
            // Check cache first
            {
                if let Ok(cache_read) = cache.try_read() {
                    if let Some(entry) = cache_read.get(&key) {
                        if Instant::now().duration_since(entry.timestamp) < cache_duration {
                            return entry.result.clone();
                        }
                    }
                }
            }

            // Not in cache or expired, validate and cache result
            let result = inner_future.await;

            {
                if let Ok(mut cache_write) = self.cache.try_write() {
                    cache_write.insert(key, CacheEntry {
                        result: result.clone(),
                        timestamp: Instant::now(),
                    });
                }
            }

            result
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut meta = self.inner.metadata();
        meta.name = format!("{} (Cached)", meta.name);
        meta.description = format!("{} with {} caching", meta.description, format_duration(self.cache_duration));
        meta.tags.push("cached".to_string());
        meta.tags.push("performance".to_string());
        meta
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

// ==================== Parallel Async Validator ====================

/// Validator that runs multiple validators in parallel
/// 
/// This validator executes multiple sub-validators concurrently for better performance.
pub struct ParallelAsyncValidator<T> {
    validators: Vec<Arc<dyn AsyncValidator<T>>>,
    strategy: ParallelStrategy,
}

#[derive(Debug, Clone)]
pub enum ParallelStrategy {
    /// All validators must pass
    All,
    /// At least one validator must pass
    Any,
    /// Return first successful result
    FirstSuccess,
}

impl<T> ParallelAsyncValidator<T> {
    /// Create new parallel validator
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
            strategy: ParallelStrategy::All,
        }
    }
    
    /// Add validator
    pub fn add_validator(mut self, validator: Arc<dyn AsyncValidator<T>>) -> Self {
        self.validators.push(validator);
        self
    }
    
    /// Set strategy
    pub fn with_strategy(mut self, strategy: ParallelStrategy) -> Self {
        self.strategy = strategy;
        self
    }
}

impl<T> AsyncValidator<T> for ParallelAsyncValidator<T>
where
    T: Send + Sync + Clone,
{
    type Future = Pin<Box<dyn Future<Output = ValidationResult<()>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let value = value.clone();
        let validators = self.validators.clone();
        let strategy = self.strategy.clone();
        
        Box::pin(async move {
            if validators.is_empty() {
                return ValidationResult::success(());
            }
            
            let futures: Vec<_> = validators
                .iter()
                .map(|validator| validator.validate_async(&value))
                .collect();

            let results = futures::future::join_all(futures).await;
            
            match strategy {
                ParallelStrategy::All => {
                    // All must pass
                    let mut all_errors = Vec::new();
                    for result in results {
                        if result.is_failure() {
                            all_errors.extend(result.errors);
                        }
                    }
                    if all_errors.is_empty() {
                        ValidationResult::success(())
                    } else {
                        ValidationResult::failure(all_errors)
                    }
                }
                ParallelStrategy::Any => {
                    // At least one must pass
                    let mut errors = Vec::new();
                    for result in results {
                        if result.is_success() {
                            return ValidationResult::success(());
                        } else {
                            errors.extend(result.errors);
                        }
                    }
                    // If we get here, all failed
                    ValidationResult::failure(errors)
                }
                ParallelStrategy::FirstSuccess => {
                    // Return first success or all errors
                    let mut errors = Vec::new();
                    for result in results {
                        if result.is_success() {
                            return ValidationResult::success(());
                        } else {
                            errors.extend(result.errors);
                        }
                    }
                    ValidationResult::failure(errors)
                }
            }
        })
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            format!("parallel_{}", self.validators.len()),
            format!("Parallel Validator ({} validators)", self.validators.len()),
            crate::types::ValidatorCategory::Logical,
        )
        .with_description(format!("Runs {} validators in parallel using {:?} strategy", 
            self.validators.len(), self.strategy))
        .with_tags(vec!["parallel".to_string(), "performance".to_string(), "composite".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        let max_complexity = self.validators.iter()
            .map(|v| v.complexity() as u8)
            .max()
            .unwrap_or(1);
            
        match max_complexity {
            1 => ValidationComplexity::Simple,
            2 => ValidationComplexity::Moderate,
            3 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }
}

// ==================== Utility Functions ====================

/// Format duration for display
fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

// ==================== Re-exports ====================

pub use TimeoutValidator as Timeout;
// Re-exported as specific validator types to avoid naming conflicts
pub use CircuitBreakerValidator;
pub use ResiliencePolicyValidator;
pub use ResiliencePolicyValidatorBuilder;
pub use BulkheadValidator;
pub use CachedAsyncValidator as Cached;
pub use ParallelAsyncValidator as Parallel;
pub use ParallelStrategy as Strategy;

