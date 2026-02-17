//! OPTIONAL combinator - validates Option types
//!
//! This module provides the [`Optional`] combinator which wraps a validator
//! to work with `Option<T>` types. `None` values pass validation automatically,
//! while `Some(value)` values are validated with the inner validator.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::Optional;
//! use nebula_validator::foundation::Validate;
//!
//! // Validator that accepts None or valid strings
//! let validator = Optional::new(min_length(5));
//! assert!(validator.validate(&None).is_ok()); // None passes
//! assert!(validator.validate(&Some("hello".to_string())).is_ok()); // Some valid passes
//! assert!(validator.validate(&Some("hi".to_string())).is_err()); // Some invalid fails
//! ```

use crate::foundation::{Validate, ValidationError};

/// Makes a validator work with `Option` types.
///
/// The `Optional` combinator passes validation for `None` values and
/// delegates to the inner validator for `Some(value)` values.
///
/// # Type Parameters
///
/// * `V` - The inner validator type
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Optional;
/// use nebula_validator::foundation::Validate;
///
/// let validator = Optional::new(min_length(5));
///
/// // None passes automatically
/// let none: Option<String> = None;
/// assert!(validator.validate(&none).is_ok());
///
/// // Some with valid value passes
/// assert!(validator.validate(&Some("hello".to_string())).is_ok());
///
/// // Some with invalid value fails
/// assert!(validator.validate(&Some("hi".to_string())).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Optional<V> {
    /// The inner validator for `Some` values.
    pub(crate) inner: V,
}

impl<V> Optional<V> {
    /// Creates a new `Optional` combinator.
    ///
    /// # Arguments
    ///
    /// * `inner` - The validator to use for `Some` values
    pub fn new(inner: V) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner validator.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Extracts the inner validator.
    pub fn into_inner(self) -> V {
        self.inner
    }
}

impl<V, T> Validate for Optional<V>
where
    V: Validate<Input = T>,
{
    type Input = Option<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match input {
            None => Ok(()),
            Some(value) => self.inner.validate(value),
        }
    }
}

/// Creates an `Optional` combinator from a validator.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::optional;
/// use nebula_validator::foundation::Validate;
///
/// let validator = optional(min_length(5));
/// assert!(validator.validate(&None::<String>).is_ok());
/// assert!(validator.validate(&Some("hello".to_string())).is_ok());
/// assert!(validator.validate(&Some("hi".to_string())).is_err());
/// ```
pub fn optional<V>(validator: V) -> Optional<V> {
    Optional::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = String;
        fn validate(&self, input: &String) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_optional_none() {
        let validator = Optional::new(MinLength { min: 5 });
        let input: Option<String> = None;
        assert!(validator.validate(&input).is_ok());
    }

    #[test]
    fn test_optional_some_valid() {
        let validator = Optional::new(MinLength { min: 5 });
        let input = Some("hello".to_string());
        assert!(validator.validate(&input).is_ok());
    }

    #[test]
    fn test_optional_some_invalid() {
        let validator = Optional::new(MinLength { min: 5 });
        let input = Some("hi".to_string());
        assert!(validator.validate(&input).is_err());
    }

    #[test]
    fn test_optional_helper() {
        let validator = optional(MinLength { min: 5 });
        let none: Option<String> = None;
        let some_valid = Some("hello".to_string());
        let some_invalid = Some("hi".to_string());

        assert!(validator.validate(&none).is_ok());
        assert!(validator.validate(&some_valid).is_ok());
        assert!(validator.validate(&some_invalid).is_err());
    }
}
