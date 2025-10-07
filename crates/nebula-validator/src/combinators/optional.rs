//! OPTIONAL combinator - validates Option types
//!
//! The OPTIONAL combinator makes a validator work with Option<T>.
//! It succeeds if the value is None, or if it's Some and validation passes.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = MinLength { min: 5 }.optional();
//!
//! assert!(validator.validate(&None).is_ok());           // None is valid
//! assert!(validator.validate(&Some("hello")).is_ok());  // Some valid string
//! assert!(validator.validate(&Some("hi")).is_err());    // Some invalid string
//! ```

use crate::core::{TypedValidator, ValidatorMetadata};

// ============================================================================
// OPTIONAL COMBINATOR
// ============================================================================

/// Makes a validator work with Option types.
///
/// The validator succeeds if:
/// - The value is `None`, OR
/// - The value is `Some(x)` and `x` passes validation
///
/// # Type Parameters
///
/// * `V` - Inner validator type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = MinLength { min: 3 }.optional();
///
/// assert!(validator.validate(&None).is_ok());
/// assert!(validator.validate(&Some("hello")).is_ok());
/// assert!(validator.validate(&Some("hi")).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Optional<V> {
    pub(crate) inner: V,
}

impl<V> Optional<V> {
    /// Creates a new OPTIONAL combinator.
    ///
    /// # Arguments
    ///
    /// * `inner` - The validator to make optional
    pub fn new(inner: V) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner validator.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Extracts the inner validator.
    pub fn into_inner(self) -> V {
        self.inner
    }
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<V, T> TypedValidator for Optional<V>
where
    V: TypedValidator<Input = T>,
{
    type Input = Option<T>;
    type Output = ();
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            None => Ok(()), // None is always valid
            Some(value) => {
                self.inner.validate(value)?;
                Ok(())
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.inner.metadata();

        ValidatorMetadata {
            name: format!("Optional({})", inner_meta.name),
            description: Some(format!("Optional {}", inner_meta.name)),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("optional".to_string());
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
impl<V, T> crate::core::AsyncValidator for Optional<V>
where
    V: TypedValidator<Input = T> + crate::core::AsyncValidator<
        Input = T,
        Output = <V as TypedValidator>::Output,
        Error = <V as TypedValidator>::Error
    > + Send + Sync,
    T: Sync,
{
    type Input = Option<T>;
    type Output = ();
    type Error = <V as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            None => Ok(()),
            Some(value) => {
                self.inner.validate_async(value).await?;
                Ok(())
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates an OPTIONAL combinator from a validator.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::optional;
///
/// let validator = optional(min_length(5));
/// assert!(validator.validate(&None).is_ok());
/// ```
pub fn optional<V>(validator: V) -> Optional<V>
where
    V: TypedValidator,
{
    Optional::new(validator)
}

// ============================================================================
// REQUIRED COMBINATOR (opposite of OPTIONAL)
// ============================================================================

/// Validates that Option is Some and the inner value passes validation.
///
/// This is stricter than Optional - it fails on None.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::required_some;
///
/// let validator = required_some(min_length(5));
/// assert!(validator.validate(&None).is_err());         // None fails
/// assert!(validator.validate(&Some("hello")).is_ok()); // Valid Some
/// ```
pub fn required_some<V>(validator: V) -> RequiredSome<V>
where
    V: TypedValidator,
{
    RequiredSome::new(validator)
}

/// Validator that requires Option to be Some.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredSome<V> {
    inner: V,
}

impl<V> RequiredSome<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &V {
        &self.inner
    }

    pub fn into_inner(self) -> V {
        self.inner
    }
}

impl<V, T> TypedValidator for RequiredSome<V>
where
    V: TypedValidator<Input = T>,
{
    type Input = Option<T>;
    type Output = ();
    type Error = RequiredError<V::Error>;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            None => Err(RequiredError::NoneValue),
            Some(value) => {
                self.inner
                    .validate(value)
                    .map(|_| ())
                    .map_err(RequiredError::ValidationFailed)
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.inner.metadata();
        ValidatorMetadata {
            name: format!("RequiredSome({})", inner_meta.name),
            description: Some(format!("Required {}", inner_meta.name)),
            ..inner_meta
        }
    }
}

/// Error type for RequiredSome validator.
#[derive(Debug, Clone)]
pub enum RequiredError<E> {
    /// Value was None when Some was required.
    NoneValue,
    /// Validation of the inner value failed.
    ValidationFailed(E),
}

impl<E: std::fmt::Display> std::fmt::Display for RequiredError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequiredError::NoneValue => write!(f, "Value is required but was None"),
            RequiredError::ValidationFailed(e) => write!(f, "Validation failed: {}", e),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for RequiredError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RequiredError::ValidationFailed(e) => Some(e),
            _ => None,
        }
    }
}

// ============================================================================
// NULLABLE VALIDATOR
// ============================================================================

/// Validates nullable values - accepts None or valid Some.
///
/// This is an alias for Optional for better semantics.
pub type Nullable<V> = Optional<V>;

/// Creates a nullable validator.
pub fn nullable<V>(validator: V) -> Nullable<V>
where
    V: TypedValidator,
{
    Optional::new(validator)
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{TypedValidator, ValidationError, ValidatorExt};

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
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_optional_none() {
        let validator = Optional::new(MinLength { min: 5 });
        let value: Option<&str> = None;
        assert!(validator.validate(&value).is_ok());
    }

    #[test]
    fn test_optional_some_valid() {
        let validator = Optional::new(MinLength { min: 5 });
        let value: Option<&str> = Some("hello");
        assert!(validator.validate(&value).is_ok());
    }

    #[test]
    fn test_optional_some_invalid() {
        let validator = Optional::new(MinLength { min: 5 });
        let value: Option<&str> = Some("hi");
        assert!(validator.validate(&value).is_err());
    }

    #[test]
    fn test_optional_helper() {
        let validator = optional(MinLength { min: 5 });
        let none: Option<&str> = None;
        let valid: Option<&str> = Some("hello");
        let invalid: Option<&str> = Some("hi");

        assert!(validator.validate(&none).is_ok());
        assert!(validator.validate(&valid).is_ok());
        assert!(validator.validate(&invalid).is_err());
    }

    #[test]
    fn test_required_some_none() {
        let validator = required_some(MinLength { min: 5 });
        let result = validator.validate(&None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RequiredError::NoneValue));
    }

