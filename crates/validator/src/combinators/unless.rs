//! UNLESS combinator - inverse conditional validation

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// UNLESS COMBINATOR
// ============================================================================

/// Conditionally skips validation when a predicate is true.
///
/// This is the inverse of `When` - validates only when condition is false.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Unless;
/// use nebula_validator::foundation::Validate;
///
/// // Skip validation for admin users
/// let validator = Unless::new(
///     MinLength { min: 10 },
///     |user: &User| user.is_admin
/// );
///
/// // Admin bypasses validation
/// assert!(validator.validate(&admin_user).is_ok());
///
/// // Regular user must pass validation
/// assert!(validator.validate(&regular_user_short_password).is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Unless<V, C> {
    validator: V,
    condition: C,
}

impl<V, C> Unless<V, C> {
    /// Creates a new UNLESS combinator.
    ///
    /// Validation is skipped when `condition` returns true.
    pub fn new(validator: V, condition: C) -> Self {
        Self {
            validator,
            condition,
        }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns a reference to the condition.
    pub fn condition(&self) -> &C {
        &self.condition
    }

    /// Extracts the validator and condition.
    pub fn into_parts(self) -> (V, C) {
        (self.validator, self.condition)
    }
}

impl<V, C> Validate for Unless<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if (self.condition)(input) {
            // Condition is true, skip validation
            Ok(())
        } else {
            // Condition is false, run validation
            self.validator.validate(input)
        }
    }
}

/// Creates an UNLESS combinator.
///
/// Validation is skipped when `condition` returns true.
pub fn unless<V, C>(validator: V, condition: C) -> Unless<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    Unless::new(validator, condition)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = str;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "min_length",
                    format!("Must be at least {} characters", self.min),
                ))
            }
        }
    }

    #[test]
    fn test_unless_condition_true_skips() {
        // Skip validation for strings starting with "skip_"
        let validator = Unless::new(MinLength { min: 10 }, |s: &str| s.starts_with("skip_"));

        // Condition true - validation skipped
        assert!(validator.validate("skip_").is_ok());
        assert!(validator.validate("skip_x").is_ok());
    }

    #[test]
    fn test_unless_condition_false_validates() {
        let validator = Unless::new(MinLength { min: 10 }, |s: &str| s.starts_with("skip_"));

        // Condition false - validation runs
        assert!(validator.validate("short").is_err());
        assert!(validator.validate("long_enough_string").is_ok());
    }

    #[test]
    fn test_unless_empty_condition() {
        // Skip validation for empty strings
        let validator = Unless::new(MinLength { min: 5 }, |s: &str| s.is_empty());

        assert!(validator.validate("").is_ok()); // Empty - skipped
        assert!(validator.validate("hi").is_err()); // Not empty - validated, fails
        assert!(validator.validate("hello").is_ok()); // Not empty - validated, passes
    }

    #[test]
    fn test_unless_helper() {
        let validator = unless(MinLength { min: 10 }, |s: &str| s.starts_with("admin:"));

        assert!(validator.validate("admin:x").is_ok()); // Admin - skipped
        assert!(validator.validate("user:hi").is_err()); // User - validated (7 chars < 10)
        assert!(validator.validate("user:hello_world").is_ok()); // User - validated (16 chars >= 10)
    }
}
