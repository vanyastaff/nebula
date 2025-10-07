//! WHEN combinator - conditional validation
//!
//! The WHEN combinator only applies validation when a condition is met.
//! If the condition returns false, validation is skipped and succeeds.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // Only validate length if string starts with "long_"
//! let validator = MinLength { min: 10 }
//!     .when(|s: &&str| s.starts_with("long_"));
//!
//! assert!(validator.validate("short").is_ok());        // condition false, skipped
//! assert!(validator.validate("long_enough").is_ok());  // condition true, validates
//! assert!(validator.validate("long_").is_err());       // condition true, too short
//! ```

use crate::core::{TypedValidator, ValidatorMetadata};

// ============================================================================
// WHEN COMBINATOR
// ============================================================================

/// Conditionally applies a validator based on a predicate.
///
/// The validator only runs if the condition returns `true`.
/// If the condition returns `false`, validation is skipped and succeeds.
///
/// # Type Parameters
///
/// * `V` - Inner validator type
/// * `C` - Condition function type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// // Only validate email format for non-empty strings
/// let validator = EmailValidator
///     .when(|s: &&str| !s.is_empty());
///
/// assert!(validator.validate("").is_ok());              // empty, skipped
/// assert!(validator.validate("user@example.com").is_ok()); // valid email
/// assert!(validator.validate("invalid").is_err());      // invalid email
/// ```
#[derive(Debug, Clone, Copy)]
pub struct When<V, C> {
    pub(crate) validator: V,
    pub(crate) condition: C,
}

