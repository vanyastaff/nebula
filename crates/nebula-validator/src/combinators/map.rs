//! MAP combinator - transforms validation output
//!
//! The MAP combinator transforms the output of a successful validation
//! without changing the validation logic itself.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! let validator = MinLength { min: 5 }.map(|_| "Valid!");
//!
//! let result = validator.validate("hello")?;
//! assert_eq!(result, "Valid!");
//! ```

use crate::core::{TypedValidator, ValidatorMetadata};

// ============================================================================
// MAP COMBINATOR
// ============================================================================

/// Maps the output of a successful validation.
///
/// This combinator allows transforming the validation result without
/// changing the validation logic. The validation still succeeds or fails
/// based on the inner validator, but the success value can be transformed.
///
/// # Type Parameters
///
/// * `V` - Inner validator type
/// * `F` - Mapping function type
///
/// # Examples
///
/// ```rust
/// use nebula_validator::prelude::*;
///
/// // Transform () to a string
/// let validator = MinLength { min: 5 }.map(|_| "Valid string!");
/// assert_eq!(validator.validate("hello").unwrap(), "Valid string!");
///
/// // Extract information from validated value
/// let length_validator = MinLength { min: 3 }.map(|_| {
///     // Here you could access the validated value if needed
///     "passed"
/// });
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Map<V, F> {
    pub(crate) validator: V,
    pub(crate) mapper: F,
}

impl<V, F> Map<V, F> {
    /// Creates a new MAP combinator.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator to apply first
    /// * `mapper` - Function to transform the output
    pub fn new(validator: V, mapper: F) -> Self {
        Self { validator, mapper }
    }

    /// Returns a reference to the inner validator.
    pub fn validator(&self) -> &V {
        &self.validator
    }

    /// Returns a reference to the mapper function.
    pub fn mapper(&self) -> &F {
        &self.mapper
    }

    /// Extracts the validator and mapper.
    pub fn into_parts(self) -> (V, F) {
        (self.validator, self.mapper)
    }
}

// ============================================================================
// TYPED VALIDATOR IMPLEMENTATION
// ============================================================================

impl<V, F, O> TypedValidator for Map<V, F>
where
    V: TypedValidator,
    F: Fn(V::Output) -> O,
{
    type Input = V::Input;
    type Output = O;
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let output = self.validator.validate(input)?;
        Ok((self.mapper)(output))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("Map({})", inner_meta.name),
            description: inner_meta
                .description
                .map(|desc| format!("Mapped: {desc}")),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("map".to_string());
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
impl<V, F, O> crate::core::AsyncValidator for Map<V, F>
where
    V: TypedValidator
        + crate::core::AsyncValidator<
            Input = <V as TypedValidator>::Input,
            Output = <V as TypedValidator>::Output,
            Error = <V as TypedValidator>::Error,
        > + Send
        + Sync,
    F: Fn(<V as TypedValidator>::Output) -> O + Send + Sync,
    <V as TypedValidator>::Input: Sync,
    O: Send,
{
    type Input = <V as TypedValidator>::Input;
    type Output = O;
    type Error = <V as TypedValidator>::Error;

    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let output = self.validator.validate_async(input).await?;
        Ok((self.mapper)(output))
    }

    fn metadata(&self) -> ValidatorMetadata {
        <Self as TypedValidator>::metadata(self)
    }
}

// ============================================================================
// BUILDER METHODS
// ============================================================================

