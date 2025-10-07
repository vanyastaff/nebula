//! AND combinator - logical conjunction of validators
//!
//! The AND combinator requires both validators to pass for the combined
//! validator to succeed. It short-circuits on the first failure.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = MinLength { min: 5 }
//!     .and(MaxLength { max: 20 })
//!     .and(AlphanumericOnly);
//!
//! // All three must pass
//! assert!(validator.validate("hello").is_ok());
//! assert!(validator.validate("hi").is_err()); // too short
//! ```

use crate::core::{TypedValidator, ValidatorMetadata, ValidationComplexity};

// ============================================================================
// AND COMBINATOR
// ============================================================================

/// Combines two validators with logical AND.
///
/// Both validators must pass for validation to succeed.
/// Evaluates left-to-right and short-circuits on first failure.
///
/// # Type Parameters
///
/// * `L` - Left validator type
/// * `R` - Right validator type
///
/// # Type Constraints
///
/// Both validators must:
/// - Validate the same input type
/// - Return the same error type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = MinLength { min: 3 }.and(MaxLength { max: 10 });
///
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err()); // fails MinLength
/// assert!(validator.validate("verylongstring").is_err()); // fails MaxLength
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct And<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> And<L, R> {
    /// Creates a new AND combinator.
    ///
    /// # Arguments
    ///
    /// * `left` - First validator to check
    /// * `right` - Second validator to check (only if first passes)
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }

    /// Returns a reference to the left validator.
    pub fn left(&self) -> &L {
        &self.left
    }

    /// Returns a reference to the right validator.
    pub fn right(&self) -> &R {
        &self.right
    }

    /// Decomposes the combinator into its parts.
    pub fn into_parts(self) -> (L, R) {
        (self.left, self.right)
    }
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<L, R> TypedValidator for And<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Error = L::Error>,
{
    type Input = L::Input;
    type Output = ();
    type Error = L::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Short-circuit: if left fails, don't check right
        self.left.validate(input)?;
        self.right.validate(input)?;
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();

        // Combined complexity is the maximum of both
        let complexity = std::cmp::max(left_meta.complexity, right_meta.complexity);

        // Cacheable only if both are cacheable
        let cacheable = left_meta.cacheable && right_meta.cacheable;

        ValidatorMetadata {
            name: format!("And({}, {})", left_meta.name, right_meta.name),
            description: Some(format!(
                "Both {} and {} must pass",
                left_meta.name, right_meta.name
            )),
            complexity,
            cacheable,
            estimated_time: None,
            tags: {
                let mut tags = left_meta.tags;
                tags.extend(right_meta.tags);
                tags.push("combinator".to_string());
                tags
            },
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// ASYNC VALIDATOR IMPLEMENTATION
// ============================================================================

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<L, R> crate::core::AsyncValidator for And<L, R>
where
    L: TypedValidator + crate::core::AsyncValidator<
        Input = <L as TypedValidator>::Input,
        Error = <L as TypedValidator>::Error
    > + Send + Sync,
    R: TypedValidator<Input = <L as TypedValidator>::Input, Error = <L as TypedValidator>::Error>
        + crate::core::AsyncValidator<Input = <L as TypedValidator>::Input, Error = <L as TypedValidator>::Error>
        + Send + Sync,
    <L as TypedValidator>::Input: Sync,
{
    type Input = <L as TypedValidator>::Input;
    type Output = ();
    type Error = <L as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Short-circuit: if left fails, don't check right
        self.left.validate_async(input).await?;
        self.right.validate_async(input).await?;
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// BUILDER METHODS
// ============================================================================

impl<L, R> And<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Error = L::Error>,
{
    /// Chains another validator with AND.
    ///
    /// Creates `And(And(left, right), other)`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = min.and(max).and(alphanumeric);
    /// // Equivalent to: (min AND max) AND alphanumeric
    /// ```
    pub fn and<V>(self, other: V) -> And<Self, V>
    where
        Self: TypedValidator,
        V: TypedValidator<Input = <Self as TypedValidator>::Input, Error = <Self as TypedValidator>::Error>,
    {
        And::new(self, other)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates an AND combinator from two validators.
///
/// This is a convenience function that's equivalent to `left.and(right)`.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::and;
///
/// let validator = and(min_length(5), max_length(20));
/// ```
pub fn and<L, R>(left: L, right: R) -> And<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Error = L::Error>,
{
    And::new(left, right)
}

/// Creates an AND combinator from a slice of validators.
///
/// All validators in the slice must pass.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::and_all;
///
/// let validators = vec![
///     min_length(3),
///     max_length(20),
///     alphanumeric(),
/// ];
///
/// let combined = and_all(validators);
/// ```
pub fn and_all<V>(validators: Vec<V>) -> impl TypedValidator<Input = V::Input, Output = (), Error = V::Error>
where
    V: TypedValidator,
{
    AndAll { validators }
}

/// Validator that checks if all validators in a collection pass.
#[derive(Debug, Clone)]
pub struct AndAll<V> {
    validators: Vec<V>,
}

impl<V> TypedValidator for AndAll<V>
where
    V: TypedValidator,
{
    type Input = V::Input;
    type Output = ();
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        for validator in &self.validators {
            validator.validate(input)?;
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        let mut complexity = ValidationComplexity::Constant;
        let mut cacheable = true;
        let mut tags = Vec::new();

        for validator in &self.validators {
            let meta = validator.metadata();
            complexity = std::cmp::max(complexity, meta.complexity);
            cacheable = cacheable && meta.cacheable;
            tags.extend(meta.tags);
        }

        ValidatorMetadata {
            name: format!("AndAll(count={})", self.validators.len()),
            description: Some(format!("All {} validators must pass", self.validators.len())),
            complexity,
            cacheable,
            estimated_time: None,
            tags,
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// LAWS AND PROPERTIES
// ============================================================================

#[cfg(test)]
mod laws {
    use super::*;
    use crate::core::ValidationError;

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
    fn test_associativity() {
        // (a AND b) AND c === a AND (b AND c)
        let a = AlwaysValid;
        let b = AlwaysValid;
        let c = AlwaysValid;

        let left = And::new(And::new(a, b), c);
        let right = And::new(AlwaysValid, And::new(AlwaysValid, AlwaysValid));

        assert_eq!(
            left.validate("test").is_ok(),
            right.validate("test").is_ok()
        );
    }

    #[test]
    fn test_commutativity_fails() {
        // AND is NOT commutative in terms of which error is returned
        // But it IS commutative in terms of success/failure
        let a = AlwaysFails;
        let b = AlwaysValid;

        let left = And::new(a, b);
        let right = And::new(AlwaysValid, AlwaysFails);

        // Both fail (commutative for boolean result)
        assert_eq!(
            left.validate("test").is_ok(),
            right.validate("test").is_ok()
        );
        assert!(left.validate("test").is_err());
        assert!(right.validate("test").is_err());
    }

    #[test]
    fn test_short_circuit() {
        // If left fails, right should not be evaluated
        let mut right_called = false;

        struct ChecksCall<'a> {
            flag: &'a mut bool,
        }

        impl<'a> TypedValidator for ChecksCall<'a> {
            type Input = str;
            type Output = ();
            type Error = ValidationError;
            fn validate(&self, _: &str) -> Result<(), ValidationError> {
                *self.flag = true;
                Ok(())
            }
        }

        let left = AlwaysFails;
        let right = ChecksCall {
            flag: &mut right_called,
        };

        let validator = And::new(left, right);
        let _ = validator.validate("test");

        // Right should not have been called
        assert!(!right_called);
    }

    #[test]
    fn test_identity() {
        // a AND AlwaysValid === a
        let a = AlwaysFails;
        let identity = And::new(a, AlwaysValid);

        assert_eq!(
            AlwaysFails.validate("test").is_ok(),
            identity.validate("test").is_ok()
        );
    }
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

    struct MaxLength {
        max: usize,
    }

    impl TypedValidator for MaxLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() <= self.max {
                Ok(())
            } else {
                Err(ValidationError::max_length("", self.max, input.len()))
            }
        }
    }

    #[test]
    fn test_and_both_pass() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_and_left_fails() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_right_fails() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("verylongstring").is_err());
    }

    #[test]
    fn test_and_both_fail() {
        let validator = And::new(MinLength { min: 10 }, MaxLength { max: 5 });
        assert!(validator.validate("hello").is_err());
    }

    #[test]
    fn test_and_chain() {
        let validator = MinLength { min: 3 }
            .and(MaxLength { max: 10 })
            .and(MinLength { min: 5 });

        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_metadata() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        let meta = validator.metadata();

        assert!(meta.name.contains("And"));
        assert!(meta.description.is_some());
    }

    #[test]
    fn test_and_all() {
        let validators = vec![
            MinLength { min: 3 },
            MinLength { min: 5 },
            MinLength { min: 7 },
        ];

        let combined = and_all(validators);
        assert!(combined.validate("helloworld").is_ok());
        assert!(combined.validate("hello").is_err());
    }

    #[test]
    fn test_into_parts() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        let (left, right) = validator.into_parts();

        assert_eq!(left.min, 5);
        assert_eq!(right.max, 10);
    }
}