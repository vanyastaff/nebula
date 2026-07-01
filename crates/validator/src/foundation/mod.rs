//! Core validation types and traits
//!
//! This module provides the fundamental building blocks for type-safe validation:
//!
//! - [`Validate`](crate::foundation::traits::Validate) - Core trait for validators, generic over
//!   input type
//! - [`Validatable`](crate::foundation::traits::Validatable) - Extension trait enabling
//!   `value.validate(&validator)` syntax
//! - [`ValidateExt`](crate::foundation::traits::ValidateExt) - Combinator methods (`.and()`,
//!   `.or()`, `.not()`)
//! - [`ValidationError`](crate::foundation::error::ValidationError) - Structured validation errors
//!
//! # Two Ways to Validate
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // Extension method: read left-to-right
//! "hello".validate_with(&min_length(3))?;
//!
//! // Direct method: traditional style
//! min_length(3).validate("hello")?;
//! # Ok::<(), nebula_validator::foundation::ValidationError>(())
//! ```
//!
//! # Type Safety Through Trait Bounds
//!
//! Validators use trait bounds to ensure compile-time type safety:
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // String validators work with any `AsRef<str>` input.
//! assert!(min_length(3).validate("hello").is_ok());
//!
//! // Numeric validators work with any matching `Ord` type.
//! assert!(min(10).validate(&42).is_ok());
//!
//! // Invalid combinations are rejected at compile time:
//! // `42.validate_with(&min_length(3))` does not compile, because i32 is not `AsRef<str>`.
//! ```
//!
//! # Composition
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = min_length(5)
//!     .and(max_length(20))
//!     .and(alphanumeric());
//!
//! "hello123".validate_with(&validator)?;
//! # Ok::<(), nebula_validator::foundation::ValidationError>(())
//! ```

// Module declarations
pub mod any;
pub mod error;
pub mod field_path;
pub mod traits;
pub mod validatable;

// Re-export core types
pub use any::AnyValidator;
pub use error::{ErrorSeverity, ValidationError, ValidationErrors, ValidationMode};
pub use field_path::FieldPath;
pub use traits::{Validatable, Validate, ValidateExt};
pub use validatable::AsValidatable;

// ============================================================================
// PRELUDE
// ============================================================================

/// Common imports for working with the validator core.
///
/// Combinator types (`And`, `Or`, `Not`, `When`) live in [`crate::combinators`] —
/// import them from there or use the top-level [`crate::prelude`] which re-exports both.
///
/// ```rust
/// use nebula_validator::foundation::prelude::*;
/// // The foundation prelude brings in the traits; import validator constructors
/// // from the `validators` module (combinators live in `combinators`).
/// use nebula_validator::validators::{min, min_length};
///
/// // Extension method style
/// "hello".validate_with(&min_length(3))?;
/// 42.validate_with(&min(10))?;
///
/// // Direct method style
/// min_length(3).validate("hello")?;
/// # Ok::<(), nebula_validator::foundation::ValidationError>(())
/// ```
pub mod prelude {
    pub use super::{
        AnyValidator, ErrorSeverity, Validatable, Validate, ValidateExt, ValidationError,
        ValidationErrors,
    };
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

    // Test validator using the new Validate<T> pattern
    struct AlwaysValid;

    impl Validate<str> for AlwaysValid {
        fn validate(&self, _input: &str) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;

    impl Validate<str> for AlwaysFails {
        fn validate(&self, _input: &str) -> Result<(), ValidationError> {
            Err(ValidationError::new("always_fails", "Always fails"))
        }
    }

    #[test]
    fn test_extension_method() {
        // New: extension method style
        assert!("test".validate_with(&AlwaysValid).is_ok());
        assert!("test".validate_with(&AlwaysFails).is_err());
    }

    #[test]
    fn test_direct_method() {
        // Traditional: direct call style
        assert!(AlwaysValid.validate("test").is_ok());
        assert!(AlwaysFails.validate("test").is_err());
    }

    #[test]
    fn test_and_combinator() {
        let both = AlwaysValid.and(AlwaysValid);
        assert!("test".validate_with(&both).is_ok());

        let one_fails = AlwaysValid.and(AlwaysFails);
        assert!("test".validate_with(&one_fails).is_err());
    }

    #[test]
    fn test_or_combinator() {
        let one_passes = AlwaysFails.or(AlwaysValid);
        assert!("test".validate_with(&one_passes).is_ok());

        let both_fail = AlwaysFails.or(AlwaysFails);
        assert!("test".validate_with(&both_fail).is_err());
    }

    #[test]
    fn test_not_combinator() {
        let not_fails = AlwaysFails.not();
        assert!("test".validate_with(&not_fails).is_ok());

        let not_valid = AlwaysValid.not();
        assert!("test".validate_with(&not_valid).is_err());
    }

    #[test]
    fn test_method_chaining() {
        // Chain multiple validations via extension method
        let result = "hello"
            .validate_with(&AlwaysValid)
            .and_then(|s| s.validate_with(&AlwaysValid));
        assert!(result.is_ok());
    }
}