impl<V, F> Map<V, F> {
    /// Chains another map operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let validator = min_length(5)
    ///     .map(|_| 42)
    ///     .map(|n| n * 2);
    ///
    /// assert_eq!(validator.validate("hello").unwrap(), 84);
    /// ```
    pub fn map<G, O2>(self, g: G) -> Map<Self, G>
    where
        G: Fn(<Self as TypedValidator>::Output) -> O2,
        Self: TypedValidator,
    {
        Map::new(self, g)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Creates a MAP combinator from a validator and function.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::map;
///
/// let validator = map(min_length(5), |_| "Valid!");
/// ```
pub fn map<V, F, O>(validator: V, mapper: F) -> Map<V, F>
where
    V: TypedValidator,
    F: Fn(V::Output) -> O,
{
    Map::new(validator, mapper)
}

// ============================================================================
// SPECIAL MAP VARIANTS
// ============================================================================

/// Maps the validation output to a constant value.
///
/// Useful when you just want to mark that validation passed.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::map_to;
///
/// let validator = map_to(min_length(5), "VALID");
/// assert_eq!(validator.validate("hello").unwrap(), "VALID");
/// ```
pub fn map_to<V, O>(validator: V, value: O) -> Map<V, impl Fn(V::Output) -> O>
where
    V: TypedValidator,
    O: Clone,
{
    let value_clone = value.clone();
    Map::new(validator, move |_| value_clone.clone())
}

/// Maps validation to unit type `()`.
///
/// Useful for discarding the output when you only care about pass/fail.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::map_unit;
///
/// let validator = map_unit(some_validator);
/// validator.validate(input)?; // Just succeeds or fails
/// ```
pub fn map_unit<V>(validator: V) -> Map<V, impl Fn(V::Output)>
where
    V: TypedValidator,
{
    Map::new(validator, |_| ())
}

// ============================================================================
// MAP WITH CONTEXT
// ============================================================================

/// Maps the output using both the validation output and input.
///
/// This variant gives you access to the original input in the mapper.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::map_with_input;
///
/// let validator = map_with_input(min_length(5), |_, input| {
///     format!("Valid: {}", input)
/// });
///
/// assert_eq!(
///     validator.validate("hello").unwrap(),
///     "Valid: hello"
/// );
/// ```
pub fn map_with_input<V, F, O>(validator: V, mapper: F) -> MapWithInput<V, F>
where
    V: TypedValidator,
    F: Fn(V::Output, &V::Input) -> O,
{
    MapWithInput::new(validator, mapper)
}

/// MAP combinator that has access to the input.
#[derive(Debug, Clone, Copy)]
pub struct MapWithInput<V, F> {
    validator: V,
    mapper: F,
}

impl<V, F> MapWithInput<V, F> {
    pub fn new(validator: V, mapper: F) -> Self {
        Self { validator, mapper }
    }
}

impl<V, F, O> TypedValidator for MapWithInput<V, F>
where
    V: TypedValidator,
    F: Fn(V::Output, &V::Input) -> O,
{
    type Input = V::Input;
    type Output = O;
    type Error = V::Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let output = self.validator.validate(input)?;
        Ok((self.mapper)(output, input))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();
        ValidatorMetadata {
            name: format!("MapWithInput({})", inner_meta.name),
            ..inner_meta
        }
    }
}

// ============================================================================
// LAWS AND PROPERTIES
// ============================================================================

#[cfg(test)]
mod laws {
    use super::*;
    use crate::core::{ValidationError, traits::ValidatorExt};

    struct Identity;
    impl TypedValidator for Identity {
        type Input = str;
        type Output = usize;
        type Error = ValidationError;
        fn validate(&self, input: &str) -> Result<usize, ValidationError> {
            Ok(input.len())
        }
    }

    #[test]
    fn test_map_identity() {
        // map(validator, |x| x) === validator
        let validator = Identity;
        let mapped = Map::new(validator, |x| x);

        assert_eq!(
            Identity.validate("test").unwrap(),
            mapped.validate("test").unwrap()
        );
    }

    #[test]
    fn test_map_composition() {
        // map(map(validator, f), g) === map(validator, |x| g(f(x)))
        let f = |x: usize| x * 2;
        let g = |x: usize| x + 1;

        let composed = Map::new(Identity, |x| g(f(x)));
        let chained = Map::new(Map::new(Identity, f), g);

        assert_eq!(
            composed.validate("test").unwrap(),
            chained.validate("test").unwrap()
        );
    }

    #[test]
    fn test_map_preserves_errors() {
        // If validator fails, map doesn't change that
        struct AlwaysFails;
        impl TypedValidator for AlwaysFails {
            type Input = str;
            type Output = ();
            type Error = ValidationError;
            fn validate(&self, _: &str) -> Result<(), ValidationError> {
                Err(ValidationError::new("fail", "Always fails"))
            }
        }

        let mapped = Map::new(AlwaysFails, |_| 42);
        assert!(mapped.validate("test").is_err());
    }
}

// ============================================================================
// STANDARD TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ValidationError, traits::ValidatorExt};

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
    fn test_map_transforms_output() {
        let validator = Map::new(MinLength { min: 5 }, |_| "Valid!");
        assert_eq!(validator.validate("hello").unwrap(), "Valid!");
    }

    #[test]
    fn test_map_preserves_validation() {
        let validator = Map::new(MinLength { min: 5 }, |_| 42);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_map_chain() {
        let validator = MinLength { min: 3 }
            .map(|_| 10)
            .map(|n| n * 2)
            .map(|n| n + 5);

        assert_eq!(validator.validate("hello").unwrap(), 25);
    }

    #[test]
    fn test_map_to() {
        let validator = map_to(MinLength { min: 5 }, "SUCCESS");
        assert_eq!(validator.validate("hello").unwrap(), "SUCCESS");
    }

    #[test]
    fn test_map_unit() {
        let validator = map_unit(MinLength { min: 5 });
        assert_eq!(validator.validate("hello").unwrap(), ());
    }

    #[test]
    fn test_map_with_input() {
        let validator =
            map_with_input(MinLength { min: 3 }, |_, input| format!("Valid: {}", input));

        assert_eq!(validator.validate("hello").unwrap(), "Valid: hello");
    }

    #[test]
    fn test_map_metadata() {
        let validator = Map::new(MinLength { min: 5 }, |_| 42);
        let meta = validator.metadata();

        assert!(meta.name.contains("Map"));
        assert!(meta.tags.contains(&"map".to_string()));
    }

    #[test]
    fn test_into_parts() {
        let min_length = MinLength { min: 5 };
        let mapper = |_: &()| 42;
        let validator = Map::new(min_length, mapper);

        let (extracted_validator, _extracted_mapper) = validator.into_parts();
        assert_eq!(extracted_validator.min, 5);
    }
}
