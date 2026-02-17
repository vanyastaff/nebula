//! Core traits for the validation system
//!
//! This module defines the fundamental traits that all validators must implement.

use crate::foundation::validatable::AsValidatable;
use std::borrow::Borrow;

// ============================================================================
// CORE VALIDATOR TRAIT
// ============================================================================

/// The core trait that all validators must implement.
///
/// This trait is generic over the input type, allowing for compile-time
/// type safety while maintaining flexibility. All validators return
/// `Result<(), ValidationError>` for a consistent API.
///
/// # Type Parameters
///
/// * `Input` - The type being validated (can be `?Sized` for DSTs like `str`)
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::foundation::{Validate, ValidationError};
///
/// struct MinLength {
///     min: usize,
/// }
///
/// impl Validate for MinLength {
///     type Input = str;
///
///     fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
///         if input.len() >= self.min {
///             Ok(())
///         } else {
///             Err(ValidationError::new(
///                 "min_length",
///                 format!("Must be at least {} characters", self.min),
///             ))
///         }
///     }
/// }
/// ```
pub trait Validate {
    /// The type of input being validated.
    ///
    /// Use `?Sized` to allow validation of unsized types like `str` and `[T]`.
    type Input: ?Sized;

    /// Validates the input value.
    ///
    /// # Arguments
    ///
    /// * `input` - The value to validate
    ///
    /// # Returns
    ///
    /// * `Ok(())` if validation succeeds
    /// * `Err(ValidationError)` if validation fails
    fn validate(&self, input: &Self::Input) -> Result<(), crate::foundation::ValidationError>;

    /// Validates any type that can be converted to `Self::Input`.
    ///
    /// This method enables universal validation - a single validator can accept
    /// multiple input types (e.g., `&str`, `String`, `Cow<str>`) without explicit
    /// conversion by the caller.
    ///
    /// # Type Parameters
    ///
    /// * `S` - Any type that implements `AsValidatable<Self::Input>`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::foundation::{Validate, ValidationError};
    ///
    /// struct MinLength { min: usize }
    ///
    /// impl Validate for MinLength {
    ///     type Input = str;
    ///
    ///     fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
    ///         if input.len() >= self.min {
    ///             Ok(())
    ///         } else {
    ///             Err(ValidationError::new("min_length", "too short"))
    ///         }
    ///     }
    /// }
    ///
    /// let validator = MinLength { min: 3 };
    ///
    /// // Works with &str
    /// assert!(validator.validate_any("hello").is_ok());
    ///
    /// // Works with String
    /// assert!(validator.validate_any(&String::from("hello")).is_ok());
    ///
    /// // Works with Cow<str>
    /// use std::borrow::Cow;
    /// assert!(validator.validate_any(&Cow::Borrowed("hello")).is_ok());
    /// ```
    fn validate_any<S>(&self, value: &S) -> Result<(), crate::foundation::ValidationError>
    where
        Self: Sized,
        S: AsValidatable<Self::Input> + ?Sized,
        for<'a> <S as AsValidatable<Self::Input>>::Output<'a>: Borrow<Self::Input>,
    {
        let output = value.as_validatable()?;
        self.validate(output.borrow())
    }
}

// ============================================================================
// VALIDATOR EXTENSION TRAIT
// ============================================================================

/// Extension trait providing combinator methods for validators.
///
/// This trait is automatically implemented for all types that implement
/// `Validate`, providing a fluent API for composing validators.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::prelude::*;
///
/// let validator = MinLength { min: 5 }
///     .and(MaxLength { max: 20 })
///     .and(AlphanumericOnly);
///
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// ```
pub trait ValidateExt: Validate + Sized {
    /// Combines two validators with logical AND.
    ///
    /// Both validators must pass for the combined validator to succeed.
    /// Short-circuits on the first failure.
    ///
    /// # Type Constraints
    ///
    /// The other validator must validate the same input type and return
    /// the same error type.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = MinLength { min: 3 }.and(MaxLength { max: 10 });
    /// assert!(validator.validate("hello").is_ok());
    /// assert!(validator.validate("hi").is_err()); // too short
    /// assert!(validator.validate("verylongstring").is_err()); // too long
    /// ```
    fn and<V>(self, other: V) -> And<Self, V>
    where
        V: Validate<Input = Self::Input>,
    {
        And::new(self, other)
    }

