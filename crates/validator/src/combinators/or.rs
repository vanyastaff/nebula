//! OR combinator - logical disjunction of validators
//!
//! This module provides the [`Or`] combinator which combines two validators
//! with logical OR semantics - at least one validator must pass for the combined
//! validator to succeed.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // At least one validator must pass
//! let validator = exact_length(5).or(exact_length(10));
//! assert!("hello".validate_with(&validator).is_ok()); // 5 chars
//! assert!("helloworld".validate_with(&validator).is_ok()); // 10 chars
//! assert!("hi".validate_with(&validator).is_err()); // neither 5 nor 10
//! ```

use crate::foundation::{Validate, ValidationError};

/// Combines two validators with logical OR.
///
/// At least one validator must pass for the combined validator to succeed.
/// If the first validator passes, the second is not evaluated (short-circuits).
/// If both fail, the combined error contains both error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Or<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> Or<L, R> {
    /// Creates a new `Or` combinator.
    pub const fn new(left: L, right: R) -> Self {
        Self { left, right }
    }

    /// Returns a reference to the left validator.
    pub fn left(&self) -> &L {
        &self.left
    }

    /// Returns a reference to the right validator.
    pub fn right(&self) -> &R {
        &self.right
    }

    /// Extracts the left and right validators.
    pub fn into_parts(self) -> (L, R) {
        (self.left, self.right)
    }
}

impl<T: ?Sized, L, R> Validate<T> for Or<L, R>
where
    L: Validate<T>,
    R: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        // Contract: right side is evaluated only if the left side fails.
        match self.left.validate(input) {
            Ok(()) => Ok(()),
            Err(error) if !error.is_violation() => Err(error),
            Err(left_error) => match self.right.validate(input) {
                Ok(()) => Ok(()),
                Err(right_error) => {
                    Err(ValidationError::new("or_failed", "All alternatives failed")
                        .with_nested_error(left_error)
                        .with_nested_error(right_error))
                },
            },
        }
    }
}

/// Creates an `Or` combinator from two validators.
pub fn or<L, R>(left: L, right: R) -> Or<L, R> {
    Or::new(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{Validatable, ValidateExt};

    struct ExactLength(usize);

    impl Validate<str> for ExactLength {
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() == self.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "exact_length",
                    format!("Expected length {}", self.0),
                ))
            }
        }
    }

    #[test]
    fn test_or_left_passes() {
        let validator = Or::new(ExactLength(5), ExactLength(10));
        assert!("hello".validate_with(&validator).is_ok());
    }

    #[test]
    fn test_or_right_passes() {
        let validator = Or::new(ExactLength(5), ExactLength(10));
        assert!("helloworld".validate_with(&validator).is_ok());
    }

    #[test]
    fn test_or_both_fail() {
        let validator = Or::new(ExactLength(5), ExactLength(10));
        let err = "hi".validate_with(&validator).unwrap_err();
        assert_eq!(err.code.as_ref(), "or_failed");
        assert_eq!(err.nested().len(), 2);
    }

    #[test]
    fn test_or_chain() {
        let validator = ExactLength(3).or(ExactLength(5)).or(ExactLength(7));
        assert!("abc".validate_with(&validator).is_ok());
        assert!("hello".validate_with(&validator).is_ok());
        assert!("hi".validate_with(&validator).is_err());
    }
}
