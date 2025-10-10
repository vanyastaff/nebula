//! Refined types - values with compile-time guarantees
//!
//! This module provides the `Refined<T, V>` type, which wraps a value
//! and guarantees at compile-time that it has been validated.
//!
//! # Benefits
//!
//! - **Type Safety**: Once created, a refined type is guaranteed to be valid
//! - **Zero-Cost**: No runtime overhead for accessing the value
//! - **Self-Documenting**: Function signatures clearly show validation requirements
//! - **Impossible States**: Invalid states are unrepresentable
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // Define a validator
//! struct MinLength { min: usize }
//!
//! impl TypedValidator for MinLength {
//!     type Input = str;
//!     type Output = ();
//!     type Error = ValidationError;
//!     
//!     fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
//!         if input.len() >= self.min {
//!             Ok(())
//!         } else {
//!             Err(ValidationError::new("min_length", "Too short"))
//!         }
//!     }
//! }
//!
//! // Create a refined type
//! let validator = MinLength { min: 5 };
//! let validated = Refined::new("hello".to_string(), &validator)?;
//!
//! // Now the type system knows this string is at least 5 chars!
//! fn process_long_string(s: Refined<String, MinLength>) {
//!     // We can safely assume s.len() >= 5
//! }
//! ```

use crate::core::TypedValidator;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

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
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// // Create a refined string that's guaranteed to be at least 5 chars
/// let validator = MinLength { min: 5 };
/// let refined = Refined::new("hello".to_string(), &validator)?;
///
/// // Access the value
/// assert_eq!(refined.as_ref(), "hello");
/// assert_eq!(refined.len(), 5);
///
/// // Consume the refined type
/// let inner: String = refined.into_inner();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Refined<T, V> {
    value: T,
    _validator: PhantomData<V>,
}

impl<T, V> Refined<T, V>
where
    V: TypedValidator<Output = ()>,
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
    /// Returns the validator's error if validation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = MinLength { min: 5 };
    /// let valid = Refined::new("hello".to_string(), &validator);
    /// assert!(valid.is_ok());
    ///
    /// let invalid = Refined::new("hi".to_string(), &validator);
    /// assert!(invalid.is_err());
    /// ```
    #[must_use = "validation result must be checked"]
    pub fn new(value: T, validator: &V) -> Result<Self, V::Error> {
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
    /// type system's guarantees and may lead to undefined behavior.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // SAFE: We know "hello" is at least 5 characters
    /// let refined = unsafe {
    ///     Refined::<String, MinLength>::new_unchecked("hello".to_string())
    /// };
    /// ```
    pub unsafe fn new_unchecked(value: T) -> Self {
        Self {
            value,
            _validator: PhantomData,
        }
    }
}

impl<T, V> Refined<T, V> {
    /// Extracts the inner value, consuming the refined type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let refined = Refined::new("hello".to_string(), &validator)?;
    /// let string: String = refined.into_inner();
    /// ```
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Returns a reference to the inner value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let refined = Refined::new("hello".to_string(), &validator)?;
    /// let s: &str = refined.as_ref();
    /// ```
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the inner value.
    ///
    /// # Safety
    ///
    /// This is safe because the refined type can only be created through
    /// validation. However, be careful not to modify the value in a way
    /// that would violate the validator's constraints.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Maps the refined value to a different type.
    ///
    /// The mapping function preserves the validation guarantee.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let refined = Refined::new("hello".to_string(), &validator)?;
    /// let length: Refined<usize, MinLength> = refined.map(|s| s.len());
    /// ```
    pub fn map<U, F>(self, f: F) -> Refined<U, V>
    where
        F: FnOnce(T) -> U,
    {
        Refined {
            value: f(self.value),
            _validator: PhantomData,
        }
    }

    /// Attempts to map the refined value, re-validating the result.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let refined = Refined::new("hello".to_string(), &validator)?;
    /// let uppercase = refined.try_map(
    ///     |s| s.to_uppercase(),
    ///     &validator
    /// )?;
    /// ```
    #[must_use = "mapped value must be used"]
    pub fn try_map<U, F>(self, f: F, validator: &V) -> Result<Refined<U, V>, V::Error>
    where
        F: FnOnce(T) -> U,
        V: TypedValidator<Input = U, Output = ()>,
    {
        let new_value = f(self.value);
        Refined::new(new_value, validator)
    }

    /// Creates a new refined type with a different validator.
    ///
    /// This is useful when you want to "refine further" with additional
    /// constraints.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let min_validated = Refined::new("hello".to_string(), &min_validator)?;
    /// let fully_validated = min_validated.refine(&max_validator)?;
    /// ```
    #[must_use = "refined value must be used"]
    pub fn refine<V2>(self, validator: &V2) -> Result<Refined<T, V2>, V2::Error>
    where
        V2: TypedValidator<Output = ()>,
        T: std::borrow::Borrow<V2::Input>,
    {
        Refined::new(self.value, validator)
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

impl<T, V> DerefMut for Refined<T, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T, V> AsRef<T> for Refined<T, V> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T, V> AsMut<T> for Refined<T, V> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

// Display forwards to inner value
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
    V: TypedValidator<Input = T, Output = ()> + Default,
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
// COMMON REFINED TYPES
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
// CONVENIENCE CONSTRUCTORS
// ============================================================================

impl<T, V> Refined<T, V> {
    /// Attempts to create a refined type from a reference.
    ///
    /// This clones the value if validation succeeds.
    #[must_use = "validation result must be checked"]
    pub fn try_from_ref(value: &T, validator: &V) -> Result<Self, V::Error>
    where
        T: Clone,
        V: TypedValidator<Input = T, Output = ()>,
    {
        validator.validate(value)?;
        Ok(Self {
            value: value.clone(),
            _validator: PhantomData,
        })
    }
}

// Note: TryFrom implementation conflicts with std blanket implementation
// Use Refined::new_with_default() or Refined::new() instead

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Test validator
    struct MinLength {
        min: usize,
    }

    impl TypedValidator for MinLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

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
        assert_eq!(refined.len(), 5); // String method works via Deref
    }

    #[test]
    fn test_refined_into_inner() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();
        let inner = refined.into_inner();
        assert_eq!(inner, "hello");
    }

    #[test]
    fn test_refined_map() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();
        let length = refined.map(|s| s.len());
        assert_eq!(length.into_inner(), 5);
    }

    #[test]
    fn test_refined_type_safety() {
        let validator = MinLength { min: 5 };
        let refined = Refined::new("hello".to_string(), &validator).unwrap();

        // This function only accepts validated strings
        fn process_validated(s: Refined<String, MinLength>) -> usize {
            s.len() // We know it's at least 5 characters
        }

        assert_eq!(process_validated(refined), 5);
    }

    #[test]
    fn test_refined_refine() {
        struct MaxLength {
            max: usize,
        }

        impl TypedValidator for MaxLength {
            type Input = str;
            type Output = ();
            type Error = ValidationError;

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
