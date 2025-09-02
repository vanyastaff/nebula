//! Advanced basic validators for nebula-validator
//! 
//! This module provides enhanced basic validators with improved functionality,
//! including AlwaysValid, AlwaysInvalid, and Predicate validators.

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::traits::Validatable;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::core::{Valid, ValidationProof};
use crate::context::ValidationContext;

// ==================== Always Valid Validator ====================

/// Validator that always succeeds
#[derive(Debug, Clone)]
pub struct AlwaysValid {
    proof_ttl: Option<Duration>,
}

impl AlwaysValid {
    /// Create a new AlwaysValid validator
    pub fn new() -> Self {
        Self { proof_ttl: None }
    }
    
    /// Set TTL for validation proof
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.proof_ttl = Some(ttl);
        self
    }
}

#[async_trait]
impl Validatable for AlwaysValid {
    async fn validate(&self, _value: &Value) -> ValidationResult<()> {
        let _proof = ValidationProof::new(crate::types::ValidatorId::new("always_valid"));
        if let Some(_ttl) = self.proof_ttl {
            // proof = proof.with_ttl(ttl);
        }
        
        // Create a new validation result with proof
        ValidationResult {
            is_valid: true,
            value: Some(()),
            errors: Vec::new(),
            metadata: crate::types::ValidationMetadata::default(),
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "always_valid",
            "Always returns valid",
            crate::types::ValidatorCategory::Basic,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Trivial
    }
}

impl Default for AlwaysValid {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Always Invalid Validator ====================

/// Validator that always fails
#[derive(Debug, Clone)]
pub struct AlwaysInvalid {
    error: ValidationError,
}

impl AlwaysInvalid {
    /// Create a new AlwaysInvalid validator
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: ValidationError::new(
                ErrorCode::new("always_invalid"),
                message,
            ),
        }
    }
    
    /// Create with specific error code and message
    pub fn with_code(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ValidationError::new(code, message),
        }
    }
}

#[async_trait]
impl Validatable for AlwaysInvalid {
    async fn validate(&self, _value: &Value) -> ValidationResult<()> {
        ValidationResult::failure(vec![self.error.clone()])
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "always_invalid",
            "Always returns invalid",
            crate::types::ValidatorCategory::Basic,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Trivial
    }
}

// ==================== Predicate Validator ====================

/// Validator with custom predicate function
#[derive(Clone)]
pub struct Predicate<F> {
    predicate: F,
    error_message: String,
    name: String,
}

impl<F> Predicate<F>
where
    F: Fn(&Value) -> bool + Send + Sync + Clone,
{
    /// Create a new predicate validator
    pub fn new(name: impl Into<String>, predicate: F, error_message: impl Into<String>) -> Self {
        Self {
            predicate,
            error_message: error_message.into(),
            name: name.into(),
        }
    }
}

#[async_trait]
impl<F> Validatable for Predicate<F>
where
    F: Fn(&Value) -> bool + Send + Sync + Clone,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if (self.predicate)(value) {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(vec![
                ValidationError::new(
                    ErrorCode::new("predicate_failed"),
                    &self.error_message,
                )
            ])
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            format!("Predicate validator: {}", self.name),
            crate::types::ValidatorCategory::Basic,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Lazy Validator ====================

/// Lazy validator - created only when needed
pub struct Lazy<F> {
    factory: F,
    cached: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn Validatable>>>>,
}

impl<F> Lazy<F>
where
    F: Fn() -> Box<dyn Validatable> + Send + Sync,
{
    /// Create a new lazy validator
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            cached: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

#[async_trait]
impl<F> Validatable for Lazy<F>
where
    F: Fn() -> Box<dyn Validatable> + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let mut cache = self.cached.write().await;
        if cache.is_none() {
            *cache = Some((self.factory)());
        }
        
        cache.as_ref().unwrap().validate(value).await
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "lazy",
            "Lazy validator",
            crate::types::ValidatorCategory::Advanced,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Complex
    }
}

// ==================== Deferred Validator ====================

/// Deferred validator - executed later
pub struct Deferred {
    validator: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn Validatable>>>>,
}

