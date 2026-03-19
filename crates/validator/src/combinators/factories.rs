//! Collection factory functions for combining validators.
//!
//! This module provides `all_of` and `any_of` functions for combining
//! multiple validators of the same type into a single validator.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::{all_of, any_of};
//! use nebula_validator::validators::{min_length, max_length, alphanumeric};
//!
//! // All validators must pass
//! let username_validator = all_of([
//!     min_length(3),
//!     max_length(20),
//!     alphanumeric(),
//! ]);
//!
//! // At least one validator must pass
//! let flexible_validator = any_of([
//!     min_length(5),
//!     max_length(3),  // Allow either short or long
//! ]);
//! ```

use crate::foundation::{Validate, ValidationError, ValidationErrors, ValidationMode};

// ============================================================================
// ALL OF (AND semantics)
// ============================================================================

/// Combines multiple validators with AND semantics.
///
/// All validators must pass for the combined validator to pass.
/// Errors are collected from all failing validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::all_of;
/// use nebula_validator::validators::{min_length, max_length};
/// use nebula_validator::foundation::Validate;
///
/// let validator = all_of([min_length(3), max_length(10)]);
///
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());  // too short
/// assert!(validator.validate("hello world!!").is_err());  // too long
/// ```
#[inline]
pub fn all_of<V, I>(validators: I) -> AllOf<V>
where
    I: IntoIterator<Item = V>,
{
    AllOf {
        validators: validators.into_iter().collect(),
        mode: ValidationMode::default(),
    }
}

/// A validator that requires all inner validators to pass.
///
/// Created by [`all_of()`]. Supports both fail-fast and collect-all modes
/// via [`with_mode()`](AllOf::with_mode).
#[derive(Debug, Clone)]
pub struct AllOf<V> {
    validators: Vec<V>,
    mode: ValidationMode,
}

impl<V> AllOf<V> {
    /// Returns the inner validators.
    #[must_use]
    pub fn validators(&self) -> &[V] {
        &self.validators
    }

    /// Returns the number of validators.
    #[must_use]
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Returns true if there are no validators.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// Sets the validation mode (fail-fast or collect-all).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_mode(mut self, mode: ValidationMode) -> Self {
        self.mode = mode;
        self
    }

    /// Returns the current validation mode.
    #[must_use]
    pub fn mode(&self) -> ValidationMode {
        self.mode
    }
}

impl<T: ?Sized, V> Validate<T> for AllOf<V>
where
    V: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        let mut errors = ValidationErrors::new();

        for validator in &self.validators {
            if let Err(e) = validator.validate(input) {
                if self.mode.is_fail_fast() {
                    return Err(e);
                }
                errors.add(e);
            }
        }

        if errors.has_errors() {
            Err(errors.into_single_error("all_of validation failed"))
        } else {
            Ok(())
        }
    }
}

// ============================================================================
// ANY OF (OR semantics)
// ============================================================================

/// Combines multiple validators with OR semantics.
///
/// At least one validator must pass for the combined validator to pass.
/// If all fail, errors from all validators are included.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::any_of;
/// use nebula_validator::validators::exact_length;
/// use nebula_validator::foundation::Validate;
///
/// let validator = any_of([exact_length(3), exact_length(5), exact_length(7)]);
///
/// assert!(validator.validate("abc").is_ok());    // length 3
/// assert!(validator.validate("hello").is_ok());  // length 5
/// assert!(validator.validate("abcdefg").is_ok()); // length 7
/// assert!(validator.validate("hi").is_err());    // length 2 - none match
/// ```
#[inline]
pub fn any_of<V, I>(validators: I) -> AnyOf<V>
where
    I: IntoIterator<Item = V>,
{
    AnyOf {
        validators: validators.into_iter().collect(),
    }
}

/// A validator that requires at least one inner validator to pass.
///
/// Created by [`any_of()`].
#[derive(Debug, Clone)]
pub struct AnyOf<V> {
    validators: Vec<V>,
}

impl<V> AnyOf<V> {
    /// Returns the inner validators.
    #[must_use]
    pub fn validators(&self) -> &[V] {
        &self.validators
    }

    /// Returns the number of validators.
    #[must_use]
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Returns true if there are no validators.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }
}

impl<T: ?Sized, V> Validate<T> for AnyOf<V>
where
    V: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        if self.validators.is_empty() {
            return Ok(());
        }

        let mut errors = ValidationErrors::new();

        for validator in &self.validators {
            match validator.validate(input) {
                Ok(()) => return Ok(()),
                Err(e) => errors.add(e),
            }
        }

        let count = errors.len();
        Err(ValidationError::new(
            "any_of_failed",
            format!("All {count} validators in any_of failed"),
        )
        .with_nested(errors.into_iter().collect()))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::AnyValidator;
    use crate::validators::{exact_length, max_length, min_length};

    #[test]
    fn test_all_of_all_pass() {
        // Use AnyValidator for heterogeneous types
        let validator = all_of([
            AnyValidator::new(min_length(3)),
            AnyValidator::new(max_length(10)),
        ]);
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_all_of_one_fails() {
        let validator = all_of([
            AnyValidator::new(min_length(3)),
            AnyValidator::new(max_length(10)),
        ]);
        assert!(validator.validate("hi").is_err()); // too short
    }

    #[test]
    fn test_all_of_collects_errors() {
        let validator = all_of([
            AnyValidator::new(min_length(10)),
            AnyValidator::new(max_length(3)),
        ]);
        let err = validator.validate("hello").unwrap_err();
        // Should have 2 nested errors
        assert_eq!(err.nested().len(), 2);
    }

    #[test]
    fn test_all_of_empty() {
        let validator: AllOf<crate::validators::MinLength> = all_of([]);
        assert!(validator.validate("anything").is_ok());
    }

    #[test]
    fn test_all_of_same_type() {
        // With same type, no AnyValidator needed
        let validator = all_of([min_length(3), min_length(5)]);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_any_of_first_passes() {
        let validator = any_of([exact_length(5), exact_length(10)]);
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_any_of_second_passes() {
        let validator = any_of([exact_length(5), exact_length(10)]);
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_any_of_none_pass() {
        let validator = any_of([exact_length(5), exact_length(10)]);
        let err = validator.validate("hi").unwrap_err();
        assert_eq!(err.nested().len(), 2);
    }

    #[test]
    fn test_any_of_empty() {
        let validator: AnyOf<crate::validators::MinLength> = any_of([]);
        assert!(validator.validate("anything").is_ok());
    }

    #[test]
    fn test_all_of_len() {
        let validator = all_of([min_length(1), min_length(2), min_length(3)]);
        assert_eq!(validator.len(), 3);
        assert!(!validator.is_empty());
    }

    #[test]
    fn test_any_of_len() {
        let validator = any_of([min_length(1), min_length(2)]);
        assert_eq!(validator.len(), 2);
        assert!(!validator.is_empty());
    }
}
