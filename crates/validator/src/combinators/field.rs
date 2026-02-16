//! FIELD combinator - validates specific fields of structs
//!
//! The FIELD combinator allows validating individual fields of a struct
//! without requiring derive macros.

use crate::combinators::error::CombinatorError;
use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};
use std::borrow::Cow;
use std::marker::PhantomData;

// ============================================================================
// FIELD COMBINATOR
// ============================================================================

/// Validates a specific field of a struct.
///
/// # Type Parameters
///
/// * `T` - The parent struct type
/// * `U` - The field type (can be `?Sized`)
/// * `V` - The validator type
/// * `F` - The accessor function type
pub struct Field<T, U, V, F>
where
    U: ?Sized,
{
    name: Option<String>,
    validator: V,
    accessor: F,
    _phantom: PhantomData<fn(&T) -> &U>,
}

impl<T, U, V, F> Field<T, U, V, F>
where
    U: ?Sized,
{
    /// Creates a new field validator without a name.
    pub fn new(validator: V, accessor: F) -> Self {
        Self {
            name: None,
            validator,
            accessor,
            _phantom: PhantomData,
        }
    }

    /// Creates a new field validator with a name.
    pub fn named(name: impl Into<String>, validator: V, accessor: F) -> Self {
        Self {
            name: Some(name.into()),
            validator,
            accessor,
            _phantom: PhantomData,
        }
    }

    /// Returns the field name, if any.
    pub fn field_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns a reference to the accessor function.
    pub fn accessor(&self) -> &F {
        &self.accessor
    }

    /// Extracts the validator and accessor.
    pub fn into_parts(self) -> (Option<String>, V, F) {
        (self.name, self.validator, self.accessor)
    }
}

// Clone impl - manual because F might not derive Clone
impl<T, U, V, F> Clone for Field<T, U, V, F>
where
    V: Clone,
    F: Clone,
    U: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            validator: self.validator.clone(),
            accessor: self.accessor.clone(),
            _phantom: PhantomData,
        }
    }
}

// Debug impl
impl<T, U, V, F> std::fmt::Debug for Field<T, U, V, F>
where
    V: std::fmt::Debug,
    U: ?Sized,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("name", &self.name)
            .field("validator", &self.validator)
            .field("accessor", &"<function>")
            .finish()
    }
}

// ============================================================================
// FIELD ERROR TYPE
// ============================================================================

/// Error wrapper that includes field name context.
#[derive(Debug, Clone)]
pub struct FieldError {
    /// Name of the field that failed validation
    pub field_name: Option<String>,
    /// The underlying validation error
    pub inner: ValidationError,
}

impl FieldError {
    /// Creates a new field error.
    pub fn new(field_name: Option<String>, inner: ValidationError) -> Self {
        Self { field_name, inner }
    }

    /// Returns the field name, if any.
    pub fn field_name(&self) -> Option<&str> {
        self.field_name.as_deref()
    }

    /// Returns a reference to the inner error.
    pub fn inner(&self) -> &ValidationError {
        &self.inner
    }

    /// Consumes the error and returns the inner error.
    pub fn into_inner(self) -> ValidationError {
        self.inner
    }

    /// Adds a field name to an unnamed error.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_field_name(mut self, name: impl Into<String>) -> Self {
        self.field_name = Some(name.into());
        self
    }
}

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = &self.field_name {
            write!(f, "Field '{}': {}", name, self.inner)
        } else {
            write!(f, "Field validation failed: {}", self.inner)
        }
    }
}

impl std::error::Error for FieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

// ============================================================================
// CONVERSIONS
// ============================================================================

/// Convert `FieldError` to `CombinatorError`
impl From<FieldError> for CombinatorError<ValidationError> {
    fn from(error: FieldError) -> Self {
        CombinatorError::FieldFailed {
            field_name: error.field_name,
            error: Box::new(error.inner),
        }
    }
}

