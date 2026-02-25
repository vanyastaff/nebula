//! NOT combinator - logical negation of validators
//!
//! This module provides the [`Not`] combinator which inverts the result
//! of a validator - it succeeds when the inner validator fails and vice versa.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Validator that forbids a pattern
//! let validator = contains("forbidden").not();
//! assert!("this is allowed".validate(&validator).is_ok());
//! assert!("this is forbidden".validate(&validator).is_err());
//! ```

use crate::foundation::{Validate, ValidationError};

/// Inverts a validator with logical NOT.
///
/// The `Not` combinator reverses the validation result:
/// - If the inner validator succeeds, `Not` fails
/// - If the inner validator fails, `Not` succeeds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Not<V> {
    pub(crate) inner: V,
}

impl<V> Not<V> {
    /// Creates a new `Not` combinator.
    pub const fn new(inner: V) -> Self {
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

impl<T: ?Sized, V> Validate<T> for Not<V>
where
    V: Validate<T>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        match self.inner.validate(input) {
            Ok(()) => Err(ValidationError::new(
                "not_failed",
                "Validation should have failed but passed",
            )),
            Err(_) => Ok(()),
        }
    }
}

/// Creates a `Not` combinator from a validator.
pub fn not<V>(validator: V) -> Not<V> {
    Not::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{Validatable, ValidateExt};

    struct Contains(&'static str);

    impl Validate<str> for Contains {
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.contains(self.0) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "contains",
                    format!("Must contain '{}'", self.0),
                ))
            }
        }
    }

    #[test]
    fn test_not_inverts_success() {
        let validator = Not::new(Contains("forbidden"));
        assert!("this is forbidden".validate_with(&validator).is_err());
    }

    #[test]
    fn test_not_inverts_failure() {
        let validator = Not::new(Contains("forbidden"));
        assert!("this is allowed".validate_with(&validator).is_ok());
    }

    #[test]
    fn test_not_via_ext() {
        let validator = Contains("test").not();
        assert!("hello world".validate_with(&validator).is_ok());
        assert!("test string".validate_with(&validator).is_err());
    }

    #[test]
    fn test_double_negation() {
        let validator = Contains("test").not().not();
        assert!("test".validate_with(&validator).is_ok());
        assert!("hello".validate_with(&validator).is_err());
    }
}
