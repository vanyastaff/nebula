//! Custom validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// Custom validator - wraps a custom validation function
pub struct Custom<F> {
    validator_fn: F,
    name: String,
    description: Option<String>,
}

impl<F> Custom<F> {
    pub fn new(validator_fn: F) -> Self {
        Self {
            validator_fn,
            name: "custom".to_string(),
            description: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[async_trait]
impl<F> Validatable for Custom<F>
where
    F: Fn(&Value) -> ValidationResult<()> + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        (self.validator_fn)(value)
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: &self.name,
            description: self.description.as_deref(),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom"],
        }
    }
}

/// AsyncCustom validator - wraps an async custom validation function
pub struct AsyncCustom<F> {
    validator_fn: F,
    name: String,
    description: Option<String>,
}

impl<F> AsyncCustom<F> {
    pub fn new(validator_fn: F) -> Self {
        Self {
            validator_fn,
            name: "async_custom".to_string(),
            description: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[async_trait]
impl<F> Validatable for AsyncCustom<F>
where
    F: Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = ValidationResult<()>> + Send + '_>> + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        (self.validator_fn)(value).await
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: &self.name,
            description: self.description.as_deref(),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "async"],
        }
    }
}

/// Lazy validator - creates a validator on demand
pub struct Lazy<F> {
    factory: F,
    name: String,
    description: Option<String>,
}

impl<F> Lazy<F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            name: "lazy".to_string(),
            description: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[async_trait]
impl<F> Validatable for Lazy<F>
where
    F: Fn() -> Box<dyn Validatable + Send + Sync> + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let validator = (self.factory)();
        validator.validate(value).await
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: &self.name,
            description: self.description.as_deref(),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "lazy"],
        }
    }
}

/// Cached validator - caches validation results
pub struct Cached<V> {
    validator: V,
    cache: std::sync::Mutex<std::collections::HashMap<String, ValidationResult<()>>>,
    cache_key_fn: Box<dyn Fn(&Value) -> String + Send + Sync>,
}

impl<V> Cached<V> {
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            cache_key_fn: Box::new(|value| serde_json::to_string(value).unwrap_or_default()),
        }
    }

    pub fn with_cache_key<F>(mut self, cache_key_fn: F) -> Self
    where
        F: Fn(&Value) -> String + Send + Sync + 'static,
    {
        self.cache_key_fn = Box::new(cache_key_fn);
        self
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync> Validatable for Cached<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let cache_key = (self.cache_key_fn)(value);
        
        // Check cache first
        if let Ok(cache) = self.cache.lock() {
            if let Some(cached_result) = cache.get(&cache_key) {
                return cached_result.clone();
            }
        }
        
        // Perform validation
        let result = self.validator.validate(value).await;
        
        // Cache the result
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(cache_key, result.clone());
        }
        
        result
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "cached",
            description: Some("Caches validation results for performance"),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "cached"],
        }
    }
}

/// Retry validator - retries validation on failure
pub struct Retry<V> {
    validator: V,
    max_attempts: usize,
    delay_ms: u64,
}

impl<V> Retry<V> {
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            max_attempts: 3,
            delay_ms: 100,
        }
    }

    pub fn max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    pub fn delay_ms(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync> Validatable for Retry<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let mut last_error = None;
        
        for attempt in 1..=self.max_attempts {
            match self.validator.validate(value).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    last_error = Some(error);
                    
                    if attempt < self.max_attempts {
                        // Wait before retrying
                        tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
                    }
                }
            }
        }
        
        // All attempts failed
        Err(last_error.unwrap_or_else(|| {
            ValidationError::new(
                ErrorCode::InternalError,
                "Validation failed after all retry attempts"
            )
        }))
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "retry",
            description: Some(&format!("Retries validation up to {} times with {}ms delay", 
                self.max_attempts, self.delay_ms)),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "retry"],
        }
    }
}

/// Timeout validator - applies a timeout to validation
pub struct Timeout<V> {
    validator: V,
    timeout_ms: u64,
}

impl<V> Timeout<V> {
    pub fn new(validator: V, timeout_ms: u64) -> Self {
        Self {
            validator,
            timeout_ms,
        }
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync> Validatable for Timeout<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let timeout_duration = tokio::time::Duration::from_millis(self.timeout_ms);
        
        match tokio::time::timeout(timeout_duration, self.validator.validate(value)).await {
            Ok(result) => result,
            Err(_) =>                 Err(ValidationError::new(
                    ErrorCode::InternalError,
                    format!("Validation timed out after {}ms", self.timeout_ms)
                )),
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "timeout",
            description: Some(&format!("Validation must complete within {}ms", self.timeout_ms)),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "timeout"],
        }
    }
}

/// Fallback validator - provides fallback validation on failure
pub struct Fallback<V, F> {
    primary: V,
    fallback: F,
}

impl<V, F> Fallback<V, F> {
    pub fn new(primary: V, fallback: F) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync, F: Validatable + Send + Sync> Validatable for Fallback<V, F> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match self.primary.validate(value).await {
            Ok(()) => Ok(()),
            Err(_) => {
                // Primary validation failed, try fallback
                self.fallback.validate(value).await
            }
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "fallback",
            description: Some("Uses fallback validator if primary fails"),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "fallback"],
        }
    }
}

/// Transform validator - transforms value before validation
pub struct Transform<V, T> {
    validator: V,
    transform: T,
}

impl<V, T> Transform<V, T> {
    pub fn new(validator: V, transform: T) -> Self {
        Self { validator, transform }
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync, T> Validatable for Transform<V, T>
where
    T: Fn(&Value) -> Value + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let transformed = (self.transform)(value);
        self.validator.validate(&transformed).await
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "transform",
            description: Some("Transforms value before validation"),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "transform"],
        }
    }
}

/// AsyncTransform validator - async transforms value before validation
pub struct AsyncTransform<V, T> {
    validator: V,
    transform: T,
}

impl<V, T> AsyncTransform<V, T> {
    pub fn new(validator: V, transform: T) -> Self {
        Self { validator, transform }
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync, T> Validatable for AsyncTransform<V, T>
where
    T: Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + '_>> + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let transformed = (self.transform)(value).await;
        self.validator.validate(&transformed).await
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "async_transform",
            description: Some("Async transforms value before validation"),
            category: crate::ValidatorCategory::Custom,
            tags: vec!["custom", "async_transform"],
        }
    }
}
