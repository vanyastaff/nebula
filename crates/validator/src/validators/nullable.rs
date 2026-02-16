//! Nullable validators for Option types

use crate::foundation::{Validate, ValidationError};
use std::marker::PhantomData;

/// Validates that an `Option` is `Some`.
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

/// Creates a Required validator.
#[must_use]
pub fn required<T>() -> Required<T> {
    Required {
        _phantom: PhantomData,
    }
}

/// Alias for Required.
pub type NotNull<T> = Required<T>;

/// Creates a NotNull validator.
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