impl<V, C> When<V, C> {
    /// Creates a new WHEN combinator.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to apply conditionally
    /// * `condition` - Predicate that determines if validation should run
    pub fn new(validator: V, condition: C) -> Self {
        Self {
            validator,
            condition,
        }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns a reference to the condition function.
    pub fn condition(&self) -> &C {
        &self.condition
    }

    /// Extracts the validator and condition.
    pub fn into_parts(self) -> (V, C) {
        (self.validator, self.condition)
    }
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<V, C> TypedValidator for When<V, C>
where
    V: TypedValidator,
    C: Fn(&V::Input) -> bool,
{
    type Input = V::Input;
    type Output = ();
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if (self.condition)(input) {
            // Condition met, apply validation
            self.validator.validate(input)?;
            Ok(())
        } else {
            // Condition not met, skip validation
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("When({})", inner_meta.name),
            description: Some(format!("Conditionally apply {}", inner_meta.name)),
            complexity: inner_meta.complexity,
            cacheable: false, // Can't cache because condition may vary
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("conditional".to_string());
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
impl<V, C> crate::core::AsyncValidator for When<V, C>
where
    V: TypedValidator + crate::core::AsyncValidator<
        Input = <V as TypedValidator>::Input,
        Output = <V as TypedValidator>::Output,
        Error = <V as TypedValidator>::Error
    > + Send + Sync,
    C: Fn(&<V as TypedValidator>::Input) -> bool + Send + Sync,
    <V as TypedValidator>::Input: Sync,
{
    type Input = <V as TypedValidator>::Input;
    type Output = ();
    type Error = <V as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if (self.condition)(input) {
            self.validator.validate_async(input).await?;
            Ok(())
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// BUILDER METHODS
// ============================================================================

impl<V, C> When<V, C> {
    /// Chains another conditional validator.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = min_length(5)
    ///     .when(|s| s.starts_with("long_"))
    ///     .when(|s| !s.is_empty());
    /// ```
    pub fn when<C2>(self, condition: C2) -> When<Self, C2>
    where
        C2: Fn(&<Self as TypedValidator>::Input) -> bool,
        Self: TypedValidator,
    {
        When::new(self, condition)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a WHEN combinator from a validator and condition.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::when;
///
/// let validator = when(min_length(10), |s: &&str| s.starts_with("long_"));
/// ```
pub fn when<V, C>(validator: V, condition: C) -> When<V, C>
where
    V: TypedValidator,
    C: Fn(&V::Input) -> bool,
{
    When::new(validator, condition)
}

/// Creates a validator that only runs when input is not empty.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::when_not_empty;
///
/// let validator = when_not_empty(email_validator);
/// assert!(validator.validate("").is_ok()); // empty, skipped
/// ```
pub fn when_not_empty<V>(validator: V) -> When<V, impl Fn(&V::Input) -> bool>
where
    V: TypedValidator,
    V::Input: AsRef<str>,
{
    When::new(validator, |input: &V::Input| !input.as_ref().is_empty())
}

/// Creates a validator that only runs when input is Some.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::when_some;
///
/// let validator = when_some(min_length(5));
/// assert!(validator.validate(&None).is_ok()); // None, skipped
/// ```
pub fn when_some<V, T>(validator: V) -> When<V, impl Fn(&Option<T>) -> bool>
where
    V: TypedValidator<Input = T>,
{
    When::new(validator, |input: &Option<T>| input.is_some())
}

// ============================================================================
// UNLESS COMBINATOR (opposite of WHEN)
// ============================================================================

/// Applies validation UNLESS condition is true.
///
/// This is the inverse of WHEN - validates when condition is false.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::unless;
///
/// // Don't validate if string starts with "skip_"
/// let validator = unless(min_length(10), |s: &&str| s.starts_with("skip_"));
///
/// assert!(validator.validate("skip_short").is_ok()); // skipped
/// assert!(validator.validate("short").is_err());     // validated, fails
/// ```
pub fn unless<V, C>(validator: V, condition: C) -> When<V, Box<dyn for<'a> Fn(&'a <V as TypedValidator>::Input) -> bool>>
where
    V: TypedValidator,
    C: for<'a> Fn(&'a <V as TypedValidator>::Input) -> bool + 'static,
{
    When::new(validator, Box::new(move |input| !condition(input)))
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ValidationError;

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
    fn test_when_condition_true() {
        let validator = When::new(MinLength { min: 10 }, |s: &&str| s.starts_with("long_"));

        // Condition true, validates
        assert!(validator.validate("long_enough_string").is_ok());
        assert!(validator.validate("long_short").is_err());
    }

    #[test]
    fn test_when_condition_false() {
        let validator = When::new(MinLength { min: 10 }, |s: &&str| s.starts_with("long_"));

        // Condition false, skips validation
        assert!(validator.validate("short").is_ok());
    }

    #[test]
    fn test_when_chain() {
        let validator = MinLength { min: 5 }
            .when(|s: &&str| s.starts_with("check_"))
            .when(|s: &&str| !s.is_empty());

        assert!(validator.validate("").is_ok()); // empty, second condition false
        assert!(validator.validate("other").is_ok()); // first condition false
        assert!(validator.validate("check_hello").is_ok()); // both true, validates OK
        assert!(validator.validate("check_hi").is_err()); // both true, too short
    }

    #[test]
    fn test_when_not_empty() {
        let validator = when_not_empty(MinLength { min: 5 });

        assert!(validator.validate("").is_ok()); // empty, skipped
        assert!(validator.validate("hello").is_ok()); // not empty, valid
        assert!(validator.validate("hi").is_err()); // not empty, too short
    }

    #[test]
    fn test_unless() {
        let validator = unless(MinLength { min: 10 }, |s: &&str| s.starts_with("skip_"));

        assert!(validator.validate("skip_short").is_ok()); // skipped
        assert!(validator.validate("short").is_err()); // not skipped, too short
        assert!(validator.validate("long_enough_string").is_ok()); // not skipped, valid
    }

    #[test]
    fn test_when_metadata() {
        let validator = When::new(MinLength { min: 5 }, |s: &&str| s.starts_with("test"));
        let meta = validator.metadata();

        assert!(meta.name.contains("When"));
        assert!(!meta.cacheable); // Conditional validators can't be cached
        assert!(meta.tags.contains(&"conditional".to_string()));
    }

    #[test]
    fn test_into_parts() {
        let min_length = MinLength { min: 5 };
        let condition = |s: &&str| s.starts_with("test");
        let validator = When::new(min_length, condition);

        let (extracted_validator, _extracted_condition) = validator.into_parts();
        assert_eq!(extracted_validator.min, 5);
    }

    #[test]
    fn test_when_with_complex_condition() {
        let validator = MinLength { min: 10 }.when(|s: &&str| {
            s.starts_with("long_") && s.contains("_test")
        });

        assert!(validator.validate("short").is_ok()); // condition false
        assert!(validator.validate("long_other").is_ok()); // condition false
        assert!(validator.validate("long_test_string").is_ok()); // condition true, valid
        assert!(validator.validate("long_test").is_err()); // condition true, too short
    }
}