    /// Combines two validators with logical OR.
    ///
    /// At least one validator must pass for the combined validator to succeed.
    /// Short-circuits on the first success.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = ExactLength { length: 5 }.or(ExactLength { length: 10 });
    /// assert!(validator.validate("hello").is_ok()); // length 5
    /// assert!(validator.validate("helloworld").is_ok()); // length 10
    /// assert!(validator.validate("hi").is_err()); // neither 5 nor 10
    /// ```
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: Validate<Input = Self::Input>,
    {
        Or::new(self, other)
    }

    /// Inverts the validator with logical NOT.
    ///
    /// The combined validator succeeds if the original validator fails,
    /// and vice versa.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = Contains { substring: "test" }.not();
    /// assert!(validator.validate("hello world").is_ok()); // doesn't contain "test"
    /// assert!(validator.validate("test string").is_err()); // contains "test"
    /// ```
    fn not(self) -> Not<Self> {
        Not::new(self)
    }

    /// Makes validation conditional based on a predicate.
    ///
    /// The validator only runs if the condition returns `true`.
    /// If the condition returns `false`, validation is skipped.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = MinLength { min: 10 }.when(|s: &&str| s.starts_with("long"));
    /// assert!(validator.validate("longstring123").is_ok()); // checked, passes
    /// assert!(validator.validate("short").is_ok()); // not checked, skipped
    /// ```
    fn when<C>(self, condition: C) -> When<Self, C>
    where
        C: Fn(&Self::Input) -> bool,
    {
        When::new(self, condition)
    }

    /// Makes a validator optional.
    ///
    /// The validator succeeds if the input is `None` or if validation passes.
    fn optional(self) -> Optional<Self> {
        Optional::new(self)
    }

    /// Adds caching to the validator with default capacity (1000 entries).
    ///
    /// Results are cached based on the input value's hash using LRU eviction.
    /// Use with caution for validators with side effects.
    ///
    /// # Requirements
    ///
    /// - Input must be `Hash` and `Eq`
    /// - Output and Error must be `Clone`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let validator = expensive_validator().cached();
    /// let stats = validator.cache_stats();
    /// println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);
    /// ```
    #[cfg(feature = "caching")]
    fn cached(self) -> Cached<Self>
    where
        Self::Input: std::hash::Hash,
    {
        Cached::new(self)
    }

    /// Adds caching to the validator with custom capacity.
    ///
    /// Results are cached based on the input value's hash using LRU eviction.
    /// When the cache reaches capacity, the least recently used entry is evicted.
    ///
    /// # Requirements
    ///
    /// - Input must be `Hash` and `Eq`
    /// - Output and Error must be `Clone`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Small cache for memory-constrained environments
    /// let validator = expensive_validator().cached_with_capacity(100);
    /// ```
    #[cfg(feature = "caching")]
    fn cached_with_capacity(self, capacity: usize) -> Cached<Self>
    where
        Self::Input: std::hash::Hash,
    {
        Cached::with_capacity(self, capacity)
    }
}

// Automatically implement ValidateExt for all Validate implementations
impl<T: Validate> ValidateExt for T {}

// ============================================================================
// IMPORT COMBINATOR TYPES
// ============================================================================
// Import the actual combinator implementations instead of duplicating them

pub use crate::combinators::and::And;
#[cfg(feature = "caching")]
pub use crate::combinators::cached::Cached;
pub use crate::combinators::not::Not;
pub use crate::combinators::optional::Optional;
pub use crate::combinators::or::Or;
pub use crate::combinators::when::When;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::ValidationError;

    // Simple test validator
    struct AlwaysValid;

    impl Validate for AlwaysValid {
        type Input = str;

        fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    #[test]
    fn test_validator_trait() {
        let validator = AlwaysValid;
        assert!(validator.validate("test").is_ok());
    }
}
