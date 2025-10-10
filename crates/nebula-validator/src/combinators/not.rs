//! NOT combinator - logical negation of validators
//!
//! The NOT combinator inverts a validator: it succeeds when the inner
//! validator fails, and fails when the inner validator succeeds.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = Contains { substring: "test" }.not();
//!
//! assert!(validator.validate("hello world").is_ok());  // doesn't contain "test"
//! assert!(validator.validate("test string").is_err()); // contains "test"
//! ```

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};

// ============================================================================
// NOT COMBINATOR
// ============================================================================

/// Inverts a validator with logical NOT.
///
/// The combined validator succeeds when the inner validator fails,
/// and fails when the inner validator succeeds.
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
/// let validator = StartsWith { prefix: "http://" }.not();
///
/// assert!(validator.validate("example.com").is_ok());  // doesn't start with http://
/// assert!(validator.validate("http://example.com").is_err()); // starts with http://
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Not<V> {
    pub(crate) inner: V,
}

impl<V> Not<V> {
    /// Creates a new NOT combinator.
    ///
    /// # Arguments
    ///
    /// * `inner` - The validator to invert
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

impl<V> TypedValidator for Not<V>
where
    V: TypedValidator,
{
    type Input = V::Input;
    type Output = ();
    type Error = NotError<V::Error>;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match self.inner.validate(input) {
            Ok(_) => Err(NotError::ValidatorPassed),
            Err(_inner_error) => {
                // Inner validator failed, so NOT succeeds
                Ok(())
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.inner.metadata();

        ValidatorMetadata {
            name: format!("Not({})", inner_meta.name),
            description: Some(format!("{} must NOT pass", inner_meta.name)),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("negation".to_string());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

// ============================================================================
// NOT ERROR TYPE
// ============================================================================

/// Error type for NOT combinator.
#[derive(Debug, Clone)]
pub enum NotError<E> {
    /// The inner validator passed (which means NOT failed).
    ValidatorPassed,
    /// Contains the original error for reference (usually not used).
    _InnerError(E),
}

impl<E> std::fmt::Display for NotError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotError::ValidatorPassed => {
                write!(f, "Validation must NOT pass, but it did")
            }
            NotError::_InnerError(_) => {
                write!(f, "NOT combinator error")
            }
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for NotError<E> {}

// Convert to ValidationError for convenience
impl<E> From<NotError<E>> for ValidationError {
    fn from(error: NotError<E>) -> Self {
        match error {
            NotError::ValidatorPassed => {
                ValidationError::new("not_validator", "Value must NOT pass validation")
            }
            NotError::_InnerError(_) => {
                ValidationError::new("not_validator", "NOT validator error")
            }
        }
    }
}

// ============================================================================
// ASYNC VALIDATOR IMPLEMENTATION
// ============================================================================

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<V> crate::core::AsyncValidator for Not<V>
where
    V: TypedValidator
        + crate::core::AsyncValidator<
            Input = <V as TypedValidator>::Input,
            Output = <V as TypedValidator>::Output,
            Error = <V as TypedValidator>::Error,
        > + Send
        + Sync,
    <V as TypedValidator>::Input: Sync,
{
    type Input = <V as TypedValidator>::Input;
    type Output = ();
    type Error = NotError<<V as TypedValidator>::Error>;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match self.inner.validate_async(input).await {
            Ok(_) => Err(NotError::ValidatorPassed),
            Err(_inner_error) => Ok(()),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// BUILDER METHODS
// ============================================================================

impl<V> Not<V> {
    /// Double negation - inverts the NOT.
    ///
    /// `NOT(NOT(validator))` is equivalent to `validator`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = contains("test").not().not();
    /// // Equivalent to: contains("test")
    /// ```
    pub fn not(self) -> V {
        self.inner
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a NOT combinator from a validator.
///
/// This is a convenience function that's equivalent to `validator.not()`.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::not;
///
/// let validator = not(contains("test"));
/// ```
pub fn not<V>(validator: V) -> Not<V>
where
    V: TypedValidator,
{
    Not::new(validator)
}

// ============================================================================
// LAWS AND PROPERTIES
// ============================================================================

#[cfg(test)]
mod laws {
    use super::*;

    // Test validators
    struct AlwaysValid;
    impl TypedValidator for AlwaysValid {
        type Input = str;
        type Output = ();
        type Error = ValidationError;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;
    impl TypedValidator for AlwaysFails {
        type Input = str;
        type Output = ();
        type Error = ValidationError;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Err(ValidationError::new("fail", "Always fails"))
        }
    }

    #[test]
    fn test_double_negation() {
        // NOT(NOT(a)) === a
        let validator = AlwaysValid;
        let double_not = Not::new(Not::new(validator));

        // Double NOT should behave like the original
        assert_eq!(
            AlwaysValid.validate("test").is_ok(),
            double_not.validate("test").is_ok()
        );
    }

    #[test]
    fn test_not_inverts() {
        // If a passes, NOT(a) fails
        let passes = AlwaysValid;
        let not_passes = Not::new(passes);
        assert!(not_passes.validate("test").is_err());

        // If a fails, NOT(a) passes
        let fails = AlwaysFails;
        let not_fails = Not::new(fails);
        assert!(not_fails.validate("test").is_ok());
    }

    #[test]
    fn test_de_morgan_and() {
        // NOT(a AND b) === NOT(a) OR NOT(b)
        // We can't test this directly without AND/OR, but we can verify
        // that NOT inverts the result
        use crate::combinators::And;

        let and_validator = And::new(AlwaysValid, AlwaysValid);
        let not_and = Not::new(and_validator);

        assert!(and_validator.validate("test").is_ok());
        assert!(not_and.validate("test").is_err());
    }
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{TypedValidator, ValidatorExt};

    struct Contains {
        substring: String,
    }

    impl TypedValidator for Contains {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.contains(&self.substring) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "contains",
                    format!("Must contain '{}'", self.substring),
                ))
            }
        }
    }

    #[test]
    fn test_not_passes_when_inner_fails() {
        let validator = Not::new(Contains {
            substring: "test".to_string(),
        });
        assert!(validator.validate("hello world").is_ok());
    }

    #[test]
    fn test_not_fails_when_inner_passes() {
        let validator = Not::new(Contains {
            substring: "test".to_string(),
        });
        assert!(validator.validate("test string").is_err());
    }

    #[test]
    fn test_not_double_negation() {
        let validator = Contains {
            substring: "test".to_string(),
        }
        .not()
        .not();

        // Should behave like original
        assert!(validator.validate("test string").is_ok());
        assert!(validator.validate("hello world").is_err());
    }

    #[test]
    fn test_not_metadata() {
        let validator = Not::new(Contains {
            substring: "test".to_string(),
        });
        let meta = validator.metadata();

        assert!(meta.name.contains("Not"));
        assert!(meta.description.is_some());
        assert!(meta.tags.contains(&"negation".to_string()));
    }

    #[test]
    fn test_into_inner() {
        let inner = Contains {
            substring: "test".to_string(),
        };
        let validator = Not::new(inner);
        let extracted = validator.into_inner();

        assert_eq!(extracted.substring, "test");
    }

    #[test]
    fn test_not_error_display() {
        let error = NotError::<ValidationError>::ValidatorPassed;
        let display = error.to_string();
        assert!(display.contains("must NOT pass"));
    }

    #[test]
    fn test_not_with_and() {
        use crate::combinators::And;

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

        // NOT(MinLength AND Contains)
        let validator = And::new(
            MinLength { min: 5 },
            Contains {
                substring: "test".to_string(),
            },
        )
        .not();

        // Should pass when either condition fails
        assert!(validator.validate("hi").is_ok()); // too short
        assert!(validator.validate("hello world").is_ok()); // no "test"

        // Should fail when both conditions pass
        assert!(validator.validate("test string").is_err());
    }
}