/// Convert `FieldError` to `ValidationError`
impl From<FieldError> for ValidationError {
    fn from(error: FieldError) -> Self {
        let mut validation_error =
            ValidationError::new("field_validation", format!("{}", error.inner));

        if let Some(field_name) = error.field_name {
            validation_error = validation_error.with_field(field_name);
        }

        validation_error
    }
}

// ============================================================================
// VALIDATOR IMPLEMENTATION
// ============================================================================

impl<T, U, V, F> Validate for Field<T, U, V, F>
where
    V: Validate<Input = U>,
    F: Fn(&T) -> &U,
    U: ?Sized,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let field_value = (self.accessor)(input);
        self.validator.validate(field_value).map_err(|err| {
            let field_error = FieldError::new(self.name.clone(), err);
            ValidationError::from(field_error)
        })
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: if let Some(name) = &self.name {
                format!("Field('{}', {})", name, inner_meta.name).into()
            } else {
                format!("Field({})", inner_meta.name).into()
            },
            description: Some(
                format!(
                    "Validates field{} using {}",
                    self.name
                        .as_ref()
                        .map(|n| format!(" '{n}'"))
                        .unwrap_or_default(),
                    inner_meta.name
                )
                .into(),
            ),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push(Cow::Borrowed("combinator"));
                tags.push(Cow::Borrowed("field"));
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
    U: ?Sized,
{
    Field::new(validator, accessor)
}

/// Creates a field validator with a name.
pub fn named_field<T, U, V, F>(
    name: impl Into<String>,
    validator: V,
    accessor: F,
) -> Field<T, U, V, F>
where
    U: ?Sized,
{
    Field::named(name, validator, accessor)
}

// ============================================================================
// EXTENSION TRAIT
// ============================================================================

/// Extension trait for creating field validators.
pub trait FieldValidateExt: Validate + Sized {
    /// Creates a field validator for this validator.
    fn for_field<T, F>(self, name: impl Into<String>, accessor: F) -> Field<T, Self::Input, Self, F>
    where
        F: Fn(&T) -> &Self::Input,
    {
        Field::named(name, self, accessor)
    }

    /// Creates an unnamed field validator.
    fn for_field_unnamed<T, F>(self, accessor: F) -> Field<T, Self::Input, Self, F>
    where
        F: Fn(&T) -> &Self::Input,
    {
        Field::new(self, accessor)
    }
}

impl<V: Validate> FieldValidateExt for V {}

// ============================================================================
// MULTI-FIELD VALIDATOR
// ============================================================================

/// Validates multiple fields of a struct.
pub struct MultiField<T> {
    validators: Vec<Box<dyn Fn(&T) -> Result<(), ValidationError> + Send + Sync>>,
}

impl<T> MultiField<T> {
    /// Creates a new multi-field validator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Adds a field validator.
    #[must_use = "builder methods must be chained or built"]
    pub fn add_field<U, V, F>(mut self, name: impl Into<String>, validator: V, accessor: F) -> Self
    where
        U: ?Sized,
        V: Validate<Input = U> + Send + Sync + 'static,
        F: Fn(&T) -> &U + Send + Sync + 'static,
    {
        let name = name.into();
        self.validators.push(Box::new(move |input: &T| {
            let field_value = accessor(input);
            validator.validate(field_value).map_err(|err| {
                ValidationError::new("field_validation", format!("{err}")).with_field(name.clone())
            })
        }));
        self
    }
}

