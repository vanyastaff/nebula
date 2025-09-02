//! Core type definitions for nebula-validator
//! 
//! This module contains all fundamental types used throughout the validation framework.

mod result;
mod error;
mod metadata;
mod complexity;
mod config;
mod id;

// Re-export all types
pub use result::{ValidationResult, BatchValidationResult, StreamValidationResult};
pub use error::{ValidationError, ErrorCode, ErrorSeverity, ErrorDetails, ErrorContext};
pub use metadata::{
    ValidatorMetadata, ValidationMetadata, MetadataBuilder,
    ValidatorCategory, ValidationStats, PerformanceMetrics
};
pub use complexity::{ValidationComplexity, ComplexityEstimator, ComplexityMetrics};
pub use config::{
    ValidationConfig, ConfigBuilder, CacheConfig, 
    PerformanceConfig, RetryConfig, TimeoutConfig
};
pub use id::{
    ValidatorId, ValidationId, ProofId, 
    SessionId, RequestId, TraceId
};

// Common type aliases
pub type ValidationTimestamp = chrono::DateTime<chrono::Utc>;
pub type ValidationDuration = std::time::Duration;

/// Common prelude for types
pub mod prelude {
    pub use super::{
        ValidationResult, ValidationError, ErrorCode,
        ValidatorMetadata, ValidationComplexity,
        ValidationConfig, ValidatorId,
    };
}