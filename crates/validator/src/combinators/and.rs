//! AND combinator - logical conjunction of validators
//!
//! This module provides the [`And`] combinator which combines two validators
//! with logical AND semantics - both validators must pass for the combined
//! validator to succeed.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::And;
//! use nebula_validator::foundation::Validate;
//!
//! // Both validators must pass
//! let validator = And::new(min_length(5), max_length(20));
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
/// ```rust,ignore
/// use nebula_validator::combinators::And;
/// use nebula_validator::foundation::Validate;
///
/// let validator = And::new(min_length(5), max_length(10));
///
/// // Both conditions satisfied
/// assert!(validator.validate("hello").is_ok());
///
/// // First condition fails
/// assert!(validator.validate("hi").is_err());
///
/// // Second condition fails
/// assert!(validator.validate("verylongstring").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct And<L, R> {
    /// The left (first) validator.
    pub(crate) left: L,
    /// The right (second) validator.
    pub(crate) right: R,
}

impl<L, R> And<L, R> {
    /// Creates a new `And` combinator.
    ///
    /// # Arguments
    ///
    /// * `left` - The first validator to apply
    /// * `right` - The second validator to apply
    pub fn new(left: L, right: R) -> Self {
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

impl<L, R> Validate for And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    type Input = L::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.left.validate(input)?;
        self.right.validate(input)?;
        Ok(())
    }
}

impl<L, R> And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    /// Chains another validator with AND logic.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::foundation::ValidateExt;
    ///
    /// let validator = min_length(5).and(max_length(10)).and(alphanumeric());
    /// ```
    pub fn and<V>(self, other: V) -> And<Self, V>
    where
        V: Validate<Input = L::Input>,
    {
        And::new(self, other)
    }
}

/// Creates an `And` combinator from two validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::and;
/// use nebula_validator::foundation::Validate;
///
/// let validator = and(min_length(5), max_length(10));
/// assert!(validator.validate("hello").is_ok());
/// ```
pub fn and<L, R>(left: L, right: R) -> And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    And::new(left, right)
}

/// Creates an `AndAll` combinator from a vector of validators.
///
/// This is useful when you have a dynamic number of validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::and_all;
/// use nebula_validator::foundation::Validate;
///
/// let validators = vec![min_length(3), min_length(5), min_length(7)];
/// let validator = and_all(validators);
/// assert!(validator.validate("helloworld").is_ok());
/// assert!(validator.validate("hello").is_err());
/// ```
#[must_use]
pub fn and_all<V>(validators: Vec<V>) -> AndAll<V>
where
    V: Validate,
{
    AndAll { validators }
}

/// Combines multiple validators with logical AND.
///
/// All validators in the collection must pass for this validator to succeed.
/// Validation stops at the first failure (short-circuits).
///
/// # Type Parameters
///
/// * `V` - The validator type
#[derive(Debug, Clone)]
pub struct AndAll<V> {
    validators: Vec<V>,
}

impl<V> Validate for AndAll<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(input)?;
        }
        Ok(())
    }
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

    struct MaxLength {
        max: usize,
    }

    impl Validate for MaxLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() <= self.max {
                Ok(())
            } else {
                Err(ValidationError::max_length("", self.max, input.len()))
            }
        }
    }

    #[test]
    fn test_and_both_pass() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_and_left_fails() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_chain() {
        let validator = MinLength { min: 3 }
            .and(MaxLength { max: 10 })
            .and(MinLength { min: 5 });
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_all() {
        let validators = vec![
            MinLength { min: 3 },
            MinLength { min: 5 },
            MinLength { min: 7 },
        ];
        let combined = and_all(validators);
        assert!(combined.validate("helloworld").is_ok());
        assert!(combined.validate("hello").is_err());
    }
}