impl<T> Default for MultiField<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Validate for MultiField<T> {
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            if let Err(err) = validator(input) {
                errors.push(err);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else if errors.len() == 1 {
            Err(errors
                .into_iter()
                .next()
                .expect("errors.len() == 1 guarantees next() succeeds"))
        } else {
            Err(
                ValidationError::new("multiple_field_errors", "Multiple field validation errors")
                    .with_nested(errors),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: format!("MultiField(fields={})", self.validators.len()).into(),
            description: Some(format!("Validates {} fields", self.validators.len()).into()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec!["combinator".into(), "field".into(), "multi".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct TestUser {
        name: String,
        age: u32,
    }

    #[derive(Clone, Debug)]
    struct MinValue {
        min: u32,
    }

    impl Validate for MinValue {
        type Input = u32;

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

    #[derive(Clone, Debug)]
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

    fn get_age(u: &TestUser) -> &u32 {
        &u.age
    }

    fn get_name(u: &TestUser) -> &str {
        &u.name
    }

    #[test]
    fn test_field_basic() {
        let user = TestUser {
            name: "Alice".to_string(),
            age: 25,
        };
        let age_validator = field(MinValue { min: 18 }, get_age);
        assert!(age_validator.validate(&user).is_ok());
    }

    #[test]
    fn test_field_named() {
        let user = TestUser {
            name: "Al".to_string(),
            age: 15,
        };
        let age_validator = named_field("age", MinValue { min: 18 }, get_age);

        let err = age_validator.validate(&user).unwrap_err();
        assert!(err.to_string().contains("age"));
    }

    #[test]
    fn test_field_error_display() {
        let error = FieldError::new(
            Some("age".to_string()),
            ValidationError::new("invalid", "Invalid value"),
        );

        let display = format!("{}", error);
        assert!(display.contains("age"));
        assert!(display.contains("Invalid value"));
    }

    #[test]
    fn test_field_extension_trait() {
        let user = TestUser {
            name: "Alice".to_string(),
            age: 25,
        };
        let age_validator = MinValue { min: 18 }.for_field("age", get_age);
        assert!(age_validator.validate(&user).is_ok());
    }

    #[test]
    fn test_field_name_accessor() {
        let validator: Field<TestUser, u32, _, _> =
            named_field("age", MinValue { min: 18 }, get_age);
        assert_eq!(validator.field_name(), Some("age"));
    }

    #[test]
    fn test_field_with_str() {
        let user = TestUser {
            name: "Alice".to_string(),
            age: 25,
        };
        let name_validator = named_field("name", MinLength { min: 3 }, get_name);
        assert!(name_validator.validate(&user).is_ok());

        let user_short = TestUser {
            name: "Al".to_string(),
            age: 25,
        };
        assert!(name_validator.validate(&user_short).is_err());
    }

    #[test]
    fn test_multi_field() {
        let user = TestUser {
            name: "Alice".to_string(),
            age: 25,
        };

        let validator = MultiField::new()
            .add_field("name", MinLength { min: 3 }, get_name)
            .add_field("age", MinValue { min: 18 }, get_age);

        assert!(validator.validate(&user).is_ok());

        let invalid_user = TestUser {
            name: "Al".to_string(),
            age: 15,
        };

        let result = validator.validate(&invalid_user);
        assert!(result.is_err());
    }

    #[test]
    fn test_field_into_parts() {
        let validator: Field<TestUser, u32, _, _> =
            named_field("age", MinValue { min: 18 }, get_age);
        let (name, min_value, _accessor) = validator.into_parts();
        assert_eq!(name, Some("age".to_string()));
        assert_eq!(min_value.min, 18);
    }

    #[test]
    fn test_field_with_dynamic_name() {
        // Test that String (not &'static str) works
        let field_name = format!("field_{}", 42);
        let validator: Field<TestUser, u32, MinValue, _> =
            named_field(field_name.clone(), MinValue { min: 18 }, get_age);
        assert_eq!(validator.field_name(), Some(field_name.as_str()));
    }

    #[test]
    fn test_field_clone() {
        let validator = named_field("age", MinValue { min: 18 }, get_age);
        let cloned = validator.clone();

        let user = TestUser {
            name: "Alice".to_string(),
            age: 25,
        };

        assert!(validator.validate(&user).is_ok());
        assert!(cloned.validate(&user).is_ok());
    }

    #[test]
    fn test_field_debug() {
        let validator: Field<TestUser, u32, MinValue, _> =
            named_field("age", MinValue { min: 18 }, get_age);
        let debug_str = format!("{:?}", validator);
        assert!(debug_str.contains("Field"));
        assert!(debug_str.contains("age"));
    }
}
