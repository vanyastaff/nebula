//! Core traits for the Nebula validation framework
//! 
//! This module contains all the fundamental traits that define the validation system's
//! behavior and capabilities.

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity};
use crate::context::{ValidationContext, FullValidationContext, ValidationState, StateStats, ValidationStrategy};

// ==================== Core Validation Trait ====================

/// Core validation trait for all validators
/// 
/// This trait defines the fundamental contract that all validators must implement.
/// It provides a consistent interface for validation operations while allowing
/// for both synchronous and asynchronous validation logic.
#[async_trait]
pub trait Validatable: Send + Sync {
    /// Validate a value
    async fn validate(&self, value: &Value) -> ValidationResult<()>;
    
    /// Get validator metadata
    fn metadata(&self) -> ValidatorMetadata;
    
    /// Get validation complexity
    fn complexity(&self) -> ValidationComplexity;
    
    /// Check if validator supports caching
    fn is_cacheable(&self) -> bool {
        true
    }
    
    /// Validate with additional context
    async fn validate_with_context(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> ValidationResult<()> {
        // Default implementation just calls validate
        self.validate(value).await
    }
}

// ==================== Optimized Async Validator Trait ====================

/// Optimized async validator trait that avoids async-trait allocations
/// 
/// This trait uses associated types to provide better performance for async validators
/// by avoiding the overhead of async-trait.
pub trait AsyncValidator<T>: Send + Sync {
    type Future: std::future::Future<Output = ValidationResult<()>> + Send;

    /// Validate a value asynchronously
    fn validate_async(&self, value: &T) -> Self::Future;
    
    /// Get validator metadata
    fn metadata(&self) -> ValidatorMetadata;
    
    /// Get validation complexity
    fn complexity(&self) -> ValidationComplexity;
    
    /// Check if validator supports caching
    fn is_cacheable(&self) -> bool {
        true
    }
}

// ==================== Extension Trait ====================

/// Extension trait providing fluent validation combinators
/// 
/// This trait adds convenient methods for combining validators using logical operators.
#[async_trait]
pub trait ValidatableExt: Validatable {
    /// Combine with AND logic
    fn and<R>(self, right: R) -> crate::validators::combinators::And<Self, R>
    where
        Self: Sized,
        R: Validatable + Send + Sync,
    {
        crate::validators::combinators::And::new(self, right)
    }
    
    /// Combine with OR logic
    fn or<R>(self, right: R) -> crate::validators::combinators::Or<Self, R>
    where
        Self: Sized,
        R: Validatable + Send + Sync,
    {
        crate::validators::combinators::Or::new(self, right)
    }
    
    /// Negate validator
    fn not(self) -> crate::validators::combinators::Not<Self>
    where
        Self: Sized,
    {
        crate::validators::combinators::Not::new(self)
    }
    
    /// Apply conditionally
    fn when<C>(self, condition: C) -> crate::validators::combinators::When<C, Self>
    where
        Self: Sized,
        C: Validatable + Send + Sync,
    {
        crate::validators::combinators::When::new(condition, self)
    }
}

// ==================== State-Aware Validator ====================

/// Validator that maintains internal state
/// 
/// Useful for validators that need to track validation history or maintain
/// state between validations.
#[async_trait]
pub trait StateAwareValidator: Validatable {
    /// Get current validation state
    fn get_state(&self) -> &ValidationState;
    
    /// Get state statistics
    fn get_stats(&self) -> StateStats;
    
    /// Reset validation state
    fn reset_state(&mut self);
}

// ==================== Context-Aware Validator ====================

/// Validator that requires validation context
/// 
/// Useful for validators that need access to field paths, parent objects,
/// or other contextual information.
#[async_trait]
pub trait ContextAwareValidator: Validatable {
    /// Validate with full context
    async fn validate_with_full_context(
        &self,
        value: &Value,
        context: &FullValidationContext,
    ) -> ValidationResult<()>;
}

// ==================== Composite Validator ====================

/// Validator that combines multiple sub-validators
/// 
/// Useful for complex validation logic that requires multiple validation steps.
#[async_trait]
pub trait CompositeValidator: Validatable {
    /// Get sub-validators
    fn get_validators(&self) -> &[Box<dyn Validatable>];
    
    /// Get validation strategy
    fn get_strategy(&self) -> ValidationStrategy;
}

// ==================== Timeout-Aware Validator ====================

/// Validator that supports timeout configuration
/// 
/// Useful for validators that perform external calls or database operations.
#[async_trait]
pub trait TimeoutAwareValidator: Validatable {
    /// Get timeout duration
    fn get_timeout(&self) -> Duration;
    
    /// Set timeout duration
    fn set_timeout(&mut self, timeout: Duration);
    
    /// Validate with custom timeout
    async fn validate_with_timeout(
        &self,
        value: &Value,
        timeout: Duration,
    ) -> ValidationResult<()>;
}

// ==================== Caching Validator ====================

/// Validator that supports result caching
/// 
/// Useful for validators with expensive operations that can benefit from caching.
#[async_trait]
pub trait CachingValidator: Validatable {
    /// Check if result is cached
    fn is_cached(&self, value: &Value) -> bool;
    
    /// Get cached result
    fn get_cached_result(&self, value: &Value) -> Option<ValidationResult<()>>;
    
    /// Set cache entry
    fn set_cache_entry(&mut self, value: &Value, result: ValidationResult<()>);
    
    /// Clear cache
    fn clear_cache(&mut self);
}

// ==================== Performance-Aware Validator ====================

/// Validator that tracks performance metrics
/// 
/// Useful for monitoring and optimizing validation performance.
#[async_trait]
pub trait PerformanceAwareValidator: Validatable {
    /// Get performance metrics
    fn get_performance_metrics(&self) -> crate::types::PerformanceMetrics;
    
    /// Record validation duration
    fn record_validation_duration(&mut self, duration: Duration);
    
    /// Get average validation time
    fn get_average_validation_time(&self) -> Duration;
}

// ==================== Error-Aware Validator ====================

/// Validator that provides detailed error information
/// 
/// Useful for validators that need to provide specific error details,
/// suggestions, or context.
#[async_trait]
pub trait ErrorAwareValidator: Validatable {
    /// Get error suggestions
    fn get_error_suggestions(&self, error: &ValidationError) -> Vec<String>;
    
    /// Get error context
    fn get_error_context(&self, error: &ValidationError) -> std::collections::HashMap<String, Value>;
    
    /// Can retry validation
    fn can_retry(&self, error: &ValidationError) -> bool;
}

// ==================== Blanket Implementation ====================

/// Blanket implementation for all validators
impl<T: Validatable> ValidatableExt for T {}

// ==================== Re-exports ====================

pub use Validatable as Validator;
pub use ValidatableExt as Ext;
pub use StateAwareValidator as StateAware;
pub use ContextAwareValidator as ContextAware;
pub use CompositeValidator as Composite;
pub use AsyncValidator as Async;
pub use TimeoutAwareValidator as TimeoutAware;
pub use CachingValidator as Caching;
pub use PerformanceAwareValidator as Performance;
pub use ErrorAwareValidator as ErrorAware;
