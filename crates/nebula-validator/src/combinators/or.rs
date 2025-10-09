//! OR combinator - logical disjunction of validators
//!
//! The OR combinator requires at least one validator to pass for the combined
//! validator to succeed. It short-circuits on the first success.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = ExactLength { length: 5 }
//!     .or(ExactLength { length: 10 });
//!
//! // Either can pass
//! assert!(validator.validate("hello").is_ok());      // length 5
//! assert!(validator.validate("helloworld").is_ok()); // length 10
//! assert!(validator.validate("hi").is_err());        // neither
//! ```

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// OR COMBINATOR
// ============================================================================

/// Combines two validators with logical OR.
///
/// At least one validator must pass for validation to succeed.
/// Evaluates left-to-right and short-circuits on first success.
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
/// - Return the same output type
/// - Return the same error type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// let validator = StartsWith { prefix: "http://" }
///     .or(StartsWith { prefix: "https://" });
///
/// assert!(validator.validate("http://example.com").is_ok());
/// assert!(validator.validate("https://example.com").is_ok());
/// assert!(validator.validate("ftp://example.com").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Or<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> Or<L, R> {
    /// Creates a new OR combinator.
    ///
    /// # Arguments
    ///
    /// * `left` - First validator to try
    /// * `right` - Second validator to try (only if first fails)
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

impl<L, R> TypedValidator for Or<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Output = L::Output, Error = L::Error>,
{
    type Input = L::Input;
    type Output = L::Output;
    type Error = OrError<L::Error>;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Try left first
        match self.left.validate(input) {
            Ok(output) => Ok(output),
            Err(left_error) => {
                // Left failed, try right
                match self.right.validate(input) {
                    Ok(output) => Ok(output),
                    Err(right_error) => {
                        // Both failed
                        Err(OrError {
                            left_error,
                            right_error,
                        })
                    }
                }
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();

        // Combined complexity is the maximum (worst case: both run)
        let complexity = std::cmp::max(left_meta.complexity, right_meta.complexity);

        // Cacheable only if both are cacheable
        let cacheable = left_meta.cacheable && right_meta.cacheable;

        ValidatorMetadata {
            name: format!("Or({}, {})", left_meta.name, right_meta.name),
            description: Some(format!(
                "Either {} or {} must pass",
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
// OR ERROR TYPE
// ============================================================================

/// Error type for OR combinator.
///
/// Contains both errors when both validators fail.
#[derive(Debug, Clone)]
pub struct OrError<E> {
    /// Error from the left validator.
    pub left_error: E,
    /// Error from the right validator.
    pub right_error: E,
}

impl<E: std::fmt::Display> std::fmt::Display for OrError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "All validators failed. Left: {}; Right: {}",
            self.left_error, self.right_error
        )
    }
}

impl<E: std::error::Error + 'static> std::error::Error for OrError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.left_error)
    }
}

// ============================================================================
// ASYNC VALIDATOR IMPLEMENTATION
// ============================================================================

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<L, R> crate::core::AsyncValidator for Or<L, R>
where
    L: TypedValidator
        + crate::core::AsyncValidator<
            Input = <L as TypedValidator>::Input,
            Output = <L as TypedValidator>::Output,
            Error = <L as TypedValidator>::Error,
        > + Send
        + Sync,
    R: TypedValidator<
            Input = <L as TypedValidator>::Input,
            Output = <L as TypedValidator>::Output,
            Error = <L as TypedValidator>::Error,
        > + crate::core::AsyncValidator<
            Input = <L as TypedValidator>::Input,
            Output = <L as TypedValidator>::Output,
            Error = <L as TypedValidator>::Error,
        > + Send
        + Sync,
    <L as TypedValidator>::Input: Sync,
    <L as TypedValidator>::Output: Send,
{
    type Input = <L as TypedValidator>::Input;
    type Output = <L as TypedValidator>::Output;
    type Error = OrError<<L as TypedValidator>::Error>;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Try left first
        match self.left.validate_async(input).await {
            Ok(output) => Ok(output),
            Err(left_error) => {
                // Left failed, try right
                match self.right.validate_async(input).await {
                    Ok(output) => Ok(output),
                    Err(right_error) => Err(OrError {
                        left_error,
                        right_error,
                    }),
                }
            }
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// BUILDER METHODS
// ============================================================================

impl<L, R> Or<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Output = L::Output, Error = L::Error>,
{
    /// Chains another validator with OR.
    ///
    /// Creates `Or(Or(left, right), other)`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = exact_5.or(exact_10).or(exact_15);
    /// // Equivalent to: (5 OR 10) OR 15
    /// ```
    pub fn or<V>(self, other: V) -> Or<Self, V>
    where
        Self: TypedValidator,
        V: TypedValidator<
                Input = <Self as TypedValidator>::Input,
                Output = <Self as TypedValidator>::Output,
                Error = <Self as TypedValidator>::Error,
            >,
    {
        Or::new(self, other)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates an OR combinator from two validators.
///
/// This is a convenience function that's equivalent to `left.or(right)`.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::or;
///
/// let validator = or(exact_length(5), exact_length(10));
/// ```
pub fn or<L, R>(left: L, right: R) -> Or<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Output = L::Output, Error = L::Error>,
{
    Or::new(left, right)
}

/// Creates an OR combinator from a slice of validators.
///
/// At least one validator must pass.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::or_any;
///
/// let validators = vec![
///     exact_length(5),
///     exact_length(10),
///     exact_length(15),
/// ];
///
/// let combined = or_any(validators);
/// ```
pub fn or_any<V>(
    validators: Vec<V>,
) -> impl TypedValidator<Input = V::Input, Output = V::Output, Error = OrAnyError<V::Error>>
where
    V: TypedValidator,
{
    OrAny { validators }
}

/// Validator that checks if any validator in a collection passes.
#[derive(Debug, Clone)]
pub struct OrAny<V> {
    validators: Vec<V>,
}

/// Error type for OrAny combinator.
#[derive(Debug, Clone)]
pub struct OrAnyError<E> {
    /// All errors from failed validators.
    pub errors: Vec<E>,
}

impl<E: std::fmt::Display> std::fmt::Display for OrAnyError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "All {} validators failed:", self.errors.len())?;
        for (i, error) in self.errors.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, error)?;
        }
        Ok(())
    }
}

