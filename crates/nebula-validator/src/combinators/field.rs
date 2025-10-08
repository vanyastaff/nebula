//! FIELD combinator - validates specific fields of structs
//!
//! The FIELD combinator allows validating individual fields of a struct
//! without requiring derive macros.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::combinators::field::*;
//! use nebula_validator::core::TypedValidator;
//!
//! struct User {
//!     name: String,
//!     age: u32,
//! }
//!
//! // Define accessor functions
//! fn get_name(user: &User) -> &str {
//!     &user.name
//! }
//!
//! fn get_age(user: &User) -> &u32 {
//!     &user.age
//! }
//!
//! // Validate fields
//! let name_validator = named_field("name", min_length(3), get_name);
//! let age_validator = named_field("age", min_value(18), get_age);
//! ```

use crate::combinators::error::CombinatorError;
use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
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
///
/// # Examples
///
/// ```rust
/// # struct User { age: u32 }
/// # fn get_age(u: &User) -> &u32 { &u.age }
/// use nebula_validator::combinators::field::named_field;
///
/// let validator = named_field("age", min_value(18), get_age);
/// ```
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
///
/// This error type wraps the inner validator's error and adds field context.
#[derive(Debug, Clone)]
pub struct FieldError<E> {
    pub field_name: Option<String>,
    pub inner: E,
}

impl<E> FieldError<E> {
    /// Creates a new field error.
    pub fn new(field_name: Option<String>, inner: E) -> Self {
        Self { field_name, inner }
    }

    /// Returns the field name, if any.
    pub fn field_name(&self) -> Option<&str> {
        self.field_name.as_deref()
    }

    /// Returns a reference to the inner error.
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Consumes the error and returns the inner error.
    pub fn into_inner(self) -> E {
        self.inner
    }

    /// Maps the inner error using a function.
    pub fn map_inner<F, E2>(self, f: F) -> FieldError<E2>
    where
        F: FnOnce(E) -> E2,
    {
        FieldError {
            field_name: self.field_name,
            inner: f(self.inner),
        }
    }

    /// Adds a field name to an unnamed error.
    pub fn with_field_name(mut self, name: impl Into<String>) -> Self {
        self.field_name = Some(name.into());
        self
    }
}

