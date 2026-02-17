//! Sealed category traits for type-safe validator categorization
//!
//! This module provides sealed traits that categorize validators at the type level.
//! Using sealed traits prevents external crates from implementing these categories,
//! ensuring type-level guarantees about validator behavior.
//!
//! ## Categories
//!
//! - [`StringValidator`] - Validators for string/text content
//! - [`NumericValidator`] - Validators for numeric values
//! - [`CollectionValidator`] - Validators for collections (arrays, maps)
//! - [`CompositeValidator`] - Validators that combine other validators
//!
//! ## Usage
//!
//! These traits enable generic functions that operate only on specific categories:
//!
//! ```ignore
//! use nebula_validator::foundation::category::StringValidator;
//!
//! fn validate_user_input<V: StringValidator>(validator: &V, input: &str) -> bool {
//!     // Only string validators can be passed here
//!     validator.validate(input).is_ok()
//! }
//! ```

use crate::foundation::Validate;

// ============================================================================
// Sealed module - prevents external implementations
// ============================================================================

mod sealed {
    pub trait Sealed {}
}

// ============================================================================
// Category Traits
// ============================================================================

/// Marker trait for validators that validate string content.
///
/// String validators operate on `&str` or `String` input and check properties
/// like length, format, content, or patterns.
///
/// This trait is sealed and cannot be implemented outside this crate.
///
/// # Examples
///
/// - `MinLength`, `MaxLength` - length constraints
/// - `Regex`, `Pattern` - pattern matching
/// - `Email`, `Url`, `Uuid` - format validation
/// - `Contains`, `StartsWith`, `EndsWith` - content checks
pub trait StringValidator: Validate + sealed::Sealed {
    /// Returns the category name for this validator.
    fn category() -> &'static str {
        "string"
    }

    /// Returns a description of what this validator checks.
    fn description(&self) -> &str {
        "Validates string content"
    }
}

/// Marker trait for validators that validate numeric values.
///
/// Numeric validators operate on numeric types (integers, floats) and check
/// properties like range, sign, or mathematical properties.
///
/// This trait is sealed and cannot be implemented outside this crate.
///
/// # Examples
///
/// - `InRange`, `Min`, `Max` - range constraints
/// - `Positive`, `Negative`, `NonZero` - sign constraints
/// - `Even`, `Odd`, `MultipleOf` - mathematical properties
pub trait NumericValidator: Validate + sealed::Sealed {
    /// Returns the category name for this validator.
    fn category() -> &'static str {
        "numeric"
    }

    /// Returns a description of what this validator checks.
    fn description(&self) -> &str {
        "Validates numeric values"
    }
}

/// Marker trait for validators that validate collections.
///
/// Collection validators operate on arrays, vectors, maps, and other
/// container types, checking size, elements, or structure.
///
/// This trait is sealed and cannot be implemented outside this crate.
///
/// # Examples
///
/// - `MinSize`, `MaxSize`, `ExactSize` - size constraints
/// - `NonEmpty`, `AllElements`, `AnyElement` - element checks
/// - `UniqueElements`, `SortedElements` - structural checks
pub trait CollectionValidator: Validate + sealed::Sealed {
    /// Returns the category name for this validator.
    fn category() -> &'static str {
        "collection"
    }

    /// Returns a description of what this validator checks.
    fn description(&self) -> &str {
        "Validates collections"
    }
}

/// Marker trait for composite validators that combine other validators.
///
/// Composite validators are created by combining simpler validators using
/// logical operations like AND, OR, NOT, or conditional logic.
///
/// This trait is sealed and cannot be implemented outside this crate.
///
/// # Examples
///
/// - `And<A, B>` - both validators must pass
/// - `Or<A, B>` - at least one validator must pass
/// - `Not<V>` - validator must fail
/// - `When<V, C>` - conditional validation
pub trait CompositeValidator: Validate + sealed::Sealed {
    /// Returns the category name for this validator.
    fn category() -> &'static str {
        "composite"
    }

    /// Returns the number of inner validators.
    fn validator_count(&self) -> usize {
        1
    }
}

// ============================================================================
// Sealed Implementations for Combinators
// ============================================================================

use crate::combinators::{And, Cached, Not, Optional, Or, When};

impl<A, B> sealed::Sealed for And<A, B>
where
    A: Validate,
    B: Validate<Input = A::Input>,
{
}

impl<A, B> CompositeValidator for And<A, B>
where
    A: Validate,
    B: Validate<Input = A::Input>,
{
    fn validator_count(&self) -> usize {
        2
    }
}

impl<A, B> sealed::Sealed for Or<A, B>
where
    A: Validate,
    B: Validate<Input = A::Input>,
{
}

impl<A, B> CompositeValidator for Or<A, B>
where
    A: Validate,
    B: Validate<Input = A::Input>,
{
    fn validator_count(&self) -> usize {
        2
    }
}

impl<V: Validate> sealed::Sealed for Not<V> {}

impl<V: Validate> CompositeValidator for Not<V> {
    fn validator_count(&self) -> usize {
        1
    }
}

impl<V, C> sealed::Sealed for When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
}

impl<V, C> CompositeValidator for When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    fn validator_count(&self) -> usize {
        1
    }
}

impl<V, T> sealed::Sealed for Optional<V> where V: Validate<Input = T> {}

impl<V, T> CompositeValidator for Optional<V>
where
    V: Validate<Input = T>,
{
    fn validator_count(&self) -> usize {
        1
    }
}

impl<V> sealed::Sealed for Cached<V>
where
    V: Validate,
    V::Input: std::hash::Hash + Eq,
{
}

impl<V> CompositeValidator for Cached<V>
where
    V: Validate,
    V::Input: std::hash::Hash + Eq,
{
    fn validator_count(&self) -> usize {
        1
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{ValidateExt, ValidationError};

    struct TestStringValidator;

    impl Validate for TestStringValidator {
        type Input = str;

        fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
            if input.len() >= 3 {
                Ok(())
            } else {
                Err(ValidationError::new("test", "Too short"))
            }
        }
    }

    impl sealed::Sealed for TestStringValidator {}
    impl StringValidator for TestStringValidator {}

    #[test]
    fn test_string_validator_category() {
        assert_eq!(TestStringValidator::category(), "string");
    }

    #[test]
    fn test_composite_validator_count() {
        let and_validator = TestStringValidator.and(TestStringValidator);
        assert_eq!(and_validator.validator_count(), 2);

        let not_validator = TestStringValidator.not();
        assert_eq!(not_validator.validator_count(), 1);
    }

    #[test]
    fn test_composite_validator_category() {
        let and_validator = TestStringValidator.and(TestStringValidator);
        assert_eq!(
            <And<TestStringValidator, TestStringValidator> as CompositeValidator>::category(),
            "composite"
        );

        let _ = and_validator; // Use the validator
    }
}
