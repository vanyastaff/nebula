//! Nullable validators

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
use std::marker::PhantomData;

// ============================================================================
// REQUIRED
// ============================================================================

/// Validates that an Option is Some (not None).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Required<T> {
    _phantom: PhantomData<T>,
}

impl<T> TypedValidator for Required<T> {
    type Input = Option<T>;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.is_some() {
            Ok(())
        } else {
            Err(ValidationError::required(""))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Required")
            .with_tag("logical")
            .with_tag("nullable")
    }
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
        assert!(validator.validate(&None::<String>).is_err());
    }
}
