//! Core traits for the validation system
//!
//! This module defines the fundamental traits that all validators must implement.

use crate::core::{ValidationError, ValidatorMetadata};
use std::future::Future;

// ============================================================================
// CORE VALIDATOR TRAIT
// ============================================================================

/// The core trait that all validators must implement.
///
/// This trait is generic over the input type, allowing for compile-time
/// type safety while maintaining flexibility.
///
/// # Type Parameters
///
/// * `Input` - The type being validated (can be `?Sized` for DSTs like `str`)
/// * `Output` - The result of successful validation (often `()` or a refined type)
/// * `Error` - The error type returned on validation failure
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::{TypedValidator, ValidationError};
///
/// struct MinLength {
///     min: usize,
/// }
///
/// impl TypedValidator for MinLength {
///     type Input = str;
///     type Output = ();
///     type Error = ValidationError;
///
///     fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
pub trait TypedValidator {
    /// The type of input being validated.
    ///
    /// Use `?Sized` to allow validation of unsized types like `str` and `[T]`.
    type Input: ?Sized;

    /// The type returned on successful validation.
    ///
    /// This is often `()` for simple validators, but can be a refined type
    /// that carries additional guarantees.
    type Output;

    /// The error type returned on validation failure.
    ///
    /// Should implement `std::error::Error` for interoperability.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Validates the input value.
    ///
    /// # Arguments
    ///
    /// * `input` - The value to validate
    ///
    /// # Returns
    ///
    /// * `Ok(output)` if validation succeeds
    /// * `Err(error)` if validation fails
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;

    /// Returns metadata about this validator.
    ///
    /// Override this to provide introspection capabilities.
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::default()
    }

    /// Returns the name of this validator.
    ///
    /// Used for debugging and error messages.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

// ============================================================================
// ASYNC VALIDATOR TRAIT
// ============================================================================

/// Async version of the validator trait.
///
/// Use this for validators that need to perform I/O operations,
/// such as database lookups or API calls.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::{AsyncValidator, ValidationError};
///
/// struct EmailExists {
///     db_pool: DatabasePool,
/// }
///
/// #[async_trait::async_trait]
/// impl AsyncValidator for EmailExists {
///     type Input = str;
///     type Output = ();
///     type Error = ValidationError;
///
///     async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
///         let exists = self.db_pool.check_email_exists(input).await?;
///         if exists {
///             Ok(())
///         } else {
///             Err(ValidationError::new("email_not_found", "Email does not exist"))
///         }
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait AsyncValidator: Send + Sync {
    /// The type of input being validated.
    type Input: ?Sized + Sync;

    /// The type returned on successful validation.
    type Output: Send;

    /// The error type returned on validation failure.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Asynchronously validates the input value.
    ///
    /// # Arguments
    ///
    /// * `input` - The value to validate
    ///
    /// # Returns
    ///
    /// * `Ok(output)` if validation succeeds
    /// * `Err(error)` if validation fails
    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;

    /// Returns metadata about this validator.
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::default()
    }

    /// Returns the name of this validator.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

// ============================================================================
// VALIDATOR EXTENSION TRAIT
// ============================================================================

/// Extension trait providing combinator methods for validators.
///
/// This trait is automatically implemented for all types that implement
/// `TypedValidator`, providing a fluent API for composing validators.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = MinLength { min: 5 }
///     .and(MaxLength { max: 20 })
///     .and(AlphanumericOnly);
///
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// ```
pub trait ValidatorExt: TypedValidator + Sized {
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
    /// ```rust
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = MinLength { min: 3 }.and(MaxLength { max: 10 });
    /// assert!(validator.validate("hello").is_ok());
    /// assert!(validator.validate("hi").is_err()); // too short
    /// assert!(validator.validate("verylongstring").is_err()); // too long
    /// ```
    fn and<V>(self, other: V) -> And<Self, V>
    where
        V: TypedValidator<Input = Self::Input, Error = Self::Error>,
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
    /// ```rust
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = ExactLength { length: 5 }.or(ExactLength { length: 10 });
    /// assert!(validator.validate("hello").is_ok()); // length 5
    /// assert!(validator.validate("helloworld").is_ok()); // length 10
    /// assert!(validator.validate("hi").is_err()); // neither 5 nor 10
    /// ```
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: TypedValidator<Input = Self::Input, Output = Self::Output, Error = Self::Error>,
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
    /// ```rust
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = Contains { substring: "test" }.not();
    /// assert!(validator.validate("hello world").is_ok()); // doesn't contain "test"
    /// assert!(validator.validate("test string").is_err()); // contains "test"
    /// ```
    fn not(self) -> Not<Self> {
        Not::new(self)
    }

