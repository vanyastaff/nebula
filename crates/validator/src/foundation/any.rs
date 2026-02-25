//! Type-erased validators for dynamic dispatch.
//!
//! This module provides `AnyValidator<T>`, a type-erased wrapper around validators
//! that enables storing validators of different concrete types in collections,
//! returning validators from functions without exposing internal types, and
//! runtime-configured validation pipelines.
//!
//! # When to Use
//!
//! Use `AnyValidator` when:
//! - Storing validators in collections: `Vec<AnyValidator<str>>`
//! - Function return types where the concrete type is complex: `fn get_validator() -> AnyValidator<str>`
//! - Configuration-driven validation where types are dynamic
//!
//! # Performance
//!
//! `AnyValidator` uses dynamic dispatch (virtual calls) which has a small overhead
//! (~2-5 ns per validation). For hot paths, prefer using concrete types with generics.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::foundation::AnyValidator;
//! use nebula_validator::validators::{min_length, max_length};
//!
//! // Complex combinator type becomes simple
//! let complex = min_length(3).and(max_length(20));
//! let simple: AnyValidator<str> = AnyValidator::new(complex);
//!
//! // Store different validators in a collection
//! let validators: Vec<AnyValidator<str>> = vec![
//!     AnyValidator::new(min_length(5)),
//!     AnyValidator::new(max_length(100)),
//! ];
//! ```

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// ERASED VALIDATOR TRAIT (Internal)
// ============================================================================

/// Internal trait for type-erased validation.
trait ErasedValidator<T: ?Sized>: Send + Sync {
    fn validate_erased(&self, input: &T) -> Result<(), ValidationError>;
    fn clone_erased(&self) -> Box<dyn ErasedValidator<T>>;
}

impl<T: ?Sized, V> ErasedValidator<T> for V
where
    V: Validate<T> + Clone + Send + Sync + 'static,
{
    #[inline]
    fn validate_erased(&self, input: &T) -> Result<(), ValidationError> {
        self.validate(input)
    }

    fn clone_erased(&self) -> Box<dyn ErasedValidator<T>> {
        Box::new(self.clone())
    }
}

// ============================================================================
// ANY VALIDATOR
// ============================================================================

/// A type-erased validator that can hold any validator for input type `T`.
///
/// This enables storing validators of different concrete types in the same
/// collection, or returning validators from functions without exposing their
/// internal types.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::foundation::AnyValidator;
/// use nebula_validator::validators::{min_length, email};
///
/// // Store heterogeneous validators
/// let validators: Vec<AnyValidator<str>> = vec![
///     AnyValidator::new(min_length(3)),
///     AnyValidator::new(email()),
/// ];
///
/// // Validate against all
/// for v in &validators {
///     v.validate("test@example.com")?;
/// }
/// ```
pub struct AnyValidator<T: ?Sized> {
    inner: Box<dyn ErasedValidator<T>>,
}

impl<T: ?Sized> AnyValidator<T> {
    /// Creates a new type-erased validator from any validator implementing `Validate`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::foundation::AnyValidator;
    /// use nebula_validator::validators::min_length;
    ///
    /// let validator = AnyValidator::new(min_length(5));
    /// ```
    pub fn new<V>(validator: V) -> Self
    where
        V: Validate<T> + Clone + Send + Sync + 'static,
    {
        Self {
            inner: Box::new(validator),
        }
    }
}

impl<T: ?Sized> Validate<T> for AnyValidator<T> {
    #[inline]
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        self.inner.validate_erased(input)
    }
}

impl<T: ?Sized> Clone for AnyValidator<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_erased(),
        }
    }
}

impl<T: ?Sized> std::fmt::Debug for AnyValidator<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyValidator")
            .field("inner", &"<dyn ErasedValidator>")
            .finish()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::{max_length, min_length};

    #[test]
    fn test_any_validator_basic() {
        let validator: AnyValidator<str> = AnyValidator::new(min_length(3));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_any_validator_clone() {
        let v1: AnyValidator<str> = AnyValidator::new(min_length(3));
        let v2 = v1.clone();
        assert!(v1.validate("hello").is_ok());
        assert!(v2.validate("hello").is_ok());
    }

    #[test]
    fn test_any_validator_collection() {
        let validators: Vec<AnyValidator<str>> = vec![
            AnyValidator::new(min_length(3)),
            AnyValidator::new(max_length(10)),
        ];

        // "hello" passes both
        for v in &validators {
            assert!(v.validate("hello").is_ok());
        }

        // "hi" fails min_length
        assert!(validators[0].validate("hi").is_err());

        // "hello world!" fails max_length
        assert!(validators[1].validate("hello world!").is_err());
    }

    #[test]
    fn test_any_validator_debug() {
        let validator: AnyValidator<str> = AnyValidator::new(min_length(3));
        let debug = format!("{:?}", validator);
        assert!(debug.contains("AnyValidator"));
    }

    #[test]
    fn test_any_validator_new() {
        let validator = AnyValidator::new(min_length(5));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }
}
