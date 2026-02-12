//! Refined types - values with compile-time guarantees
//!
//! This module provides the `Refined<T, V>` type, which wraps a value
//! and guarantees at compile-time that it has been validated.

use crate::core::{Validate, ValidationError};
use std::marker::PhantomData;
use std::ops::Deref;

// ============================================================================
// REFINED TYPE
// ============================================================================

/// A value that has been validated and carries that guarantee in its type.
///
/// `Refined<T, V>` wraps a value of type `T` and proves that it has been
/// validated by validator `V`. Once created, the value is guaranteed to
/// satisfy the validator's constraints.
///
/// # Type Parameters
///
/// * `T` - The underlying value type
/// * `V` - The validator type (used as a type-level marker)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Refined<T, V> {
    value: T,
    _validator: PhantomData<V>,
}

impl<T, V> Refined<T, V>
where
    V: Validate,
    T: std::borrow::Borrow<V::Input>,
{
    /// Creates a new refined type by validating the value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to validate
    /// * `validator` - The validator to use
    ///
    /// # Errors
    ///
    /// Returns a `ValidationError` if validation fails.
    #[must_use = "validation result must be checked"]
    pub fn new(value: T, validator: &V) -> Result<Self, ValidationError> {
        validator.validate(value.borrow())?;
        Ok(Self {
            value,
            _validator: PhantomData,
        })
    }

    /// Creates a refined type without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the value satisfies the validator's
    /// constraints. Using this with an invalid value will violate the
    /// type system's guarantees.
    pub unsafe fn new_unchecked(value: T) -> Self {
        Self {
            value,
            _validator: PhantomData,
        }
    }
}

impl<T, V> Refined<T, V> {
    /// Extracts the inner value, consuming the refined type.
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Returns a reference to the inner value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Maps the refined value, re-validating the result.
    #[must_use = "mapped value must be used"]
    pub fn try_map<U, F>(self, f: F, validator: &V) -> Result<Refined<U, V>, ValidationError>
    where
        F: FnOnce(T) -> U,
        V: Validate<Input = U>,
    {
        let new_value = f(self.value);
        Refined::new(new_value, validator)
    }

    /// Creates a new refined type with a different validator.
    ///
    /// This is useful when you want to "refine further" with additional
    /// constraints.
    #[must_use = "refined value must be used"]
    pub fn refine<V2>(self, validator: &V2) -> Result<Refined<T, V2>, ValidationError>
    where
        V2: Validate,
        T: std::borrow::Borrow<V2::Input>,
    {
        Refined::new(self.value, validator)
    }

    /// Attempts to create a refined type from a reference.
    ///
    /// This clones the value if validation succeeds.
    #[must_use = "validation result must be checked"]
    pub fn try_from_ref(value: &T, validator: &V) -> Result<Self, ValidationError>
    where
        T: Clone,
        V: Validate<Input = T>,
    {
        validator.validate(value)?;
        Ok(Self {
            value: value.clone(),
            _validator: PhantomData,
        })
    }
}

// ============================================================================
// TRAIT IMPLEMENTATIONS
// ============================================================================

impl<T, V> Deref for Refined<T, V> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, V> AsRef<T> for Refined<T, V> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T, V> std::fmt::Display for Refined<T, V>
where
    T: std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

// Serde support
#[cfg(feature = "serde")]
impl<T, V> serde::Serialize for Refined<T, V>
where
    T: serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.value.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de, T, V> serde::Deserialize<'de> for Refined<T, V>
where
    T: serde::Deserialize<'de>,
    V: Validate<Input = T> + Default,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = T::deserialize(deserializer)?;
        let validator = V::default();
        Refined::new(value, &validator).map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// COMMON REFINED TYPE ALIASES
// ============================================================================

/// A non-empty string.
pub type NonEmptyString<V> = Refined<String, V>;

/// A positive number.
pub type PositiveNumber<T, V> = Refined<T, V>;

/// An email address (validated).
pub type EmailAddress<V> = Refined<String, V>;

/// A URL (validated).
pub type Url<V> = Refined<String, V>;

/// A non-empty vector.
pub type NonEmptyVec<T, V> = Refined<Vec<T>, V>;

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

        fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new("min_length", "Too short"))
            }
        }
    }

    #[test]
    fn test_refined_creation_valid() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello", &validator);
        assert!(refined.is_ok());
    }

    #[test]
    fn test_refined_creation_invalid() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hi", &validator);
        assert!(refined.is_err());
    }

    #[test]
    fn test_refined_deref() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();
        assert_eq!(&*refined, "hello");
        assert_eq!(refined.len(), 5);
    }

    #[test]
    fn test_refined_into_inner() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();
        let inner = refined.into_inner();
        assert_eq!(inner, "hello");
    }

    #[test]
    fn test_refined_type_safety() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();

        fn process_validated(s: Refined<String, MinLength>) -> usize {
            s.len()
        }

        assert_eq!(process_validated(refined), 5);
    }

    #[test]
    fn test_refined_refine() {
        struct MaxLength {
            max: usize,
        }

        impl Validate for MaxLength {
            type Input = str;

            fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
                if input.len() <= self.max {
                    Ok(())
                } else {
                    Err(ValidationError::new("max_length", "Too long"))
                }
            }
        }

        let min_validator = MinLength { min: 5 };
        let max_validator = MaxLength { max: 10 };

        let min_refined = Refined::new("hello".to_string(), &min_validator).unwrap();
        let fully_refined = min_refined.refine(&max_validator);
        assert!(fully_refined.is_ok());
    }
}