impl<E: std::fmt::Display> std::fmt::Display for FieldError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = &self.field_name {
            write!(f, "Field '{}': {}", name, self.inner)
        } else {
            write!(f, "Field validation failed: {}", self.inner)
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for FieldError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

// ============================================================================
// CONVERSIONS
// ============================================================================

/// Convert FieldError to CombinatorError
impl<E> From<FieldError<E>> for CombinatorError<E> {
    fn from(error: FieldError<E>) -> Self {
        CombinatorError::FieldFailed {
            field_name: error.field_name,
            error: Box::new(error.inner),
        }
    }
}

/// Convert FieldError to ValidationError
impl<E: std::fmt::Display> From<FieldError<E>> for ValidationError {
    fn from(error: FieldError<E>) -> Self {
        let mut validation_error =
            ValidationError::new("field_validation", format!("{}", error.inner));

        if let Some(field_name) = error.field_name {
            validation_error = validation_error.with_field(field_name);
        }

        validation_error
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
            .map_err(|err| FieldError::new(self.name.clone(), err))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: if let Some(name) = &self.name {
                format!("Field('{}', {})", name, inner_meta.name)
            } else {
                format!("Field({})", inner_meta.name)
            },
            description: Some(format!(
                "Validates field{} using {}",
                self.name
                    .as_ref()
                    .map(|n| format!(" '{}'", n))
                    .unwrap_or_default(),
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
// ASYNC VALIDATOR IMPLEMENTATION
// ============================================================================

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<T, U, V, F> crate::core::AsyncValidator for Field<T, U, V, F>
where
    T: Sync,
    U: ?Sized + Sync,
    V: TypedValidator<Input = U>
        + crate::core::AsyncValidator<
            Input = U,
            Output = <V as TypedValidator>::Output,
            Error = <V as TypedValidator>::Error,
        > + Send
        + Sync,
    F: Fn(&T) -> &U + Send + Sync,
{
    type Input = T;
    type Output = ();
    type Error = FieldError<<V as TypedValidator>::Error>;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let field_value = (self.accessor)(input);
        self.validator
            .validate_async(field_value)
            .await
            .map(|_| ())
            .map_err(|err| FieldError::new(self.name.clone(), err))
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
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
pub trait FieldValidatorExt: TypedValidator + Sized {
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

impl<V: TypedValidator> FieldValidatorExt for V {}

// ============================================================================
// MULTI-FIELD VALIDATOR
// ============================================================================

/// Validates multiple fields of a struct.
///
/// # Examples
///
/// ```rust
/// # struct User { name: String, age: u32 }
/// # fn get_name(u: &User) -> &str { &u.name }
/// # fn get_age(u: &User) -> &u32 { &u.age }
/// use nebula_validator::combinators::field::MultiField;
///
/// let validator = MultiField::new()
///     .add_field("name", min_length(3), get_name)
///     .add_field("age", min_value(18), get_age);
/// ```
pub struct MultiField<T> {
    validators: Vec<Box<dyn Fn(&T) -> Result<(), ValidationError> + Send + Sync>>,
}

impl<T> MultiField<T> {
    /// Creates a new multi-field validator.
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Adds a field validator.
    pub fn add_field<U, V, F>(
        mut self,
        name: impl Into<String>,
        validator: V,
        accessor: F,
    ) -> Self
    where
        U: ?Sized,
        V: TypedValidator<Input = U> + Send + Sync + 'static,
        V::Error: std::fmt::Display,
        F: Fn(&T) -> &U + Send + Sync + 'static,
    {
        let name = name.into();
        self.validators.push(Box::new(move |input: &T| {
            let field_value = accessor(input);
            validator
                .validate(field_value)
                .map(|_| ())
                .map_err(|err| {
                    ValidationError::new("field_validation", format!("{}", err))
                        .with_field(name.clone())
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

impl<T> TypedValidator for MultiField<T> {
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            if let Err(err) = validator(input) {
                errors.push(err);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else if errors.len() == 1 {
            Err(errors.into_iter().next().unwrap())
        } else {
            Err(ValidationError::new(
                "multiple_field_errors",
                "Multiple field validation errors",
            )
            .with_nested(errors))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: format!("MultiField(fields={})", self.validators.len()),
            description: Some(format!("Validates {} fields", self.validators.len())),
            complexity: crate::core::ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec![
                "combinator".to_string(),
                "field".to_string(),
                "multi".to_string(),
            ],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// NESTED VALIDATOR
// ============================================================================

/// Validator wrapper for nested struct validation.
///
/// This validator wraps a function that validates a nested struct,
/// typically by calling its `.validate()` method.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::field::NestedValidator;
/// use nebula_validator::core::TypedValidator;
///
/// struct Address {
///     street: String,
/// }
///
/// impl Address {
///     fn validate(&self) -> Result<(), String> {
///         if self.street.is_empty() {
///             Err("Street cannot be empty".to_string())
///         } else {
///             Ok(())
///         }
///     }
/// }
///
/// let validator = NestedValidator::new(|addr: &Address| {
///     addr.validate().map_err(|e| e.into())
/// });
/// ```
#[derive(Debug, Clone)]
pub struct NestedValidator<T, F> {
    validate_fn: F,
    _phantom: PhantomData<fn(&T)>,
}

impl<T, F> NestedValidator<T, F> {
    /// Creates a new nested validator from a validation function.
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

/// Helper function to create a nested validator.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::field::nested_validator;
///
/// struct User {
///     name: String,
/// }
///
/// impl User {
///     fn validate(&self) -> Result<(), nebula_validator::core::ValidationError> {
///         // validation logic
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

/// Trait for types that can be validated.
///
/// This trait is automatically implemented for any type that has a
/// `validate()` method returning `Result<(), ValidationError>`.
pub trait Validatable {
    /// Validates the instance.
    fn validate(&self) -> Result<(), ValidationError>;
}

// Auto-implement for any type with validate method
// (This would need to be implemented by derive macro or manually)

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

    struct MinLength {
        min: usize,
    }

    impl TypedValidator for MinLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

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
        assert_eq!(err.field_name(), Some("age"));
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
        let validator = named_field("age", MinValue { min: 18 }, get_age);
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
    fn test_field_error_conversion_to_combinator() {
        let field_error = FieldError::new(
            Some("age".to_string()),
            ValidationError::new("min_value", "Too small"),
        );

        let combinator_error: CombinatorError<ValidationError> = field_error.into();
        assert!(combinator_error.is_field_error());
        assert_eq!(combinator_error.field_name(), Some("age"));
    }

    #[test]
    fn test_field_error_conversion_to_validation() {
        let field_error = FieldError::new(
            Some("age".to_string()),
            ValidationError::new("min_value", "Too small"),
        );

        let validation_error: ValidationError = field_error.into();
        assert_eq!(validation_error.field, Some("age".to_string()));
    }

    #[test]
    fn test_field_into_parts() {
        let validator = named_field("age", MinValue { min: 18 }, get_age);
        let (name, min_value, _accessor) = validator.into_parts();
        assert_eq!(name, Some("age".to_string()));
        assert_eq!(min_value.min, 18);
    }

    #[test]
    fn test_field_map_inner() {
        let error = FieldError::new(Some("age".to_string()), "original error".to_string());

        let mapped = error.map_inner(|s| format!("mapped: {}", s));
        assert_eq!(mapped.inner(), "mapped: original error");
        assert_eq!(mapped.field_name(), Some("age"));
    }

    #[test]
    fn test_field_with_dynamic_name() {
        // Test that String (not &'static str) works
        let field_name = format!("field_{}", 42);
        let validator = named_field(field_name.clone(), MinValue { min: 18 }, get_age);
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
        let validator = named_field("age", MinValue { min: 18 }, get_age);
        let debug_str = format!("{:?}", validator);
        assert!(debug_str.contains("Field"));
        assert!(debug_str.contains("age"));
    }
}