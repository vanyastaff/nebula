//! Nebula Validator - Production-ready validation framework with advanced combinators and cross-field validation

// Core modules
pub mod types;        // Core data types and structures
pub mod traits;       // Core validation traits and interfaces
pub mod context;      // Validation context and state management
pub mod registry;     // Validator registration and discovery
pub mod cache;        // Result caching system
// pub mod metrics;      // Performance metrics and monitoring
pub mod core;         // Core validation types and systems

// Validators module (concrete implementations)
pub mod validators;

// Builder API
pub mod builder;      // Fluent builder API with type safety

// Pipeline module for complex validation workflows
pub mod pipeline;

// Test module for pipeline
// #[cfg(test)]
// mod pipeline_test;

// Re-export core types from types module
pub use types::{
    ValidatorId, ValidationResult, ValidatorMetadata, ValidationMetadata,
    ValidatorCategory, ValidationComplexity, ValidationConfig,
    ValidationError, ErrorCode, ErrorSeverity,
};

// Re-export core traits from traits module
pub use traits::{
    Validatable, ValidatableExt, StateAwareValidator, ContextAwareValidator,
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
// pub use metrics::{
//     MetricsRegistry, MetricsBuilder, ValidationMetrics, CacheMetrics, SystemMetrics, AllMetrics,
// };

// Re-export core types
pub use core::{
    Validated, Valid, Invalid, ValidationProof, ProofType,
    CoreError, CoreResult, ValidatedExt, ProofExt,
};

// Re-export common validators from validators module (disabled to avoid ambiguous imports)
// Consumers can import specific validators from `crate::validators` directly.

// Re-export builder API (disabled to avoid unresolved items)

// Re-export pipeline functionality (disabled to avoid unresolved items)

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
