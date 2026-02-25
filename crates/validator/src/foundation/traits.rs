//! Core validation traits
//!
//! This module provides the fundamental traits for type-safe validation:
//!
//! - [`Validate<T>`] - Core trait for validators, generic over input type
//! - [`Validatable`] - Extension trait enabling `value.validate(&validator)` syntax
//!
//! # Design Philosophy
//!
//! The validation system uses Rust's trait bounds to ensure type safety at compile time:
//!
//! - String validators use `AsRef<str>` bounds → work with `&str`, `String`, `Cow<str>`
//! - Numeric validators use `Ord`/`PartialOrd` bounds → work with any comparable type
//! - Collection validators use `AsRef<[T]>` bounds → work with `Vec<T>`, `&[T]`, arrays
//!
//! # Examples
//!
//! ## Extension Method Style
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // String validation
//! "hello@example.com".validate(&email())?;
//!
//! // Numeric validation
//! 42.validate(&min(10))?;
//!
//! // Collection validation
//! vec![1, 2, 3].validate(&min_size(2))?;
//! ```
//!
//! ## Direct Validator Style
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! let validator = min_length(3).and(max_length(20));
//! validator.validate("hello")?;
//! ```

use crate::foundation::ValidationError;
use crate::foundation::validatable::AsValidatable;
use std::borrow::Borrow;

// ============================================================================
// CORE VALIDATOR TRAIT
// ============================================================================

/// Core validation trait, generic over input type.
///
/// Validators implement this trait for specific input types, using trait bounds
/// to ensure type safety. The generic parameter `T` determines what types
/// the validator can accept.
///
/// # Type Safety Through Bounds
///
/// ```rust,ignore
/// // String validator - accepts any AsRef<str>
/// impl<T: AsRef<str> + ?Sized> Validate<T> for MinLength { ... }
///
/// // Numeric validator - accepts any Ord type
/// impl<T: Ord> Validate<T> for Min<T> { ... }
///
/// // This ensures compile-time type safety:
/// "hello".validate(&min_length(3));  // ✓ Compiles
/// 42.validate(&min_length(3));       // ✗ Compile error: i32 doesn't impl AsRef<str>
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::foundation::{Validate, ValidationError};
///
/// struct IsPositive;
///
/// impl<T: Ord + Default> Validate<T> for IsPositive {
///     fn validate(&self, input: &T) -> Result<(), ValidationError> {
///         if input > &T::default() {
///             Ok(())
///         } else {
///             Err(ValidationError::new("positive", "Must be positive"))
///         }
///     }
/// }
///
/// // Works with any Ord + Default type
/// assert!(IsPositive.validate(&42i32).is_ok());
/// assert!(IsPositive.validate(&3.14f64).is_ok());
/// ```
pub trait Validate<T: ?Sized> {
    /// Validates the input value.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if validation succeeds
    /// - `Err(ValidationError)` if validation fails
    fn validate(&self, input: &T) -> Result<(), ValidationError>;

    /// Validates a value that can be converted to the target type via [`AsValidatable`].
    ///
    /// This bridges typed validators with dynamically-typed inputs such as
    /// `serde_json::Value`. The input is first converted through `AsValidatable`,
    /// then validated against the target type.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::prelude::*;
    /// use serde_json::json;
    ///
    /// let v = min_length(3);
    /// assert!(v.validate_any(&json!("hello")).is_ok());
    /// assert!(v.validate_any(&json!("hi")).is_err());
    /// ```
    fn validate_any<U>(&self, input: &U) -> Result<(), ValidationError>
    where
        U: AsValidatable<T>,
        Self: Sized,
    {
        let converted = input.as_validatable()?;
        self.validate(converted.borrow())
    }
}

// ============================================================================
// VALIDATABLE EXTENSION TRAIT
// ============================================================================

