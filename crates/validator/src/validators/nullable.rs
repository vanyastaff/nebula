//! Nullable validators

use crate::foundation::{Validate, ValidationError};
use std::marker::PhantomData;

// ============================================================================
// REQUIRED
// ============================================================================

/// Validates that an Option is Some (not None).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Required<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validate for Required<T> {
    type Input = Option<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_some() {
            Ok(())
        } else {
            Err(ValidationError::required(""))
        }
    }

    crate::validator_metadata!(
        "Required",
        "Value is required",
        complexity = Constant,
        tags = ["logical", "nullable"]
    );
}

#[must_use]
pub fn required<T>() -> Required<T> {
    Required {
        _phantom: PhantomData,
    }
}

// ============================================================================
// NOT NULL (alias)
// ============================================================================

/// Validates that a value is not null.
/// This is an alias for Required.
pub type NotNull<T> = Required<T>;

#[must_use]
pub fn not_null<T>() -> NotNull<T> {
    Required {
        _phantom: PhantomData,
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required() {
        let validator = required();
        assert!(validator.validate(&Some(42)).is_ok());
        assert!(validator.validate(&None::<i32>).is_err());
    }

    #[test]
    fn test_not_null() {
        let validator = not_null();
        assert!(validator.validate(&Some("hello")).is_ok());
        assert!(validator.validate(&None::<&str>).is_err());
    }
}
