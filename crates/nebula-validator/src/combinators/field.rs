//! FIELD combinator - validates specific fields of structs
//!
//! The FIELD combinator allows validating individual fields of a struct
//! without requiring derive macros.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::field::*;
//!
//! struct User {
//!     age: u32,
//! }
//!
//! // Define an accessor function
//! fn get_age(user: &User) -> &u32 {
//!     &user.age
//! }
//!
//! // Validate a single field
//! let age_validator = named_field("age", min_value(18), get_age);
//!
//! let user = User { age: 25 };
//! assert!(age_validator.validate(&user).is_ok());
//! ```

use crate::core::{TypedValidator, ValidatorMetadata};

// ============================================================================
// FIELD COMBINATOR
// ============================================================================

/// Validates a specific field of a struct.
///
/// Due to Rust's lifetime limitations with closures accessing struct fields,
/// field accessors must be defined as separate functions rather than closures.
///
/// # Examples
///
/// ```rust,ignore
/// struct User {
///     age: u32,
/// }
///
/// fn get_age(u: &User) -> &u32 { &u.age }
///
/// let validator = named_field("age", min_value(18), get_age);
/// ```
pub struct Field<T, U, V, F>
where
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    name: Option<&'static str>,
    validator: V,
    accessor: F,
    _phantom: std::marker::PhantomData<fn() -> (T, U)>,
}

impl<T, U, V, F> Field<T, U, V, F>
where
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    /// Creates a new field validator without a name.
    pub fn new(validator: V, accessor: F) -> Self {
        Self {
            name: None,
            validator,
            accessor,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a new field validator with a name.
    pub fn named(name: &'static str, validator: V, accessor: F) -> Self {
        Self {
            name: Some(name),
            validator,
            accessor,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns the field name, if any.
    pub fn field_name(&self) -> Option<&'static str> {
        self.name
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }
}

// ============================================================================
// FIELD ERROR TYPE
// ============================================================================

/// Error wrapper that includes field name context.
#[derive(Debug, Clone)]
pub struct FieldError<E> {
    pub field_name: Option<&'static str>,
    pub inner: E,
}

impl<E> FieldError<E> {
    pub fn new(field_name: Option<&'static str>, inner: E) -> Self {
        Self { field_name, inner }
    }

    pub fn field_name(&self) -> Option<&'static str> {
        self.field_name
    }

    pub fn inner(&self) -> &E {
        &self.inner
    }

    pub fn into_inner(self) -> E {
        self.inner
    }
}

impl<E: std::fmt::Display> std::fmt::Display for FieldError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = self.field_name {
            write!(f, "Validation failed for field '{}': {}", name, self.inner)
        } else {
            write!(f, "Validation failed for field: {}", self.inner)
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for FieldError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<T, U, V, F> TypedValidator for Field<T, U, V, F>
where
    V: TypedValidator<Input = U>,
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    type Input = T;
    type Output = ();
    type Error = FieldError<V::Error>;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let field_value = (self.accessor)(input);
        self.validator
            .validate(field_value)
            .map(|_| ())
            .map_err(|err| FieldError::new(self.name, err))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: if let Some(name) = self.name {
                format!("Field('{}', {})", name, inner_meta.name)
            } else {
                format!("Field({})", inner_meta.name)
            },
            description: Some(format!(
                "Validates field{} using {}",
                self.name.map(|n| format!(" '{}'", n)).unwrap_or_default(),
                inner_meta.name
            )),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("field".to_string());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a field validator without a name.
pub fn field<T, U, V, F>(validator: V, accessor: F) -> Field<T, U, V, F>
where
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    Field::new(validator, accessor)
}

/// Creates a field validator with a name.
pub fn named_field<T, U, V, F>(name: &'static str, validator: V, accessor: F) -> Field<T, U, V, F>
where
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    Field::named(name, validator, accessor)
}

// ============================================================================
// EXTENSION TRAIT
// ============================================================================

/// Extension trait for creating field validators.
pub trait FieldValidatorExt: TypedValidator + Sized {
    /// Creates a field validator for this validator.
    fn for_field<T, F>(self, name: &'static str, accessor: F) -> Field<T, Self::Input, Self, F>
    where
        F: Fn(&T) -> &Self::Input,
    {
        Field::named(name, self, accessor)
    }
}

impl<V: TypedValidator> FieldValidatorExt for V {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ValidationError;

    struct TestUser {
        age: u32,
    }

    struct MinValue {
        min: u32,
    }

    impl TypedValidator for MinValue {
        type Input = u32;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &u32) -> Result<(), ValidationError> {
            if *input >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "min_value",
                    format!("Must be at least {}", self.min),
                ))
            }
        }
    }

    fn get_age(u: &TestUser) -> &u32 {
        &u.age
    }

    #[test]
    fn test_field_basic() {
        let user = TestUser { age: 25 };
        let age_validator = field(MinValue { min: 18 }, get_age);
        assert!(age_validator.validate(&user).is_ok());
    }

    #[test]
    fn test_field_named() {
        let user = TestUser { age: 15 };
        let age_validator = named_field("age", MinValue { min: 18 }, get_age);

        let err = age_validator.validate(&user).unwrap_err();
        assert_eq!(err.field_name(), Some("age"));
        assert!(err.to_string().contains("field 'age'"));
    }

    #[test]
    fn test_field_error_display() {
        let error = FieldError::new(
            Some("age"),
            ValidationError::new("invalid", "Invalid value"),
        );

        let display = format!("{}", error);
        assert!(display.contains("field 'age'"));
        assert!(display.contains("Invalid value"));
    }

    #[test]
    fn test_field_extension_trait() {
        let user = TestUser { age: 25 };
        let age_validator = MinValue { min: 18 }.for_field("age", get_age);
        assert!(age_validator.validate(&user).is_ok());
    }

    #[test]
    fn test_field_name_accessor() {
        let validator = named_field("age", MinValue { min: 18 }, get_age);
        assert_eq!(validator.field_name(), Some("age"));
    }
}