/// Extension trait enabling `value.validate_with(&validator)` syntax.
///
/// This trait is automatically implemented for ALL types through a blanket
/// implementation, providing a fluent API for validation.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// // Extension method style - read left to right
/// let result = "hello".validate_with(&min_length(3));
///
/// // Chain multiple validations
/// "hello"
///     .validate_with(&min_length(3))?
///     .validate_with(&max_length(20))?;
///
/// // Works with any type
/// 42.validate_with(&min(10))?;
/// vec![1, 2, 3].validate_with(&not_empty())?;
/// true.validate_with(&is_true())?;
/// ```
pub trait Validatable {
    /// Validates this value using the given validator.
    ///
    /// Returns `Ok(&Self)` on success for method chaining.
    fn validate_with<V>(&self, validator: &V) -> Result<&Self, ValidationError>
    where
        V: Validate<Self>;
}

/// Blanket implementation - ALL types get the `.validate_with()` method for free.
impl<T: ?Sized> Validatable for T {
    #[inline]
    fn validate_with<V>(&self, validator: &V) -> Result<&Self, ValidationError>
    where
        V: Validate<Self>,
    {
        validator.validate(self)?;
        Ok(self)
    }
}

// ============================================================================
// VALIDATOR EXTENSION TRAIT
// ============================================================================

/// Extension trait for composing validators.
///
/// Provides combinator methods (`.and()`, `.or()`, `.not()`) for building
/// complex validation logic from simple validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// // Compose validators fluently
/// let username = min_length(3)
///     .and(max_length(20))
///     .and(alphanumeric());
///
/// // Use OR for alternatives
/// let id = uuid().or(email());
///
/// // Use NOT for negation
/// let not_empty = min_length(1).not().not(); // Double negation = original
/// ```
pub trait ValidateExt<T: ?Sized>: Validate<T> + Sized {
    /// Combines two validators with logical AND.
    ///
    /// Both validators must pass. Short-circuits on first failure.
    fn and<V: Validate<T>>(self, other: V) -> And<Self, V> {
        And::new(self, other)
    }

    /// Combines two validators with logical OR.
    ///
    /// At least one validator must pass. Short-circuits on first success.
    fn or<V: Validate<T>>(self, other: V) -> Or<Self, V> {
        Or::new(self, other)
    }

    /// Inverts the validator with logical NOT.
    fn not(self) -> Not<Self> {
        Not::new(self)
    }

    /// Makes validation conditional.
    ///
    /// Validation only runs if the condition returns `true`.
    fn when<C: Fn(&T) -> bool>(self, condition: C) -> When<Self, C> {
        When::new(self, condition)
    }
}

/// Blanket implementation - all validators get combinator methods.
impl<T: ?Sized, V: Validate<T>> ValidateExt<T> for V {}

// ============================================================================
// COMBINATOR TYPES
// ============================================================================

/// AND combinator - both validators must pass.
#[derive(Debug, Clone, Copy)]
pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> And<L, R> {
    pub const fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

impl<T: ?Sized, L: Validate<T>, R: Validate<T>> Validate<T> for And<L, R> {
    #[inline]
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        self.left.validate(input)?;
        self.right.validate(input)
    }
}

/// OR combinator - at least one validator must pass.
#[derive(Debug, Clone, Copy)]
pub struct Or<L, R> {
    left: L,
    right: R,
}

impl<L, R> Or<L, R> {
    pub const fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

impl<T: ?Sized, L: Validate<T>, R: Validate<T>> Validate<T> for Or<L, R> {
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        match self.left.validate(input) {
            Ok(()) => Ok(()),
            Err(left_err) => match self.right.validate(input) {
                Ok(()) => Ok(()),
                Err(right_err) => Err(ValidationError::new(
                    "or_failed",
                    "All alternatives failed validation",
                )
                .with_nested_error(left_err)
                .with_nested_error(right_err)),
            },
        }
    }
}

