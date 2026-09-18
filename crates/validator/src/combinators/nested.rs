//! Nested field validators
//!
//! This module provides validators for nested structs and complex field types.
//! It enables validation of custom types by delegating to their own validation logic.
//!
//! # Validators
//!
//! - [`NestedValidate`] - Validates a nested type using a custom validation function
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::combinators::{nested_validator, SelfValidating};
//! use nebula_validator::foundation::{Validate, ValidationError};
//!
//! # struct MyStruct { ok: bool }
//! # impl SelfValidating for MyStruct {
//! #     fn check(&self) -> Result<(), ValidationError> {
//! #         if self.ok { Ok(()) } else { Err(ValidationError::new("invalid", "bad")) }
//! #     }
//! # }
//! // For types implementing the SelfValidating trait
//! let validator = nested_validator::<MyStruct>();
//! assert!(validator.validate(&MyStruct { ok: true }).is_ok());
//! assert!(validator.validate(&MyStruct { ok: false }).is_err());
//! ```

use std::marker::PhantomData;

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// NESTED VALIDATOR
// ============================================================================

/// Validates a nested struct by calling its validation function.
///
/// This validator is useful when you have a custom type with its own
/// validation logic that you want to invoke from a parent validator.
///
/// # Type Parameters
///
/// * `T` - The type being validated
/// * `F` - The validation function type (`Fn(&T) -> Result<(), ValidationError>`)
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::NestedValidate;
/// use nebula_validator::foundation::{Validate, ValidationError};
///
/// struct User { age: u32 }
///
/// let validator = NestedValidate::new(|user: &User| {
///     if user.age >= 18 {
///         Ok(())
///     } else {
///         Err(ValidationError::new("age", "Must be 18+"))
///     }
/// });
///
/// assert!(validator.validate(&User { age: 21 }).is_ok());
/// assert!(validator.validate(&User { age: 16 }).is_err());
/// ```
#[derive(Debug, Clone)]
pub struct NestedValidate<T, F> {
    validate_fn: F,
    _phantom: PhantomData<fn(&T)>,
}

impl<T, F> NestedValidate<T, F> {
    /// Creates a new nested validator from a validation function.
    ///
    /// # Arguments
    ///
    /// * `validate_fn` - A function that validates the nested type
    pub fn new(validate_fn: F) -> Self {
        Self {
            validate_fn,
            _phantom: PhantomData,
        }
    }
}

impl<T, F> Validate<T> for NestedValidate<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        (self.validate_fn)(input)
    }
}

// ============================================================================
// SELF-VALIDATING TRAIT
// ============================================================================

/// Trait for types that can validate themselves.
///
/// Types implementing this trait can be validated using the
/// [`nested_validator`] function.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::SelfValidating;
/// use nebula_validator::foundation::ValidationError;
///
/// struct User { name: String }
///
/// impl SelfValidating for User {
///     fn check(&self) -> Result<(), ValidationError> {
///         if self.name.is_empty() {
///             return Err(ValidationError::new("name", "Name is required"));
///         }
///         Ok(())
///     }
/// }
///
/// assert!(User { name: "Ada".to_string() }.check().is_ok());
/// assert!(User { name: String::new() }.check().is_err());
/// ```
pub trait SelfValidating {
    /// Validates the instance and returns an error if invalid.
    fn check(&self) -> Result<(), ValidationError>;
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a nested validator for types that implement [`SelfValidating`].
///
/// # Type Parameters
///
/// * `T` - A type implementing [`SelfValidating`]
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::{nested_validator, SelfValidating};
/// use nebula_validator::foundation::{Validate, ValidationError};
///
/// # struct User { age: u32 }
/// # impl SelfValidating for User {
/// #     fn check(&self) -> Result<(), ValidationError> {
/// #         if self.age >= 18 { Ok(()) } else { Err(ValidationError::new("age", "18+")) }
/// #     }
/// # }
/// let validator = nested_validator::<User>();
/// assert!(validator.validate(&User { age: 30 }).is_ok());
/// assert!(validator.validate(&User { age: 10 }).is_err());
/// ```
#[must_use]
pub fn nested_validator<T>() -> NestedValidate<T, impl Fn(&T) -> Result<(), ValidationError>>
where
    T: SelfValidating,
{
    NestedValidate::new(|input: &T| input.check())
}

/// Creates a nested validator with a custom validation function.
///
/// # Arguments
///
/// * `validate_fn` - A function that validates the nested type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::custom_nested;
/// use nebula_validator::foundation::{Validate, ValidationError};
///
/// struct User { email: String }
///
/// let validator = custom_nested(|user: &User| {
///     if user.email.contains('@') {
///         Ok(())
///     } else {
///         Err(ValidationError::new("email", "Invalid email"))
///     }
/// });
///
/// assert!(validator.validate(&User { email: "a@b.com".to_string() }).is_ok());
/// assert!(validator.validate(&User { email: "nope".to_string() }).is_err());
/// ```
pub fn custom_nested<T, F>(validate_fn: F) -> NestedValidate<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    NestedValidate::new(validate_fn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestUser {
        name: String,
        age: u32,
    }

    impl SelfValidating for TestUser {
        fn check(&self) -> Result<(), ValidationError> {
            if self.name.is_empty() {
                return Err(ValidationError::new("name_required", "Name is required"));
            }
            if self.age < 18 {
                return Err(ValidationError::new("age_restriction", "Must be 18+"));
            }
            Ok(())
        }
    }

    #[test]
    fn test_nested_validator_valid() {
        let user = TestUser {
            name: "John".to_string(),
            age: 25,
        };
        let validator = nested_validator::<TestUser>();
        assert!(validator.validate(&user).is_ok());
    }

    #[test]
    fn test_nested_validator_invalid_name() {
        let user = TestUser {
            name: String::new(),
            age: 25,
        };
        let validator = nested_validator::<TestUser>();
        let result = validator.validate(&user);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "name_required");
    }

    #[test]
    fn test_nested_validator_invalid_age() {
        let user = TestUser {
            name: "John".to_string(),
            age: 15,
        };
        let validator = nested_validator::<TestUser>();
        let result = validator.validate(&user);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "age_restriction");
    }

    #[test]
    fn test_custom_nested() {
        let user = TestUser {
            name: "John".to_string(),
            age: 20,
        };
        let validator = custom_nested(|u: &TestUser| {
            if u.age < 21 {
                Err(ValidationError::new("drinking_age", "Must be 21+"))
            } else {
                Ok(())
            }
        });
        assert!(validator.validate(&user).is_err());
    }
}
