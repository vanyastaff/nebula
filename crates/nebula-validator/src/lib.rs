//! Nebula Validator - Production-ready validation framework with advanced combinators and cross-field validation

// Core modules
pub mod types;        // Core data types and structures
pub mod traits;       // Core validation traits and interfaces
pub mod context;      // Validation context and state management
pub mod registry;     // Validator registration and discovery
pub mod cache;        // Result caching system
pub mod metrics;      // Performance metrics and monitoring
pub mod builder;      // Fluent builder API with type safety
pub mod core;         // Core validation types and systems

// Validators module (concrete implementations)
pub mod validators;

// Pipeline module for complex validation workflows
pub mod pipeline;

// Test module for pipeline
#[cfg(test)]
mod pipeline_test;

// Re-export core types from types module
pub use types::{
    ValidatorId, ValidationResult, ValidatorMetadata, ValidationMetadata,
    ValidatorCategory, ValidationComplexity, ValidationConfig,
    ValidationError, ErrorCode, ErrorSeverity,
    CacheInfo, PerformanceMetrics,
};

// Re-export core traits from traits module
pub use traits::{
    Validatable, ValidatableExt, StateAwareValidator, ContextAwareValidator,
    CompositeValidator, AsyncValidator, CachingValidator, PerformanceAwareValidator,
    ErrorAwareValidator,
};

// Re-export context types
pub use context::{
    ValidationContext, FullValidationContext, ValidationMode,
    ValidationState, ValidationStatus, StateStats, ValidationStrategy,
};

// Re-export registry functionality
pub use registry::{
    ValidatorRegistry, RegistryBuilder, RegistryExt, RegistryStats, RegistryError,
};

// Re-export cache functionality
pub use cache::{
    ValidationCache, CacheBuilder, CacheConfig, EvictionPolicy, CacheStats, CacheError,
};

// Re-export metrics functionality
pub use metrics::{
    MetricsRegistry, MetricsBuilder, ValidationMetrics, CacheMetrics, SystemMetrics, AllMetrics,
};

// Re-export core types
pub use core::{
    Validated, Valid, Invalid, ValidationProof, ProofType,
    CoreError, CoreResult, ValidatedExt, ProofExt,
};

// Re-export common validators from validators module
pub use validators::{
    // Logical combinators (primary exports with short names)
    And, Or, Not, Xor, When,
        // Async validators with production features
    Timeout, CircuitBreakerValidator, ResiliencePolicyValidator, ResiliencePolicyValidatorBuilder,
    BulkheadValidator, Cached, Parallel, Strategy,
    // Range validators
    Numeric, StringLength, ArrayLength, Custom, Builder,
    numeric_range, string_length_range, array_length_range, range,
    // Conditional validators
    RequiredIf, ForbiddenIf, Equals, In,
    required_if, forbidden_if, eq, in_values,
    // Basic validators
    NotNull, not_null,
    // Enhanced validators
    AlwaysValid, AlwaysInvalid, Predicate, Lazy, Deferred, Memoized, Throttled,
    WhenChain, FieldCondition, Required, Optional,
    WeightedOr, ParallelAnd, EnhancedAll, EnhancedAny,
    RuleComposer, RuleChain, RuleGroup, ComposedRule,
    // ... (will be added as we implement them)
};

// Re-export builder API
pub use builder::{
    ValidationBuilder, CompositeValidator,
    string, numeric, collection, custom,
    Unvalidated, Validated,
};

// Re-export pipeline functionality
pub use pipeline::{
    Pipeline, PipelineConfig, RetryConfig, PipelineStats,
    PipelineBuilder, StageBuilder, StageType, StageConfig,
    PipelineStage, StageExecutionResult, StageError, StageMetrics,
    PipelineExecutor, ExecutionConfig, ExecutionContext,
    PipelineResult, StageResult, ValidatorResult, PipelineError, ErrorCategory,
    PipelineMetricsCollector, ValidatorMetrics, ErrorStatistics, PerformanceStatistics,
    LatencyPercentiles, MetricsReport, MetricsSummary,
};

// Re-export common dependencies
pub use serde_json::Value;
pub use async_trait::async_trait;

// ==================== Constants ====================

/// Framework version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Minimum supported Rust version
pub const MSRV: &str = "1.87";

/// Framework name
pub const FRAMEWORK_NAME: &str = "Nebula Validator";

/// Framework description
pub const FRAMEWORK_DESCRIPTION: &str = "Production-ready validation framework with advanced combinators and cross-field validation";

// ==================== Testing Utilities ====================

#[cfg(test)]
pub mod test_utils {
    use super::*;
    
    /// Create a test validator for testing purposes
    pub fn create_test_validator() -> impl Validatable {
        struct TestValidator;
        
        #[async_trait::async_trait]
        impl Validatable for TestValidator {
            async fn validate(&self, _value: &Value) -> ValidationResult<()> {
                ValidationResult::success(())
            }
            
            fn metadata(&self) -> ValidatorMetadata {
                ValidatorMetadata::new(
                    "test".to_string(),
                    "Test Validator".to_string(),
                    ValidatorCategory::Basic,
                )
            }
            
            fn complexity(&self) -> ValidationComplexity {
                ValidationComplexity::Simple
            }
        }
        
        TestValidator
    }
}
