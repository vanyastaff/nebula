//! Bridge module for legacy support
//!
//! This module provides compatibility layers between v2 validators
//! and the legacy nebula-value::Value type from v1.
//!
//! # New Trait-Based Approach (Recommended)
//!
//! Use the `Extract` trait for automatic type extraction:
//!
//! ```rust
//! use nebula_validator::bridge::extract::ValueValidatorExt;
//! use nebula_validator::validators::string::min_length;
//!
//! // Automatically adapts string validator to work with Value
//! let validator = min_length(5).for_value();
//! ```
//!
//! # Legacy Manual Bridge (Old Approach)
//!
//! For backward compatibility, manual bridge functions are still available:
//!
//! ```rust
//! use nebula_validator::bridge::value::for_string;
//! use nebula_validator::validators::string::min_length;
//!
//! let validator = for_string(min_length(5));
//! ```

pub mod extract;
pub mod value;

// Re-export main types from new trait-based approach
pub use extract::{Extract, ExtractOwned, ValueAdapter, ValueValidatorExt};

// Re-export legacy bridge types
pub use value::{
    for_array, for_bool, for_f64, for_i64, for_string, Invalid, LegacyValidator, Valid,
    ValidationContext, ValueArrayValidator, ValueBoolValidator, ValueF64Validator,
    ValueI64Validator, ValueValidator, V1Adapter,
};

/// Prelude for bridge module - imports the recommended trait-based API.
pub mod prelude {
    pub use super::extract::{Extract, ValueValidatorExt};
    pub use super::value::{for_array, for_bool, for_f64, for_i64, for_string};
}
