//! Core validation types and traits
//!
//! This module contains the fundamental building blocks of the validation system:
//!
//! - **Traits**: `TypedValidator`, `AsyncValidator`, `ValidatorExt`
//! - **Errors**: `ValidationError`, `ValidationErrors`
//! - **Metadata**: `ValidatorMetadata`, `ValidationComplexity`, `ValidatorStatistics`
//! - **Refined Types**: `Refined<T, V>` for compile-time validation guarantees
//! - **Type-State**: `Parameter<T, S>` for state-based validation
//!
//! # Architecture
//!
//! The core is designed around several key principles:
//!
//! ## 1. Type Safety
//!
//! Validators are generic over their input type, providing compile-time guarantees:
//!
//! ```rust
//! use nebula_validator::core::TypedValidator;
//!
//! struct MinLength { min: usize }
//!
//! impl TypedValidator for MinLength {
//!     type Input = str;  // Only validates strings
//!     type Output = ();
//!     type Error = ValidationError;
//!     
//!     fn validate(&self, input: &str) -> Result<(), ValidationError> {
//!         // ...
//!     }
//! }
//! ```
//!
//! ## 2. Composition
//!
//! Validators compose using logical combinators:
//!
//! ```rust
//! let validator = min_length(5)
//!     .and(max_length(20))
//!     .and(alphanumeric());
//! ```
//!
//! ## 3. Zero-Cost Abstractions
//!
//! Validators use generics and inline code, resulting in zero runtime overhead:
//!
//! ```rust
//! // This compiles to the same code as manually writing the checks!
//! let validator = min_length(5).and(max_length(20));
//! ```
//!
//! ## 4. Rich Error Information
//!
//! Errors are structured and contain detailed information:
//!
//! ```rust
//! let error = ValidationError::new("min_length", "Too short")
//!     .with_field("username")
//!     .with_param("min", "5")
//!     .with_param("actual", "3");
//! ```
//!
//! # Examples
//!
//! ## Basic validation
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = MinLength { min: 5 };
//! assert!(validator.validate("hello").is_ok());
//! assert!(validator.validate("hi").is_err());
//! ```
//!
//! ## Refined types
//!
//! ```rust
//! let validator = MinLength { min: 5 };
//! let validated = Refined::new("hello".to_string(), &validator)?;
//!
//! // Type system now knows this string is at least 5 characters!
//! fn process(s: Refined<String, MinLength>) {
//!     // s.len() >= 5 is guaranteed
//! }
//! ```
//!
//! ## Type-state pattern
//!
//! ```rust
//! let param = Parameter::new("hello".to_string());
//! let validated = param.validate(&validator)?;
//! let value = validated.unwrap(); // Safe - type guarantees validity
//! ```

// Module declarations
pub mod context;
pub mod error;
pub mod metadata;
pub mod refined;
pub mod state;
pub mod traits;

// Re-export everything at the core level for convenience
pub use context::{ContextualValidator, ValidationContext, ValidationContextBuilder};
pub use error::{ErrorSeverity, ValidationError, ValidationErrors};
pub use metadata::{
    RegisteredValidatorMetadata, ValidationComplexity, ValidatorMetadata, ValidatorMetadataBuilder,
    ValidatorStatistics,
};
pub use refined::Refined;
pub use state::{Parameter, ParameterBuilder, Unvalidated, Validated, ValidationGroup};
pub use traits::{AsyncValidator, TypedValidator, ValidatorExt};

// ============================================================================
// PRELUDE
// ============================================================================

/// Common imports for working with the validator core.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::prelude::*;
///
/// // Now you have access to all common types and traits
/// let validator = MinLength { min: 5 }.and(MaxLength { max: 20 });
/// ```
pub mod prelude {
    pub use super::{
        AsyncValidator, ContextualValidator, ErrorSeverity, Parameter, ParameterBuilder, Refined,
        TypedValidator, Unvalidated, Validated, ValidationComplexity, ValidationContext,
        ValidationContextBuilder, ValidationError, ValidationErrors, ValidatorExt,
        ValidatorMetadata,
    };
}

// ============================================================================
// VERSION INFO
// ============================================================================

/// The version of the validator core.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the version string.
#[must_use] 
pub fn version() -> &'static str {
    VERSION
}

// ============================================================================
// FEATURE FLAGS INFO
// ============================================================================

