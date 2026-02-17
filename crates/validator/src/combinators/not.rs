//! NOT combinator - logical negation of validators
//!
//! This module provides the [`Not`] combinator which inverts the result
//! of a validator - it succeeds when the inner validator fails and vice versa.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::Not;
//! use nebula_validator::foundation::Validate;
//!
//! // Validator that forbids a pattern
//! let validator = Not::new(contains("forbidden"));
//! assert!(validator.validate("this is allowed").is_ok());
//! assert!(validator.validate("this is forbidden").is_err());
//! ```

use crate::foundation::{Validate, ValidationError};

/// Inverts a validator with logical NOT.
///
/// The `Not` combinator reverses the validation result:
/// - If the inner validator succeeds, `Not` fails
/// - If the inner validator fails, `Not` succeeds
///
/// # Type Parameters
///
/// * `V` - The inner validator type
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Not;
/// use nebula_validator::foundation::Validate;
///
/// // Validator that forbids specific words
/// let validator = Not::new(contains("admin"));
///
/// // Does not contain "admin" - passes
/// assert!(validator.validate("user123").is_ok());
///
/// // Contains "admin" - fails
/// assert!(validator.validate("admin123").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Not<V> {
    /// The inner validator to invert.
    pub(crate) inner: V,
}

impl<V> Not<V> {
    /// Creates a new `Not` combinator.
    ///
    /// # Arguments
    ///
    /// * `inner` - The validator to invert
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

impl<V> Validate for Not<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::not;
/// use nebula_validator::foundation::Validate;
///
/// let validator = not(contains("forbidden"));
/// assert!(validator.validate("allowed").is_ok());
/// assert!(validator.validate("forbidden").is_err());
/// ```
pub fn not<V>(validator: V) -> Not<V> {
    Not::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::traits::ValidateExt;

    struct Contains {
        substring: &'static str,
    }

    impl Validate for Contains {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.contains(self.substring) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "contains",
                    format!("Must contain '{}'", self.substring),
                ))
            }
        }
    }

    #[test]
    fn test_not_inverts_success() {
        let validator = Not::new(Contains {
            substring: "forbidden",
        });
        assert!(validator.validate("this is forbidden").is_err());
    }

    #[test]
    fn test_not_inverts_failure() {
        let validator = Not::new(Contains {
            substring: "forbidden",
        });
        assert!(validator.validate("this is allowed").is_ok());
    }

    #[test]
    fn test_not_via_ext() {
        let validator = Contains { substring: "test" }.not();
        assert!(validator.validate("hello world").is_ok());
        assert!(validator.validate("test string").is_err());
    }

    #[test]
    fn test_double_negation() {
        let validator = Contains { substring: "test" }.not().not();
        assert!(validator.validate("test").is_ok());
        assert!(validator.validate("hello").is_err());
    }
}