/// NOT combinator - inverts validation result.
#[derive(Debug, Clone, Copy)]
pub struct Not<V> {
    inner: V,
}

impl<V> Not<V> {
    pub const fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<T: ?Sized, V: Validate<T>> Validate<T> for Not<V> {
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        match self.inner.validate(input) {
            Ok(()) => Err(ValidationError::new(
                "not_failed",
                "Validation unexpectedly passed",
            )),
            Err(_) => Ok(()),
        }
    }
}

/// Conditional validator - only runs when condition is true.
#[derive(Debug, Clone, Copy)]
pub struct When<V, C> {
    validator: V,
    condition: C,
}

impl<V, C> When<V, C> {
    pub const fn new(validator: V, condition: C) -> Self {
        Self {
            validator,
            condition,
        }
    }
}

impl<T: ?Sized, V: Validate<T>, C: Fn(&T) -> bool> Validate<T> for When<V, C> {
    #[inline]
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        if (self.condition)(input) {
            self.validator.validate(input)
        } else {
            Ok(())
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Test validator using Ord bound
    struct Min<N>(N);

    impl<T: Ord> Validate<T> for Min<T> {
        fn validate(&self, input: &T) -> Result<(), ValidationError> {
            if input >= &self.0 {
                Ok(())
            } else {
                Err(ValidationError::new("min", "Value too small"))
            }
        }
    }

    // Test validator using AsRef<str> bound
    struct MinLen(usize);

    impl<T: AsRef<str> + ?Sized> Validate<T> for MinLen {
        fn validate(&self, input: &T) -> Result<(), ValidationError> {
            if input.as_ref().len() >= self.0 {
                Ok(())
            } else {
                Err(ValidationError::new("min_length", "String too short"))
            }
        }
    }

    #[test]
    fn test_extension_method_string() {
        // Extension method style
        assert!("hello".validate_with(&MinLen(3)).is_ok());
        assert!("hi".validate_with(&MinLen(3)).is_err());

        // Works with String too
        let s = String::from("hello");
        assert!(s.validate_with(&MinLen(3)).is_ok());
    }

    #[test]
    fn test_extension_method_numeric() {
        // Works with any Ord type
        assert!(42i32.validate_with(&Min(10)).is_ok());
        assert!(5i32.validate_with(&Min(10)).is_err());

        assert!(42i64.validate_with(&Min(10i64)).is_ok());
        assert!(42u8.validate_with(&Min(10u8)).is_ok());
    }

    #[test]
    fn test_direct_validator_style() {
        let validator = MinLen(3);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_method_chaining() {
        // Chain multiple validations
        let result = "hello"
            .validate_with(&MinLen(3))
            .and_then(|s| s.validate_with(&MinLen(1)));
        assert!(result.is_ok());
    }

    #[test]
    fn test_and_combinator() {
        let validator = <MinLen as ValidateExt<str>>::and(MinLen(3), MinLen(1));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_or_combinator() {
        let len_3_or_5 = <MinLen as ValidateExt<str>>::or(MinLen(5), MinLen(3));
        assert!(len_3_or_5.validate("hello").is_ok()); // len 5
        assert!(len_3_or_5.validate("hey").is_ok()); // len 3
        assert!(len_3_or_5.validate("hi").is_err()); // len 2
    }

    #[test]
    fn test_not_combinator() {
        let not_min_3 = <MinLen as ValidateExt<str>>::not(MinLen(3));
        assert!(not_min_3.validate("hi").is_ok()); // fails min_3, so NOT passes
        assert!(not_min_3.validate("hello").is_err()); // passes min_3, so NOT fails
    }

    #[test]
    fn test_type_safety() {
        // MinLen only works with AsRef<str> types
        // Min<T> only works with Ord types
        let _ = "hello".validate_with(&MinLen(3));
        let _ = String::from("hello").validate_with(&MinLen(3));
        let _ = 42i32.validate_with(&Min(10i32));
    }
}
