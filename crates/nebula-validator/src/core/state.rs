//! Type-state pattern for validation
//!
//! This module provides compile-time guarantees through the type-state pattern.
//! Values can be in one of two states:
//! - `Unvalidated` - not yet validated
//! - `Validated<V>` - validated by validator V
//!
//! The type system prevents you from using unvalidated values where validated
//! ones are required.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Create unvalidated parameter
//! let param = Parameter::new("hello".to_string());
//!
//! // Validate it - changes type!
//! let validator = MinLength { min: 5 };
//! let validated = param.validate(&validator)?;
//!
//! // Now we can safely unwrap - type guarantees validity
//! let value = validated.unwrap();
//! ```

use crate::core::{Validate, ValidationError};
use std::marker::PhantomData;

// ============================================================================
// STATE MARKERS
// ============================================================================

/// Marker type for unvalidated state.
///
/// Values in this state have not been validated yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unvalidated;

/// Marker type for validated state.
///
/// Values in this state have been validated by validator `V`.
///
/// The `V` type parameter acts as a compile-time proof of validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validated<V> {
    _validator: PhantomData<V>,
}

// ============================================================================
// PARAMETER WITH TYPE-STATE
// ============================================================================

/// A parameter that can be in either validated or unvalidated state.
///
/// The state is tracked at compile-time using phantom types.
///
/// # Type Parameters
///
/// * `T` - The value type
/// * `S` - The state (either `Unvalidated` or `Validated<V>`)
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// // Start with unvalidated parameter
/// let param = Parameter::new("test".to_string());
///
/// // Validate it
/// let validator = MinLength { min: 3 };
/// let validated = param.validate(&validator)?;
///
/// // Type system knows it's validated
/// fn process(p: Parameter<String, Validated<MinLength>>) {
///     // Can safely assume length >= 3
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter<T, S = Unvalidated> {
    value: T,
    _state: PhantomData<S>,
}

// ============================================================================
// UNVALIDATED PARAMETER
// ============================================================================

impl<T> Parameter<T, Unvalidated> {
    /// Creates a new unvalidated parameter.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let param = Parameter::new("hello".to_string());
    /// ```
    pub fn new(value: T) -> Self {
        Self {
            value,
            _state: PhantomData,
        }
    }

    /// Validates the parameter, transitioning to the validated state.
    ///
    /// On success, returns a `Parameter<T, Validated<V>>` which proves
    /// at compile-time that the value has been validated.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to use
    ///
    /// # Errors
    ///
    /// Returns the validator's error if validation fails.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let param = Parameter::new("hello".to_string());
    /// let validator = MinLength { min: 5 };
    /// let validated = param.validate(&validator)?;
    /// ```
    #[must_use = "validation result must be checked"]
    pub fn validate<V>(self, validator: &V) -> Result<Parameter<T, Validated<V>>, ValidationError>
    where
        V: Validate,
        T: std::borrow::Borrow<V::Input>,
    {
        validator.validate(self.value.borrow())?;
        Ok(Parameter {
            value: self.value,
            _state: PhantomData,
        })
    }

    /// Attempts to validate with multiple validators.
    ///
    /// All validators must pass for this to succeed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let param = Parameter::new("hello".to_string());
    /// let validated = param.validate_all(vec![
    ///     &min_validator,
    ///     &max_validator,
    /// ])?;
    /// ```
    pub fn validate_all<V>(
        self,
        validators: Vec<&V>,
    ) -> Result<Parameter<T, Validated<V>>, ValidationError>
    where
        V: Validate,
        T: std::borrow::Borrow<V::Input>,
    {
        for validator in validators {
            validator.validate(self.value.borrow())?;
        }
        Ok(Parameter {
            value: self.value,
            _state: PhantomData,
        })
    }

    /// Validates without changing the state marker.
    ///
    /// Useful when you want to validate but don't need the type-level proof.
    #[must_use = "validation result must be checked"]
    pub fn validate_in_place<V>(&self, validator: &V) -> Result<(), ValidationError>
    where
        V: Validate,
        T: std::borrow::Borrow<V::Input>,
    {
        validator.validate(self.value.borrow())
    }

    /// Skips validation and marks as validated (unsafe).
    ///
    /// # Safety
    ///
    /// The caller must ensure that the value actually satisfies the
    /// validator's constraints. Using this incorrectly violates type safety.
    pub unsafe fn assume_validated<V>(self) -> Parameter<T, Validated<V>> {
        Parameter {
            value: self.value,
            _state: PhantomData,
        }
    }
}

// ============================================================================
// VALIDATED PARAMETER
// ============================================================================

impl<T, V> Parameter<T, Validated<V>> {
    /// Extracts the value from a validated parameter.
    ///
    /// This is safe because the type system guarantees the value is valid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let param = Parameter::new("hello".to_string());
    /// let validated = param.validate(&validator)?;
    /// let value = validated.unwrap(); // Safe!
    /// ```
    pub fn unwrap(self) -> T {
        self.value
    }

    /// Returns a reference to the validated value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Re-validates with a different validator.
    ///
    /// Changes the validation marker to the new validator.
    #[must_use = "validation result must be checked"]
    pub fn revalidate<V2>(
        self,
        validator: &V2,
    ) -> Result<Parameter<T, Validated<V2>>, ValidationError>
    where
        V2: Validate<Input = T>,
    {
        validator.validate(&self.value)?;
        Ok(Parameter {
            value: self.value,
            _state: PhantomData,
        })
    }

