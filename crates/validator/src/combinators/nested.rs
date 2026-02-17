//! Nested field validators
//!
//! This module provides validators for nested structs and complex field types.
//! It enables validation of custom types by delegating to their own validation logic.
//!
//! # Validators
//!
//! - [`NestedValidate`] - Validates a nested type using a custom validation function
//! - [`OptionalNested`] - Validates an optional nested type
//! - [`CollectionNested`] - Validates a collection of nested types
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::nested_validator;
//! use nebula_validator::foundation::Validate;
//!
//! // For types implementing Validatable trait
//! let validator = nested_validator::<MyStruct>();
//! assert!(validator.validate(&my_struct_instance).is_ok());
//! ```

use crate::foundation::{Validate, ValidationError};
use std::marker::PhantomData;

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
/// ```rust,ignore
/// use nebula_validator::combinators::NestedValidate;
/// use nebula_validator::foundation::Validate;
///
/// let validator = NestedValidate::new(|user: &User| {
///     if user.age >= 18 {
///         Ok(())
///     } else {
///         Err(ValidationError::new("age", "Must be 18+"))
///     }
/// });
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

impl<T, F> Validate for NestedValidate<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = T;

    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        (self.validate_fn)(input)
    }
}

// ============================================================================
// VALIDATABLE TRAIT
// ============================================================================

/// Trait for types that can validate themselves.
///
/// Types implementing this trait can be validated using the
/// [`nested_validator`] function.
///
/// # Examples
///
/// ```rust,ignore
/// impl Validatable for User {
///     fn validate(&self) -> Result<(), ValidationError> {
///         if self.name.is_empty() {
///             return Err(ValidationError::new("name", "Name is required"));
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait Validatable {
    /// Validates the instance.
    fn validate(&self) -> Result<(), ValidationError>;
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a nested validator for types that implement [`Validatable`].
///
/// # Type Parameters
///
/// * `T` - A type implementing [`Validatable`]
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::nested_validator;
/// use nebula_validator::foundation::Validate;
///
/// let validator = nested_validator::<User>();
/// assert!(validator.validate(&user).is_ok());
/// ```
#[must_use]
pub fn nested_validator<T>() -> NestedValidate<T, impl Fn(&T) -> Result<(), ValidationError>>
where
    T: Validatable,
{
    NestedValidate::new(|input: &T| input.validate())
}

/// Creates a nested validator with a custom validation function.
///
/// # Arguments
///
/// * `validate_fn` - A function that validates the nested type
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::custom_nested;
/// use nebula_validator::foundation::Validate;
///
/// let validator = custom_nested(|user: &User| {
///     if user.email.contains('@') {
///         Ok(())
///     } else {
///         Err(ValidationError::new("email", "Invalid email"))
///     }
/// });
/// ```
pub fn custom_nested<T, F>(validate_fn: F) -> NestedValidate<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    NestedValidate::new(validate_fn)
}

// ============================================================================
// OPTIONAL NESTED VALIDATOR
// ============================================================================

/// Validates an optional nested field.
///
/// Passes validation for `None` values and delegates to the nested
/// validator for `Some` values.
///
/// # Type Parameters
///
/// * `T` - The nested type being validated
/// * `F` - The validation function type
#[derive(Debug, Clone)]
pub struct OptionalNested<T, F> {
    validator: NestedValidate<T, F>,
}

impl<T, F> OptionalNested<T, F> {
    /// Creates a new optional nested validator.
    ///
    /// # Arguments
    ///
    /// * `validate_fn` - A function that validates the nested type
    pub fn new(validate_fn: F) -> Self {
        Self {
            validator: NestedValidate::new(validate_fn),
        }
    }
}

impl<T, F> Validate for OptionalNested<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = Option<T>;

    fn validate(&self, input: &Option<T>) -> Result<(), ValidationError> {
        match input {
            Some(value) => self.validator.validate(value),
            None => Ok(()),
        }
    }
}