impl<E: std::error::Error + 'static> std::error::Error for OrAnyError<E> {}

impl<V> TypedValidator for OrAny<V>
where
    V: TypedValidator,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = OrAnyError<V::Error>;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            match validator.validate(input) {
                Ok(output) => return Ok(output),
                Err(e) => errors.push(e),
            }
        }

        Err(OrAnyError { errors })
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
            name: format!("OrAny(count={})", self.validators.len()),
            description: Some(format!(
                "At least one of {} validators must pass",
                self.validators.len()
            )),
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
        // (a OR b) OR c === a OR (b OR c)
        let left_result = Or::new(Or::new(AlwaysFails, AlwaysValid), AlwaysFails)
            .validate("test")
            .is_ok();

        let right_result = Or::new(AlwaysFails, Or::new(AlwaysValid, AlwaysFails))
            .validate("test")
            .is_ok();

        assert_eq!(left_result, right_result);
    }

    #[test]
    fn test_commutativity() {
        // a OR b has same success/failure as b OR a
        let left = Or::new(AlwaysFails, AlwaysValid);
        let right = Or::new(AlwaysValid, AlwaysFails);

        assert_eq!(
            left.validate("test").is_ok(),
            right.validate("test").is_ok()
        );
    }

    #[test]
    fn test_short_circuit() {
        use std::cell::Cell;

        // If left succeeds, right should not be evaluated
        let right_called = Cell::new(false);

        struct ChecksCall<'a> {
            flag: &'a Cell<bool>,
        }

        impl<'a> TypedValidator for ChecksCall<'a> {
            type Input = str;
            type Output = ();
            type Error = ValidationError;
            fn validate(&self, _: &str) -> Result<(), ValidationError> {
                self.flag.set(true);
                Ok(())
            }
        }

        let left = AlwaysValid;
        let right = ChecksCall {
            flag: &right_called,
        };

        let validator = Or::new(left, right);
        let _ = validator.validate("test");

        // Right should not have been called
        assert!(!right_called.get());
    }

    #[test]
    fn test_identity() {
        // a OR AlwaysFails === a
        let a = AlwaysValid;
        let identity = Or::new(a, AlwaysFails);

        assert_eq!(
            AlwaysValid.validate("test").is_ok(),
            identity.validate("test").is_ok()
        );
    }

    #[test]
    fn test_annihilator() {
        // a OR AlwaysValid === AlwaysValid
        let annihilator = Or::new(AlwaysFails, AlwaysValid);
        assert!(annihilator.validate("test").is_ok());
    }
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{TypedValidator, ValidatorExt};

    struct ExactLength {
        length: usize,
    }

    impl TypedValidator for ExactLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() == self.length {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "exact_length",
                    format!("Length must be exactly {}", self.length),
                ))
            }
        }
    }

    #[test]
    fn test_or_left_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_or_right_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_or_both_pass() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 5 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_or_both_fail() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        let result = validator.validate("hi");
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("All validators failed"));
    }

    #[test]
    fn test_or_chain() {
        let validator = ExactLength { length: 5 }
            .or(ExactLength { length: 10 })
            .or(ExactLength { length: 15 });

        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("helloworld").is_ok());
        assert!(validator.validate("helloworldhello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_or_metadata() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        let meta = validator.metadata();

        assert!(meta.name.contains("Or"));
        assert!(meta.description.is_some());
    }

    #[test]
    fn test_or_any() {
        let validators = vec![
            ExactLength { length: 5 },
            ExactLength { length: 10 },
            ExactLength { length: 15 },
        ];

        let combined = or_any(validators);
        assert!(combined.validate("hello").is_ok());
        assert!(combined.validate("helloworld").is_ok());
        assert!(combined.validate("hi").is_err());
    }

    #[test]
    fn test_into_parts() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        let (left, right) = validator.into_parts();

        assert_eq!(left.length, 5);
        assert_eq!(right.length, 10);
    }

    #[test]
    fn test_or_error_display() {
        let error = OrError {
            left_error: ValidationError::new("left", "Left failed"),
            right_error: ValidationError::new("right", "Right failed"),
        };

        let display = error.to_string();
        assert!(display.contains("Left failed"));
        assert!(display.contains("Right failed"));
    }
}
