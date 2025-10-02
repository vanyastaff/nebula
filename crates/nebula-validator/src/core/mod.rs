//!
//! - Core error types
//! - Main validation trait
//! - `Valid<T>` and `Invalid<T>` for type-safe validation results
//! Core functionality for nebula-validator
//! This module contains the fundamental types for validation:
mod builder;
mod error;
mod macros;
mod traits;
mod validity;
mod value_ext;
// Re-export all core types
pub use error::{CoreError, CoreResult, ValidationError, ValidatorId};
pub use validity::{Invalid, Valid};
// Re-export new unified traits
pub use traits::{
    AndValidator, ConditionalValidator, NotValidator, OrValidator, ValidationComplexity,
    ValidationContext, Validator, ValidatorExt,
};
// Re-export builder patterns
pub use builder::{BuiltValidator, ValidationBuilder, validate};
// Re-export value extensions
pub use value_ext::ValueExt;
