//! AND combinator - logical conjunction of validators
//!
//! This module provides the [`And`] combinator which combines two validators
//! with logical AND semantics - both validators must pass for the combined
//! validator to succeed.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // Both validators must pass
//! let validator = min_length(5).and(max_length(20));
//! assert!(validator.validate("hello").is_ok());
//! assert!(validator.validate("hi").is_err()); // fails min_length
//! ```

use crate::foundation::{Validate, ValidationError};

/// Combines two validators with logical AND.
///
/// Both validators must pass for the combined validator to succeed.
/// Errors are returned from the first failing validator.
///
/// # Type Parameters
///
/// * `L` - The left (first) validator type
/// * `R` - The right (second) validator type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = min_length(5).and(max_length(10));
///
/// // Both conditions satisfied
/// assert!(validator.validate("hello").is_ok());
///
/// // First condition fails (too short)
/// assert!(validator.validate("hi").is_err());
///
/// // Second condition fails (too long)
/// assert!(validator.validate("verylongstring").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct And<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> And<L, R> {
    /// Creates a new `And` combinator.
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

impl<T: ?Sized, L, R> Validate<T> for And<L, R>
where
    L: Validate<T>,
    R: Validate<T>,
{
    #[inline]
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        // Contract: left side evaluates first and short-circuits on failure.
        self.left.validate(input)?;
        self.right.validate(input)
    }
}

/// Creates an `And` combinator from two validators.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = and(min_length(5), max_length(10));
/// assert!(validator.validate("hello").is_ok());
/// ```
pub fn and<L, R>(left: L, right: R) -> And<L, R> {
    And::new(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validatable;

    struct MinLength(usize);

    impl Validate<str> for MinLength {
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.0 {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.0, input.len()))
            }
        }
    }

    struct MaxLength(usize);

    impl Validate<str> for MaxLength {
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() <= self.0 {
                Ok(())
            } else {
                Err(ValidationError::max_length("", self.0, input.len()))
            }
        }
    }

    #[test]
    fn test_and_both_pass() {
        let validator = And::new(MinLength(5), MaxLength(10));
        assert!("hello".validate_with(&validator).is_ok());
    }

    #[test]
    fn test_and_left_fails() {
        let validator = And::new(MinLength(5), MaxLength(10));
        assert!("hi".validate_with(&validator).is_err());
    }

    #[test]
    fn test_and_chain() {
        use crate::foundation::ValidateExt;
        let validator = MinLength(3).and(MaxLength(10)).and(MinLength(5));
        assert!("hello".validate_with(&validator).is_ok());
        assert!("hi".validate_with(&validator).is_err());
    }
}