    /// Maps the output of a successful validation.
    ///
    /// This allows transforming the validation result without changing
    /// the validation logic.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = MinLength { min: 5 }.map(|_| "Valid!");
    /// assert_eq!(validator.validate("hello").unwrap(), "Valid!");
    /// ```
    fn map<F, O>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Output) -> O,
    {
        Map::new(self, f)
    }

    /// Makes validation conditional based on a predicate.
    ///
    /// The validator only runs if the condition returns `true`.
    /// If the condition returns `false`, validation is skipped.
    ///
    /// # Examples
    ///
    /// ```rust
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

    /// Adds caching to the validator.
    ///
    /// Results are cached based on the input value's hash.
    /// Use with caution for validators with side effects.
    ///
    /// # Requirements
    ///
    /// - Input must be `Hash` and `Eq`
    /// - Output and Error must be `Clone`
    fn cached(self) -> Cached<Self>
    where
        Self::Input: std::hash::Hash + Eq,
        Self::Output: Clone,
        Self::Error: Clone,
    {
        Cached::new(self)
    }

    /// Adds a custom error message on validation failure.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::prelude::*;
    ///
    /// let validator = MinLength { min: 5 }
    ///     .with_message(|_| ValidationError::new("custom", "Too short!"));
    /// ```
    fn with_message<F>(self, message_fn: F) -> WithMessage<Self, F>
    where
        F: Fn(&Self::Error) -> Self::Error,
    {
        WithMessage::new(self, message_fn)
    }

    /// Adds a timeout to the validator.
    ///
    /// Useful for preventing slow validators from blocking.
    #[cfg(feature = "async")]
    fn with_timeout(self, duration: std::time::Duration) -> WithTimeout<Self> {
        WithTimeout::new(self, duration)
    }
}

// Automatically implement ValidatorExt for all TypedValidator implementations
impl<T: TypedValidator> ValidatorExt for T {}

// ============================================================================
// FORWARD DECLARATIONS FOR COMBINATORS
// ============================================================================
// These are defined in separate modules but declared here for the trait

pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> And<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

pub struct Or<L, R> {
    left: L,
    right: R,
}

impl<L, R> Or<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

pub struct Not<V> {
    inner: V,
}

impl<V> Not<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

pub struct Map<V, F> {
    validator: V,
    mapper: F,
}

impl<V, F> Map<V, F> {
    pub fn new(validator: V, mapper: F) -> Self {
        Self { validator, mapper }
    }
}

pub struct When<V, C> {
    validator: V,
    condition: C,
}

impl<V, C> When<V, C> {
    pub fn new(validator: V, condition: C) -> Self {
        Self { validator, condition }
    }
}

pub struct Optional<V> {
    inner: V,
}

impl<V> Optional<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

pub struct Cached<V>
where
    V: TypedValidator,
{
    validator: V,
    cache: std::sync::RwLock<std::collections::HashMap<u64, CacheEntry<V>>>,
}

impl<V> Cached<V>
where
    V: TypedValidator,
{
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

struct CacheEntry<V: TypedValidator> {
    output: Option<V::Output>,
    error: Option<V::Error>,
}

pub struct WithMessage<V, F> {
    validator: V,
    message_fn: F,
}

impl<V, F> WithMessage<V, F> {
    pub fn new(validator: V, message_fn: F) -> Self {
        Self { validator, message_fn }
    }
}

#[cfg(feature = "async")]
pub struct WithTimeout<V> {
    validator: V,
    duration: std::time::Duration,
}

#[cfg(feature = "async")]
impl<V> WithTimeout<V> {
    pub fn new(validator: V, duration: std::time::Duration) -> Self {
        Self { validator, duration }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Simple test validator
    struct AlwaysValid;

    impl TypedValidator for AlwaysValid {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, _input: &Self::Input) -> Result<Self::Output, Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn test_validator_trait() {
        let validator = AlwaysValid;
        assert!(validator.validate("test").is_ok());
    }

    #[test]
    fn test_validator_name() {
        let validator = AlwaysValid;
        assert!(validator.name().contains("AlwaysValid"));
    }
}