//! Nested field validators
//!
//! This module provides validators for nested structs and complex field types.

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
use std::marker::PhantomData;

// ============================================================================
// NESTED VALIDATOR
// ============================================================================

/// Validates a nested struct by calling its validation method.
///
/// This validator is useful when you have a struct field that itself needs
/// validation through its own `validate()` method.
///
/// # Type Parameters
///
/// * `T` - The nested type to validate
/// * `F` - The validation function type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::NestedValidator;
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct Address {
///     street: String,
///     city: String,
/// }
///
/// impl Address {
///     fn validate(&self) -> Result<(), ValidationError> {
///         if self.street.is_empty() {
///             return Err(ValidationError::new("street_required", "Street is required"));
///         }
///         Ok(())
///     }
/// }
///
/// let validator = NestedValidator::new(|addr: &Address| addr.validate());
/// ```
#[derive(Debug, Clone)]
pub struct NestedValidator<T, F> {
    validate_fn: F,
    _phantom: PhantomData<fn(&T)>,
}

impl<T, F> NestedValidator<T, F> {
    /// Creates a new nested validator from a validation function.
    ///
    /// # Arguments
    ///
    /// * `validate_fn` - A function that takes `&T` and returns `Result<(), ValidationError>`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::combinators::nested::NestedValidator;
    ///
    /// struct User {
    ///     name: String,
    /// }
    ///
    /// impl User {
    ///     fn validate(&self) -> Result<(), nebula_validator::core::ValidationError> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let validator = NestedValidator::new(|u: &User| u.validate());
    /// ```
    pub fn new(validate_fn: F) -> Self {
        Self {
            validate_fn,
            _phantom: PhantomData,
        }
    }
}

impl<T, F> TypedValidator for NestedValidator<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &T) -> Result<Self::Output, Self::Error> {
        (self.validate_fn)(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "NestedValidator".to_string(),
            description: Some("Validates nested struct by calling its validate method".to_string()),
            complexity: crate::core::ValidationComplexity::Linear,
            cacheable: false, // Nested validation may have side effects
            estimated_time: None,
            tags: vec!["nested".to_string(), "composite".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// VALIDATABLE TRAIT
// ============================================================================

/// Trait for types that can be validated.
///
/// This trait should be implemented by types that have their own validation logic.
/// It's automatically available for types using the `#[derive(Validator)]` macro.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::Validatable;
/// use nebula_validator::core::ValidationError;
///
/// struct User {
///     name: String,
///     email: String,
/// }
///
/// impl Validatable for User {
///     fn validate(&self) -> Result<(), ValidationError> {
///         if self.name.is_empty() {
///             return Err(ValidationError::new("name_required", "Name is required"));
///         }
///         if !self.email.contains('@') {
///             return Err(ValidationError::new("email_invalid", "Invalid email"));
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait Validatable {
    /// Validates the instance.
    ///
    /// Returns `Ok(())` if validation passes, or a `ValidationError` if it fails.
    fn validate(&self) -> Result<(), ValidationError>;
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a nested validator for types that implement `Validatable`.
///
/// This is a convenience function that creates a `NestedValidator` which
/// calls the `validate()` method of the type.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::{nested_validator, Validatable};
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct User {
///     name: String,
/// }
///
/// impl Validatable for User {
///     fn validate(&self) -> Result<(), ValidationError> {
///         Ok(())
///     }
/// }
///
/// let validator = nested_validator::<User>();
/// ```
pub fn nested_validator<T>() -> NestedValidator<T, impl Fn(&T) -> Result<(), ValidationError>>
where
    T: Validatable,
{
    NestedValidator::new(|input: &T| input.validate())
}

/// Creates a nested validator with a custom validation function.
///
/// This is useful when you need custom validation logic that doesn't
/// use the standard `validate()` method.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::custom_nested;
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct User {
///     age: u32,
/// }
///
/// let validator = custom_nested(|user: &User| {
///     if user.age < 18 {
///         Err(ValidationError::new("age_restriction", "Must be 18 or older"))
///     } else {
///         Ok(())
///     }
/// });
/// ```
pub fn custom_nested<T, F>(validate_fn: F) -> NestedValidator<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    NestedValidator::new(validate_fn)
}

// ============================================================================
// OPTIONAL NESTED VALIDATOR
// ============================================================================

/// Validates optional nested fields.
///
/// This validator handles `Option<T>` fields, validating the inner value
/// only if it's present.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::OptionalNested;
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct Address {
///     city: String,
/// }
///
/// impl Address {
///     fn validate(&self) -> Result<(), ValidationError> {
///         Ok(())
///     }
/// }
///
/// let validator = OptionalNested::new(|addr: &Address| addr.validate());
///
/// // None is always valid
/// assert!(validator.validate(&None).is_ok());
///
/// // Some(value) validates the inner value
/// let addr = Some(Address { city: "NYC".to_string() });
/// assert!(validator.validate(&addr).is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct OptionalNested<T, F> {
    validator: NestedValidator<T, F>,
}

impl<T, F> OptionalNested<T, F> {
    /// Creates a new optional nested validator.
    pub fn new(validate_fn: F) -> Self {
        Self {
            validator: NestedValidator::new(validate_fn),
        }
    }
}

impl<T, F> TypedValidator for OptionalNested<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = Option<T>;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Option<T>) -> Result<Self::Output, Self::Error> {
        match input {
            Some(value) => self.validator.validate(value),
            None => Ok(()),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "OptionalNested".to_string(),
            description: Some("Validates optional nested field".to_string()),
            complexity: crate::core::ValidationComplexity::Linear,
            cacheable: false,
            estimated_time: None,
            tags: vec!["nested".to_string(), "optional".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates an optional nested validator for types that implement `Validatable`.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::{optional_nested, Validatable};
/// use nebula_validator::core::ValidationError;
///
/// struct User {
///     name: String,
/// }
///
/// impl Validatable for User {
///     fn validate(&self) -> Result<(), ValidationError> {
///         Ok(())
///     }
/// }
///
/// let validator = optional_nested::<User>();
/// ```
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
/// This validator applies nested validation to each element in a collection,
/// collecting all validation errors.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::CollectionNested;
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct Item {
///     name: String,
/// }
///
/// impl Item {
///     fn validate(&self) -> Result<(), ValidationError> {
///         if self.name.is_empty() {
///             Err(ValidationError::new("name_required", "Name required"))
///         } else {
///             Ok(())
///         }
///     }
/// }
///
/// let validator = CollectionNested::new(|item: &Item| item.validate());
///
/// let items = vec![
///     Item { name: "Item 1".to_string() },
///     Item { name: "Item 2".to_string() },
/// ];
///
/// assert!(validator.validate(&items).is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct CollectionNested<T, F> {
    validator: NestedValidator<T, F>,
}

impl<T, F> CollectionNested<T, F> {
    /// Creates a new collection nested validator.
    pub fn new(validate_fn: F) -> Self {
        Self {
            validator: NestedValidator::new(validate_fn),
        }
    }
}

impl<T, F> TypedValidator for CollectionNested<T, F>
where
    F: Fn(&T) -> Result<(), ValidationError>,
{
    type Input = [T];
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &[T]) -> Result<Self::Output, Self::Error> {
        for (index, item) in input.iter().enumerate() {
            self.validator.validate(item).map_err(|e| {
                ValidationError::new(
                    e.code,
                    format!("Element {} validation failed: {}", index, e.message),
                )
            })?;
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "CollectionNested".to_string(),
            description: Some("Validates each element in a collection".to_string()),
            complexity: crate::core::ValidationComplexity::Linear,
            cacheable: false,
            estimated_time: None,
            tags: vec!["nested".to_string(), "collection".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a collection nested validator for types that implement `Validatable`.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::nested::{collection_nested, Validatable};
/// use nebula_validator::core::ValidationError;
///
/// struct User {
///     name: String,
/// }
///
/// impl Validatable for User {
///     fn validate(&self) -> Result<(), ValidationError> {
///         Ok(())
///     }
/// }
///
/// let validator = collection_nested::<User>();
/// ```
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
