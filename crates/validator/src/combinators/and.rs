//! AND combinator - logical conjunction of validators
//!
//! This module provides the [`And`] combinator which combines two validators
//! with logical AND semantics - both validators must pass for the combined
//! validator to succeed.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Both validators must pass
//! let validator = min_length(5).and(max_length(20));
//! assert!("hello".validate(&validator).is_ok());
//! assert!("hi".validate(&validator).is_err()); // fails min_length
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
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// let validator = min_length(5).and(max_length(10));
///
/// // Both conditions satisfied
/// assert!("hello".validate(&validator).is_ok());
///
/// // First condition fails
/// assert!("hi".validate(&validator).is_err());
///
/// // Second condition fails
/// assert!("verylongstring".validate(&validator).is_err());
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
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// let validator = and(min_length(5), max_length(10));
/// assert!("hello".validate(&validator).is_ok());
/// ```
pub fn and<L, R>(left: L, right: R) -> And<L, R> {
    And::new(left, right)
}

/// Creates an `AndAll` combinator from a vector of validators.
///
/// This is useful when you have a dynamic number of validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// let validators = vec![min_length(3), min_length(5), min_length(7)];
/// let validator = and_all(validators);
/// assert!("helloworld".validate(&validator).is_ok());
/// assert!("hello".validate(&validator).is_err());
/// ```
#[must_use]
pub fn and_all<V>(validators: Vec<V>) -> AndAll<V> {
    AndAll { validators }
}

/// Combines multiple validators with logical AND.
///
/// All validators in the collection must pass for this validator to succeed.
/// Validation stops at the first failure (short-circuits).
#[derive(Debug, Clone)]
pub struct AndAll<V> {
    validators: Vec<V>,
}

impl<T: ?Sized, V> Validate<T> for AndAll<V>
where
    V: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(input)?;
        }
        Ok(())
    }
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

    #[test]
    fn test_and_all() {
        let validators = vec![MinLength(3), MinLength(5), MinLength(7)];
        let combined = and_all(validators);
        assert!("helloworld".validate_with(&combined).is_ok());
        assert!("hello".validate_with(&combined).is_err());
    }
}
