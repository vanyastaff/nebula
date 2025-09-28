//! Core functionality for nebula-validator
//!
//! This module contains the fundamental types for validation:
//! - `Valid<T>` and `Invalid<T>` for type-safe validation results
//! - Core error types
//! - Main validation trait

mod validity;
mod error;
mod traits;
mod builder;
mod macros;

// Re-export all core types
pub use validity::{Valid, Invalid};
pub use error::{CoreError, CoreResult, ValidationError, ValidatorId};

// Re-export new unified traits
pub use traits::{
    Validator, ValidatorExt, ValidationContext, ValidationComplexity,
    AndValidator, OrValidator, NotValidator, ConditionalValidator,
};

// Re-export builder patterns
pub use builder::{
    ValidationBuilder, BuiltValidator, validate,
};