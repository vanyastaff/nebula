//! Nullable validators for Option types
//!
//! This module provides validators for working with `Option<T>` types.
//!
//! # Validators
//!
//! - [`Required`] / [`NotNull`] - Validates that an `Option` is `Some`
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Ensure a value is present
//! let validator = required::<String>();
//! assert!(validator.validate(&Some("hello".to_string())).is_ok());
//! assert!(validator.validate(&None::<String>).is_err());
//! ```

use crate::foundation::{Validate, ValidationError};
use std::marker::PhantomData;

/// Validates that an `Option` is `Some`.
///
/// This validator passes if the input is `Some(value)` and fails if it is `None`.
///
/// # Type Parameters
///
/// * `T` - The inner type of the `Option`
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::Required;
/// use nebula_validator::foundation::Validate;
///
/// let validator = Required::<i32>;
/// assert!(validator.validate(&Some(42)).is_ok());
/// assert!(validator.validate(&None::<i32>).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Required<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validate for Required<T> {
    type Input = Option<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_some() {
            Ok(())
        } else {
            Err(ValidationError::new("required", "Value is required"))
        }
    }
}

/// Creates a `Required` validator.
///
/// # Type Parameters
///
/// * `T` - The inner type of the `Option` being validated
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::required;
/// use nebula_validator::foundation::Validate;
///
/// let validator = required::<String>();
/// assert!(validator.validate(&Some("hello".to_string())).is_ok());
/// assert!(validator.validate(&None::<String>).is_err());
/// ```
#[must_use]
pub fn required<T>() -> Required<T> {
    Required {
        _phantom: PhantomData,
    }
}

/// Alias for [`Required`].
///
/// This type alias provides an alternative name that may be more familiar
/// to users coming from other validation libraries or SQL contexts.
pub type NotNull<T> = Required<T>;

/// Creates a `NotNull` validator.
///
/// This is an alias for [`required`]. See that function for details.
///
/// # Type Parameters
///
/// * `T` - The inner type of the `Option` being validated
#[must_use]
pub fn not_null<T>() -> NotNull<T> {
    Required {
        _phantom: PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required() {
        assert!(required().validate(&Some(42)).is_ok());
        assert!(required().validate(&None::<i32>).is_err());
    }

    #[test]
    fn test_not_null() {
        assert!(not_null().validate(&Some("x")).is_ok());
        assert!(not_null().validate(&None::<&str>).is_err());
    }
}
