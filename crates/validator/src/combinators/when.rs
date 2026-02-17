//! WHEN combinator - conditional validation
//!
//! This module provides the [`When`] combinator which conditionally applies
//! a validator based on a predicate function. The validator only runs if the
//! condition returns `true`.
//!
//! # Use Cases
//!
//! - Skip validation for empty strings (validate only if non-empty)
//! - Apply different validation rules based on context
//! - Conditional validation based on field values
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::When;
//! use nebula_validator::foundation::Validate;
//!
//! // Only validate long strings
//! let validator = When::new(
//!     min_length(10),
//!     |s: &str| s.len() > 5
//! );
//! assert!(validator.validate("hi").is_ok()); // skipped
//! assert!(validator.validate("short").is_err()); // validated, too short
//! assert!(validator.validate("long enough").is_ok()); // validated, passes
//! ```

use crate::foundation::{Validate, ValidationError};

/// Conditionally applies a validator based on a predicate.
///
/// The `When` combinator only runs the inner validator if the condition
/// function returns `true`. If the condition returns `false`, validation
/// succeeds immediately without running the inner validator.
///
/// # Type Parameters
///
/// * `V` - The inner validator type
/// * `C` - The condition function type (must implement `Fn(&Input) -> bool`)
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::When;
/// use nebula_validator::foundation::Validate;
///
/// // Only validate non-empty strings
/// let validator = When::new(
///     min_length(5),
///     |s: &str| !s.is_empty()
/// );
///
/// // Empty string - skipped, passes
/// assert!(validator.validate("").is_ok());
///
/// // Short non-empty string - validated, fails
/// assert!(validator.validate("hi").is_err());
///
/// // Long string - validated, passes
/// assert!(validator.validate("hello world").is_ok());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct When<V, C> {
    /// The inner validator to apply conditionally.
    pub(crate) validator: V,
    /// The condition function that determines whether to validate.
    pub(crate) condition: C,
}

impl<V, C> When<V, C> {
    /// Creates a new `When` combinator.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to apply conditionally
    /// * `condition` - A function that returns `true` if validation should run
    pub fn new(validator: V, condition: C) -> Self {
        Self {
            validator,
            condition,
        }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns a reference to the condition function.
    pub fn condition(&self) -> &C {
        &self.condition
    }

    /// Extracts the validator and condition function.
    pub fn into_parts(self) -> (V, C) {
        (self.validator, self.condition)
    }
}

impl<V, C> Validate for When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if (self.condition)(input) {
            self.validator.validate(input)
        } else {
            Ok(())
        }
    }
}

/// Creates a `When` combinator from a validator and condition.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::when;
/// use nebula_validator::foundation::Validate;
///
/// let validator = when(min_length(10), |s: &str| s.starts_with("prefix:"));
/// assert!(validator.validate("other").is_ok()); // skipped
/// assert!(validator.validate("prefix:short").is_err()); // validated, fails
/// assert!(validator.validate("prefix:long enough").is_ok()); // validated, passes
/// ```
pub fn when<V, C>(validator: V, condition: C) -> When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    When::new(validator, condition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::traits::ValidateExt;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_when_condition_true() {
        let validator = When::new(MinLength { min: 10 }, |s: &str| s.starts_with("check_"));
        assert!(validator.validate("check_hello").is_ok()); // 11 chars >= 10
        assert!(validator.validate("check_").is_err()); // 6 chars < 10
    }

    #[test]
    fn test_when_condition_false() {
        let validator = When::new(MinLength { min: 5 }, |s: &str| s.starts_with("check_"));
        assert!(validator.validate("hi").is_ok());
        assert!(validator.validate("").is_ok());
    }

    #[test]
    fn test_when_via_ext() {
        let validator = MinLength { min: 10 }.when(|s: &str| !s.is_empty());
        assert!(validator.validate("").is_ok());
        assert!(validator.validate("short").is_err());
        assert!(validator.validate("long_enough!").is_ok());
    }
}
