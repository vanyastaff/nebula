//! Core validation types and traits
//!
//! This module contains the fundamental building blocks of the validation system:
//!
//! - **Traits**: `Validate`, `AsyncValidate`, `ValidateExt`
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
//! ```rust,ignore
//! use nebula_validator::core::Validate;
//!
//! struct MinLength { min: usize }
//!
//! impl Validate for MinLength {
//!     type Input = str;  // Only validates strings
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
//! ```rust,ignore
//! let validator = min_length(5)
//!     .and(max_length(20))
//!     .and(alphanumeric());
//! ```
//!
//! ## 3. Zero-Cost Abstractions
//!
//! Validators use generics and inline code, resulting in zero runtime overhead:
//!
//! ```rust,ignore
//! // This compiles to the same code as manually writing the checks!
//! let validator = min_length(5).and(max_length(20));
//! ```
//!
//! ## 4. Rich Error Information
//!
//! Errors are structured and contain detailed information:
//!
//! ```rust,ignore
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
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! let validator = MinLength { min: 5 };
//! assert!(validator.validate("hello").is_ok());
//! assert!(validator.validate("hi").is_err());
//! ```
//!
//! ## Refined types
//!
//! ```rust,ignore
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
//! ```rust,ignore
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
pub mod validatable;

// Re-export everything at the core level for convenience
pub use context::{ContextualValidator, ValidationContext, ValidationContextBuilder};
pub use error::{ErrorSeverity, ValidationError, ValidationErrors};
pub use metadata::{
    RegisteredValidatorMetadata, ValidationComplexity, ValidatorMetadata, ValidatorMetadataBuilder,
    ValidatorStatistics,
};
pub use refined::Refined;
pub use state::{Parameter, ParameterBuilder, Unvalidated, Validated};
pub use traits::{AsyncValidate, Validate, ValidateExt};
pub use validatable::AsValidatable;

// ============================================================================
// PRELUDE
// ============================================================================

/// Common imports for working with the validator core.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::core::prelude::*;
///
/// // Now you have access to all common types and traits
/// let validator = MinLength { min: 5 }.and(MaxLength { max: 20 });
/// ```
pub mod prelude {
    pub use super::{
        AsValidatable, AsyncValidate, ContextualValidator, ErrorSeverity, Parameter,
        ParameterBuilder, Refined, Unvalidated, Validate, ValidateExt, Validated,
        ValidationComplexity, ValidationContext, ValidationContextBuilder, ValidationError,
        ValidationErrors, ValidatorMetadata,
    };
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
/// ```rust,ignore
/// use nebula_validator::core::validate_value;
///
/// let result = validate_value("hello", &min_length(5))?;
/// ```
#[must_use = "validation result must be checked"]
pub fn validate_value<V>(value: &V::Input, validator: &V) -> Result<(), ValidationError>
where
    V: Validate,
{
    validator.validate(value)
}

/// Validates a value with multiple validators.
///
/// All validators must pass for this to succeed.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::core::validate_with_all;
///
/// let result = validate_with_all("hello", &[
///     &min_length(3),
///     &max_length(10),
/// ])?;
/// ```
pub fn validate_with_all<V>(value: &V::Input, validators: &[&V]) -> Result<(), ValidationErrors>
where
    V: Validate + ?Sized,
{
    let mut errors = ValidationErrors::new();

    for validator in validators {
        if let Err(e) = validator.validate(value) {
            errors.add(e);
        }
    }

    if errors.has_errors() {
        Err(errors)
    } else {
        Ok(())
    }
}

/// Validates a value with multiple validators (at least one must pass).
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::core::validate_with_any;
///
/// let result = validate_with_any("hello", &[
///     &exact_length(5),
///     &exact_length(10),
/// ])?;
/// ```
pub fn validate_with_any<V>(value: &V::Input, validators: &[&V]) -> Result<(), ValidationErrors>
where
    V: Validate + ?Sized,
{
    let mut errors = ValidationErrors::new();

    for validator in validators {
        match validator.validate(value) {
            Ok(()) => return Ok(()),
            Err(e) => {
                errors.add(e);
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

    // Simple test validator for testing utilities
    struct AlwaysValid;

    impl Validate for AlwaysValid {
        type Input = str;

        fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;

    impl Validate for AlwaysFails {
        type Input = str;

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
        let result = validate_with_all("test", &[&AlwaysValid, &AlwaysValid]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_all_failure() {
        let valid = AlwaysValid;
        let fails = AlwaysFails;
        let validators: &[&dyn Validate<Input = str>] = &[&valid, &fails];
        let result = validate_with_all("test", validators);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_with_any_success() {
        let valid = AlwaysValid;
        let fails = AlwaysFails;
        let validators: &[&dyn Validate<Input = str>] = &[&fails, &valid];
        let result = validate_with_any("test", validators);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_any_all_fail() {
        let result = validate_with_any("test", &[&AlwaysFails, &AlwaysFails]);
        assert!(result.is_err());
    }
}
