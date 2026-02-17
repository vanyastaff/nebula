//! OR combinator - logical disjunction of validators
//!
//! This module provides the [`Or`] combinator which combines two validators
//! with logical OR semantics - at least one validator must pass for the combined
//! validator to succeed.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::Or;
//! use nebula_validator::foundation::Validate;
//!
//! // At least one validator must pass
//! let validator = Or::new(exact_length(5), exact_length(10));
//! assert!(validator.validate("hello").is_ok()); // 5 chars
//! assert!(validator.validate("helloworld").is_ok()); // 10 chars
//! assert!(validator.validate("hi").is_err()); // neither 5 nor 10
//! ```

use crate::foundation::{Validate, ValidationError};

/// Combines two validators with logical OR.
///
/// At least one validator must pass for the combined validator to succeed.
/// If the first validator passes, the second is not evaluated (short-circuits).
/// If both fail, the combined error contains both error messages.
///
/// # Type Parameters
///
/// * `L` - The left (first) validator type
/// * `R` - The right (second) validator type
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Or;
/// use nebula_validator::foundation::Validate;
///
/// let validator = Or::new(exact_length(5), exact_length(10));
///
/// // Left validator passes
/// assert!(validator.validate("hello").is_ok());
///
/// // Right validator passes
/// assert!(validator.validate("helloworld").is_ok());
///
/// // Both fail
/// assert!(validator.validate("hi").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Or<L, R> {
    /// The left (first) validator.
    pub(crate) left: L,
    /// The right (second) validator.
    pub(crate) right: R,
}

impl<L, R> Or<L, R> {
    /// Creates a new `Or` combinator.
    ///
    /// # Arguments
    ///
    /// * `left` - The first validator to try
    /// * `right` - The second validator to try if the first fails
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

impl<L, R> Validate for Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    type Input = L::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match self.left.validate(input) {
            Ok(()) => Ok(()),
            Err(left_error) => match self.right.validate(input) {
                Ok(()) => Ok(()),
                Err(right_error) => {
                    Err(ValidationError::new("or_failed", "All alternatives failed")
                        .with_nested(vec![left_error, right_error]))
                }
            },
        }
    }
}

impl<L, R> Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    /// Chains another validator with OR logic.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::foundation::ValidateExt;
    ///
    /// let validator = exact_length(3).or(exact_length(5)).or(exact_length(7));
    /// ```
    pub fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: Validate<Input = L::Input>,
    {
        Or::new(self, other)
    }
}

/// Creates an `Or` combinator from two validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::or;
/// use nebula_validator::foundation::Validate;
///
/// let validator = or(exact_length(5), exact_length(10));
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("helloworld").is_ok());
/// ```
pub fn or<L, R>(left: L, right: R) -> Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    Or::new(left, right)
}

/// Creates an `OrAny` combinator from a vector of validators.
///
/// This is useful when you have a dynamic number of validators and
/// want at least one to pass.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::or_any;
/// use nebula_validator::foundation::Validate;
///
/// let validators = vec![exact_length(3), exact_length(5), exact_length(7)];
/// let validator = or_any(validators);
/// assert!(validator.validate("abc").is_ok());
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// ```
#[must_use]
pub fn or_any<V>(validators: Vec<V>) -> OrAny<V>
where
    V: Validate,
{
    OrAny { validators }
}

/// Tries multiple validators until one passes.
///
/// Iterates through all validators in order, returning success as soon as
/// one validator passes. If all validators fail, returns a combined error
/// containing all individual errors.
///
/// # Type Parameters
///
/// * `V` - The validator type
#[derive(Debug, Clone)]
pub struct OrAny<V> {
    validators: Vec<V>,
}

impl<V> Validate for OrAny<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            match validator.validate(input) {
                Ok(()) => return Ok(()),
                Err(e) => errors.push(e),
            }
        }

        let count = errors.len();
        Err(
            ValidationError::new("or_any_failed", format!("All {count} alternatives failed"))
                .with_nested(errors),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::traits::ValidateExt;

    struct ExactLength {
        length: usize,
    }

    impl Validate for ExactLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() == self.length {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "exact_length",
                    format!("Expected length {}", self.length),
                ))
            }
        }
    }

    #[test]
    fn test_or_left_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_or_right_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_or_both_fail() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        let err = validator.validate("hi").unwrap_err();
        assert_eq!(err.code.as_ref(), "or_failed");
        assert_eq!(err.nested.len(), 2);
        assert_eq!(err.nested[0].code.as_ref(), "exact_length");
        assert_eq!(err.nested[1].code.as_ref(), "exact_length");
    }

    #[test]
    fn test_or_chain() {
        let validator = ExactLength { length: 3 }
            .or(ExactLength { length: 5 })
            .or(ExactLength { length: 7 });
        assert!(validator.validate("abc").is_ok());
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_or_any() {
        let validators = vec![
            ExactLength { length: 3 },
            ExactLength { length: 5 },
            ExactLength { length: 7 },
        ];
        let combined = or_any(validators);
        assert!(combined.validate("abc").is_ok());
        assert!(combined.validate("hello").is_ok());

        let err = combined.validate("hi").unwrap_err();
        assert_eq!(err.code.as_ref(), "or_any_failed");
        assert_eq!(err.nested.len(), 3);
        for nested in &err.nested {
            assert_eq!(nested.code.as_ref(), "exact_length");
        }
    }
}