/// Returns information about enabled features.
#[must_use] 
pub fn features() -> Features {
    Features {
        async_support: cfg!(feature = "async"),
        serde_support: cfg!(feature = "serde"),
        cache_support: cfg!(feature = "cache"),
    }
}

/// Information about enabled features.
#[derive(Debug, Clone, Copy)]
pub struct Features {
    /// Whether async validation is enabled.
    pub async_support: bool,

    /// Whether serde serialization is enabled.
    pub serde_support: bool,

    /// Whether caching is enabled.
    pub cache_support: bool,
}

// ============================================================================
// UTILITIES
// ============================================================================

/// Validates a value and returns a more detailed result.
///
/// This is a convenience function for one-off validations.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::validate_value;
///
/// let result = validate_value("hello", &min_length(5))?;
/// ```
#[must_use = "validation result must be checked"]
pub fn validate_value<V>(value: &V::Input, validator: &V) -> Result<V::Output, V::Error>
where
    V: TypedValidator,
{
    validator.validate(value)
}

/// Validates a value with multiple validators.
///
/// All validators must pass for this to succeed.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::validate_with_all;
///
/// let result = validate_with_all("hello", vec![
///     &min_length(3),
///     &max_length(10),
/// ])?;
/// ```
pub fn validate_with_all<V>(
    value: &V::Input,
    validators: Vec<&V>,
) -> Result<V::Output, ValidationErrors>
where
    V: TypedValidator + ?Sized,
{
    let mut errors = ValidationErrors::new();

    for validator in &validators {
        if let Err(e) = validator.validate(value) {
            errors.add(ValidationError::new("validation_failed", e.to_string()));
        }
    }

    if errors.has_errors() {
        Err(errors)
    } else {
        // For simplicity, assume first validator's output type
        validators[0]
            .validate(value)
            .map_err(|_| ValidationErrors::new())
    }
}

/// Validates a value with multiple validators (at least one must pass).
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::validate_with_any;
///
/// let result = validate_with_any("hello", vec![
///     &exact_length(5),
///     &exact_length(10),
/// ])?;
/// ```
pub fn validate_with_any<V>(
    value: &V::Input,
    validators: Vec<&V>,
) -> Result<V::Output, ValidationErrors>
where
    V: TypedValidator + ?Sized,
{
    let mut errors = ValidationErrors::new();

    for validator in validators {
        match validator.validate(value) {
            Ok(output) => return Ok(output),
            Err(e) => {
                errors.add(ValidationError::new("validation_failed", e.to_string()));
            }
        }
    }

    Err(errors)
}

// ============================================================================
// TYPE ALIASES
// ============================================================================

/// A validation result using the standard `ValidationError`.
pub type ValidationResult<T> = Result<T, ValidationError>;

/// A validation result that can contain multiple errors.
pub type ValidationResultMulti<T> = Result<T, ValidationErrors>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod core_tests {
    use super::*;
    use crate::core::traits::ValidatorExt;

    #[test]
    fn test_version() {
        let v = version();
        assert!(!v.is_empty());
    }

    #[test]
    fn test_features() {
        let features = features();
        // At least one feature should be enabled in tests
        assert!(features.async_support || features.serde_support || features.cache_support);
    }

    // Simple test validator for testing utilities
    struct AlwaysValid;

    impl TypedValidator for AlwaysValid {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;

    impl TypedValidator for AlwaysFails {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
            Err(ValidationError::new("always_fails", "Always fails"))
        }
    }

    #[test]
    fn test_validate_value() {
        let validator = AlwaysValid;
        assert!(validate_value("test", &validator).is_ok());
    }

    #[test]
    fn test_validate_with_all_success() {
        let result = validate_with_all("test", vec![&AlwaysValid, &AlwaysValid]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_all_failure() {
        let valid = AlwaysValid;
        let fails = AlwaysFails;
        let validators: Vec<&dyn TypedValidator<Input = str, Output = (), Error = ValidationError>> =
            vec![&valid, &fails];
        let result = validate_with_all("test", validators);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_with_any_success() {
        let valid = AlwaysValid;
        let fails = AlwaysFails;
        let validators: Vec<&dyn TypedValidator<Input = str, Output = (), Error = ValidationError>> =
            vec![&fails, &valid];
        let result = validate_with_any("test", validators);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_any_all_fail() {
        let result = validate_with_any("test", vec![&AlwaysFails, &AlwaysFails]);
        assert!(result.is_err());
    }
}