    #[test]
    fn test_required_some_valid() {
        let validator = required_some(MinLength { min: 5 });
        assert!(validator.validate(&Some("hello")).is_ok());
    }

    #[test]
    fn test_required_some_invalid() {
        let validator = required_some(MinLength { min: 5 });
        let result = validator.validate(&Some("hi"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RequiredError::ValidationFailed(_)
        ));
    }

    #[test]
    fn test_nullable_alias() {
        let validator: Nullable<MinLength> = nullable(MinLength { min: 5 });
        assert!(validator.validate(&None).is_ok());
        assert!(validator.validate(&Some("hello")).is_ok());
    }

    #[test]
    fn test_optional_metadata() {
        let validator = Optional::new(MinLength { min: 5 });
        let meta = validator.metadata();

        assert!(meta.name.contains("Optional"));
        assert!(meta.tags.contains(&"optional".to_string()));
    }

    #[test]
    fn test_into_inner() {
        let min_length = MinLength { min: 5 };
        let validator = Optional::new(min_length);
        let extracted = validator.into_inner();

        assert_eq!(extracted.min, 5);
    }

    #[test]
    fn test_required_error_display() {
        let error = RequiredError::<ValidationError>::NoneValue;
        assert!(error.to_string().contains("required"));

        let validation_error = ValidationError::new("test", "Test error");
        let error = RequiredError::ValidationFailed(validation_error);
        assert!(error.to_string().contains("Validation failed"));
    }

    // TODO: Fix this test - requires fixing Optional to work with ?Sized types
    // #[test]
    // fn test_optional_with_complex_validator() {
    //     struct MaxLength {
    //         max: usize,
    //     }
    //
    //     impl TypedValidator for MaxLength {
    //         type Input = str;
    //         type Output = ();
    //         type Error = ValidationError;
    //
    //         fn validate(&self, input: &str) -> Result<(), ValidationError> {
    //             if input.len() <= self.max {
    //                 Ok(())
    //             } else {
    //                 Err(ValidationError::max_length("", self.max, input.len()))
    //             }
    //         }
    //     }
    //
    //     let validator = MinLength { min: 5 }.and(MaxLength { max: 10 }).optional();
    //
    //     let none_value: Option<&str> = None;
    //     assert!(validator.validate(&none_value).is_ok());
    //
    //     let some_value: Option<&str> = Some("hello");
    //     assert!(validator.validate(&some_value).is_ok());
    //
    //     let short_value: Option<&str> = Some("hi");
    //     assert!(validator.validate(&short_value).is_err());
    //
    //     let long_value: Option<&str> = Some("verylongstring");
    //     assert!(validator.validate(&long_value).is_err());
    // }
}