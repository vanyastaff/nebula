//! Boolean validators

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};

// ============================================================================
// IS TRUE
// ============================================================================

/// Validates that a boolean is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsTrue;

impl TypedValidator for IsTrue {
    type Input = bool;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if *input {
            Ok(())
        } else {
            Err(ValidationError::new("is_true", "Value must be true"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("IsTrue")
            .with_tag("logical")
            .with_tag("boolean")
    }
}

#[must_use]
pub const fn is_true() -> IsTrue {
    IsTrue
}

// ============================================================================
// IS FALSE
// ============================================================================

/// Validates that a boolean is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsFalse;

impl TypedValidator for IsFalse {
    type Input = bool;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if *input {
            Err(ValidationError::new("is_false", "Value must be false"))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("IsFalse")
            .with_tag("logical")
            .with_tag("boolean")
    }
}

#[must_use]
pub const fn is_false() -> IsFalse {
    IsFalse
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_true() {
        let validator = is_true();
        assert!(validator.validate(&true).is_ok());
        assert!(validator.validate(&false).is_err());
    }

    #[test]
    fn test_is_false() {
        let validator = is_false();
        assert!(validator.validate(&false).is_ok());
        assert!(validator.validate(&true).is_err());
    }
}