    /// Adds additional validation on top of existing validation.
    ///
    /// The result proves that BOTH validators have passed.
    pub fn and_validate<V2>(
        self,
        validator: &V2,
    ) -> Result<Parameter<T, Validated<(V, V2)>>, ValidationError>
    where
        V2: Validate<Input = T>,
    {
        validator.validate(&self.value)?;
        Ok(Parameter {
            value: self.value,
            _state: PhantomData,
        })
    }

    /// Maps the value and re-validates the result.
    pub fn map_and_revalidate<U, F, V2>(
        self,
        f: F,
        validator: &V2,
    ) -> Result<Parameter<U, Validated<V2>>, ValidationError>
    where
        F: FnOnce(T) -> U,
        V2: Validate<Input = U>,
    {
        let new_value = f(self.value);
        validator.validate(&new_value)?;
        Ok(Parameter {
            value: new_value,
            _state: PhantomData,
        })
    }
}

// ============================================================================
// COMMON OPERATIONS (AVAILABLE IN ALL STATES)
// ============================================================================

impl<T, S> Parameter<T, S> {
    /// Returns a reference to the value regardless of validation state.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Clones the value if T is Clone.
    pub fn clone_value(&self) -> T
    where
        T: Clone,
    {
        self.value.clone()
    }

    /// Converts to the inner value, discarding state information.
    pub fn into_value(self) -> T {
        self.value
    }
}

// ============================================================================
// BUILDER PATTERN WITH TYPE-STATE
// ============================================================================

/// A builder for parameters that enforces validation before building.
///
/// # Examples
///
/// ```rust,ignore
/// let param = ParameterBuilder::new()
///     .value("hello".to_string())
///     .validate(&validator)?
///     .build();
/// ```
pub struct ParameterBuilder<T, S = Unvalidated> {
    value: Option<T>,
    _state: PhantomData<S>,
}

impl<T> ParameterBuilder<T, Unvalidated> {
    /// Creates a new parameter builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: None,
            _state: PhantomData,
        }
    }

    /// Sets the value.
    #[must_use = "builder methods must be chained or built"]
    pub fn value(mut self, value: T) -> Self {
        self.value = Some(value);
        self
    }

    /// Validates the value.
    #[must_use = "validation result must be checked"]
    pub fn validate<V>(
        self,
        validator: &V,
    ) -> Result<ParameterBuilder<T, Validated<V>>, ValidationError>
    where
        V: Validate<Input = T>,
    {
        let value = self.value.expect("Value must be set before validation");
        validator.validate(&value)?;
        Ok(ParameterBuilder {
            value: Some(value),
            _state: PhantomData,
        })
    }
}

impl<T, V> ParameterBuilder<T, Validated<V>> {
    /// Builds the parameter (only available after validation).
    pub fn build(self) -> Parameter<T, Validated<V>> {
        Parameter {
            value: self.value.expect("Value must be set"),
            _state: PhantomData,
        }
    }
}

impl<T> Default for ParameterBuilder<T, Unvalidated> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// TRAIT IMPLEMENTATIONS
// ============================================================================

impl<T, S> AsRef<T> for Parameter<T, S> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T, S> std::ops::Deref for Parameter<T, S> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, S> std::fmt::Display for Parameter<T, S>
where
    T: std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ValidationError;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = String;

        fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new("min_length", "Too short"))
            }
        }
    }

    #[test]
    fn test_unvalidated_parameter() {
        let param = Parameter::new("hello".to_string());
        assert_eq!(param.value(), "hello");
    }

    #[test]
    fn test_validation_success() {
        let param = Parameter::new("hello".to_string());
        let validator = MinLength { min: 5 };
        let validated = param.validate(&validator);
        assert!(validated.is_ok());
    }

    #[test]
    fn test_validation_failure() {
        let param = Parameter::new("hi".to_string());
        let validator = MinLength { min: 5 };
        let validated = param.validate(&validator);
        assert!(validated.is_err());
    }

    #[test]
    fn test_validated_unwrap() {
        let param = Parameter::new("hello".to_string());
        let validator = MinLength { min: 5 };
        let validated = param.validate(&validator).unwrap();
        let value = validated.unwrap();
        assert_eq!(value, "hello");
    }

    #[test]
    fn test_type_state_safety() {
        let param = Parameter::new("hello".to_string());
        let validator = MinLength { min: 5 };
        let validated = param.validate(&validator).unwrap();

        // This function only accepts validated parameters
        fn process_validated(p: Parameter<String, Validated<MinLength>>) -> usize {
            p.unwrap().len()
        }

        assert_eq!(process_validated(validated), 5);
    }

    #[test]
    fn test_builder_pattern() {
        let validator = MinLength { min: 5 };
        let param = ParameterBuilder::new()
            .value("hello".to_string())
            .validate(&validator)
            .unwrap()
            .build();

        assert_eq!(param.unwrap(), "hello");
    }

    #[test]
    fn test_revalidate() {
        struct MaxLength {
            max: usize,
        }

        impl Validate for MaxLength {
            type Input = String;

            fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
                if input.len() <= self.max {
                    Ok(())
                } else {
                    Err(ValidationError::new("max_length", "Too long"))
                }
            }
        }

        let param = Parameter::new("hello".to_string());
        let min_validator = MinLength { min: 3 };
        let max_validator = MaxLength { max: 10 };

        let validated = param.validate(&min_validator).unwrap();
        let revalidated = validated.revalidate(&max_validator);
        assert!(revalidated.is_ok());
    }
}