impl Deferred {
    /// Create a new deferred validator
    pub fn new() -> Self {
        Self {
            validator: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    /// Set validator
    pub async fn set<V: Validatable + 'static>(&self, validator: V) {
        let mut lock = self.validator.write().await;
        *lock = Some(Box::new(validator));
    }
    
    /// Reset validator
    pub async fn reset(&self) {
        let mut lock = self.validator.write().await;
        *lock = None;
    }
}

#[async_trait]
impl Validatable for Deferred {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let lock = self.validator.read().await;
        match lock.as_ref() {
            Some(validator) => validator.validate(value).await,
            None => ValidationResult::success(()), // Default to success
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "deferred",
            "Deferred validator",
            crate::types::ValidatorCategory::Advanced,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Complex
    }
}

impl Default for Deferred {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Memoized Validator ====================

/// Memoized validator - caches results
pub struct Memoized<V> {
    validator: V,
    cache: std::sync::Arc<dashmap::DashMap<u64, (ValidationResult<()>, std::time::Instant)>>,
    ttl: Duration,
    max_entries: usize,
}

impl<V: Validatable> Memoized<V> {
    /// Create a new memoized validator
    pub fn new(validator: V, ttl: Duration) -> Self {
        Self {
            validator,
            cache: std::sync::Arc::new(dashmap::DashMap::new()),
            ttl,
            max_entries: 1000,
        }
    }
    
    /// Set maximum cache entries
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }
    
    /// Calculate hash for value
    fn calculate_hash(&self, value: &Value) -> u64 {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        
        let mut hasher = DefaultHasher::new();
        format!("{:?}", value).hash(&mut hasher);
        hasher.finish()
    }
}

#[async_trait]
impl<V: Validatable> Validatable for Memoized<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let hash = self.calculate_hash(value);
        
        // Check cache
        if let Some(entry) = self.cache.get(&hash) {
            let (ref result, ref timestamp) = *entry;
            if timestamp.elapsed() < self.ttl {
                return result.clone();
            }
        }
        
        // Execute validation
        let result = self.validator.validate(value).await;
        
        // Clean old entries if needed
        if self.cache.len() >= self.max_entries {
            let now = std::time::Instant::now();
            self.cache.retain(|_, (_, timestamp)| {
                now.duration_since(*timestamp) < self.ttl
            });
        }
        
        // Save to cache
        self.cache.insert(hash, (result.clone(), std::time::Instant::now()));
        
        result
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let base = self.validator.metadata();
        ValidatorMetadata::new(
            format!("memoized_{}", base.id.as_str()),
            format!("Memoized {}", base.name),
            crate::types::ValidatorCategory::Performance,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.validator.complexity()
    }
}

// ==================== Throttled Validator ====================

/// Throttled validator - limits call frequency
pub struct Throttled<V> {
    validator: V,
    rate_limiter: std::sync::Arc<RateLimiter>,
}

pub struct RateLimiter {
    max_per_second: u32,
    window: std::sync::Arc<tokio::sync::RwLock<std::collections::VecDeque<std::time::Instant>>>,
}

impl RateLimiter {
    fn new(max_per_second: u32) -> Self {
        Self {
            max_per_second,
            window: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new())),
        }
    }
    
    async fn check_and_update(&self) -> bool {
        let mut window = self.window.write().await;
        let now = std::time::Instant::now();
        
        // Remove old entries
        while let Some(front) = window.front() {
            if now.duration_since(*front).as_secs() >= 1 {
                window.pop_front();
            } else {
                break;
            }
        }
        
        // Check if we can add new entry
        if window.len() < self.max_per_second as usize {
            window.push_back(now);
            true
        } else {
            false
        }
    }
}

impl<V: Validatable> Throttled<V> {
    /// Create a new throttled validator
    pub fn new(validator: V, max_per_second: u32) -> Self {
        Self {
            validator,
            rate_limiter: std::sync::Arc::new(RateLimiter::new(max_per_second)),
        }
    }
}

#[async_trait]
impl<V: Validatable> Validatable for Throttled<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Check rate limit
        if !self.rate_limiter.check_and_update().await {
            return ValidationResult::failure(vec![
                ValidationError::new(
                    ErrorCode::new("rate_limit_exceeded"),
                    "Validation rate limit exceeded",
                )
            ]);
        }
        
        self.validator.validate(value).await
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let base = self.validator.metadata();
        ValidatorMetadata::new(
            format!("throttled_{}", base.id.as_str()),
            format!("Throttled {}", base.name),
            crate::types::ValidatorCategory::Performance,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.validator.complexity()
    }
}