/// Creates an optional nested validator for types that implement [`Validatable`].
///
/// # Type Parameters
///
/// * `T` - A type implementing [`Validatable`]
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::optional_nested;
/// use nebula_validator::foundation::Validate;
///
/// let validator = optional_nested::<Address>();
/// assert!(validator.validate(&None).is_ok());
/// assert!(validator.validate(&Some(address)).is_ok());
/// ```
#[must_use]
pub fn optional_nested<T>() -> OptionalNested<T, impl Fn(&T) -> Result<(), ValidationError>>
where
    T: Validatable,
{
    OptionalNested::new(|input: &T| input.validate())
}

// ============================================================================
// COLLECTION NESTED VALIDATOR
// ============================================================================

/// Validates each element in a collection of nested structs.
///
/// Iterates through a slice and validates each element using the
/// nested validator. Returns an error containing the index of the
/// first failing element.
///
/// # Type Parameters
///
/// * `T` - The nested type being validated
/// * `F` - The validation function type
#[derive(Debug, Clone)]
pub struct CollectionNested<T, F> {
    validator: NestedValidate<T, F>,
}

impl<T, F> CollectionNested<T, F> {
    /// Creates a new collection nested validator.
    ///
    /// # Arguments
    ///
    /// * `validate_fn` - A function that validates each element
    pub fn new(validate_fn: F) -> Self {
        Self {
            validator: NestedValidate::new(validate_fn),
        }
    }
}

impl<T, F> Validate for CollectionNested<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = [T];

    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        for (index, item) in input.iter().enumerate() {
            self.validator.validate(item).map_err(|e| {
                ValidationError::new(
                    e.code.clone(),
                    format!("Element {} validation failed: {}", index, e.message),
                )
            })?;
        }
        Ok(())
    }
}

/// Creates a collection nested validator for types that implement [`Validatable`].
///
/// # Type Parameters
///
/// * `T` - A type implementing [`Validatable`]
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::collection_nested;
/// use nebula_validator::foundation::Validate;
///
/// let validator = collection_nested::<User>();
/// let users = vec![user1, user2, user3];
/// assert!(validator.validate(&users).is_ok());
/// ```
#[must_use]
pub fn collection_nested<T>() -> CollectionNested<T, impl Fn(&T) -> Result<(), ValidationError>>
where
    T: Validatable,
{
    CollectionNested::new(|input: &T| input.validate())
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestUser {
        name: String,
        age: u32,
    }

    impl Validatable for TestUser {
        fn validate(&self) -> Result<(), ValidationError> {
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
            name: "".to_string(),
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

    #[test]
    fn test_optional_nested_some_valid() {
        let user = Some(TestUser {
            name: "John".to_string(),
            age: 25,
        });
        let validator = optional_nested::<TestUser>();
        assert!(validator.validate(&user).is_ok());
    }

    #[test]
    fn test_optional_nested_some_invalid() {
        let user = Some(TestUser {
            name: "".to_string(),
            age: 25,
        });
        let validator = optional_nested::<TestUser>();
        assert!(validator.validate(&user).is_err());
    }

    #[test]
    fn test_optional_nested_none() {
        let user: Option<TestUser> = None;
        let validator = optional_nested::<TestUser>();
        assert!(validator.validate(&user).is_ok());
    }

    #[test]
    fn test_collection_nested_all_valid() {
        let users = vec![
            TestUser {
                name: "John".to_string(),
                age: 25,
            },
            TestUser {
                name: "Jane".to_string(),
                age: 30,
            },
        ];
        let validator = collection_nested::<TestUser>();
        assert!(validator.validate(&users).is_ok());
    }

    #[test]
    fn test_collection_nested_one_invalid() {
        let users = vec![
            TestUser {
                name: "John".to_string(),
                age: 25,
            },
            TestUser {
                name: "".to_string(), // Invalid
                age: 30,
            },
        ];
        let validator = collection_nested::<TestUser>();
        let result = validator.validate(&users);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Element 1"));
    }

    #[test]
    fn test_collection_nested_empty() {
        let users: Vec<TestUser> = vec![];
        let validator = collection_nested::<TestUser>();
        assert!(validator.validate(&users).is_ok());
    }
}
