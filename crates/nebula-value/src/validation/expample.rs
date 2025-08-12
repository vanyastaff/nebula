pub trait Validator<T>: Send + Sync {
    /// Validates the given value and returns a result.
    ///
    /// Returns `Ok(())` if the value is valid, or a `ValidationError`
    /// if validation fails.
    fn validate(&self, value: &T) -> ValidationResult;

    /// Returns the name of this validator for debugging and error messages.
    ///
    /// The default implementation uses the type name.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Returns a human-readable description of what this validator checks.
    ///
    /// This is used for documentation and error messages.
    fn description(&self) -> Option<&'static str> {
        None
    }

    /// Combines this validator with another using logical AND.
    ///
    /// The resulting validator passes only if both validators pass.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_value::validation::Validator;
    ///
    /// let validator = MinLength::<3>.and(MaxLength::<10>);
    /// ```
    fn and<V>(self, other: V) -> And<Self, V>
    where
        Self: Sized,
        V: Validator<T>,
    {
        And::new(self, other)
    }

    /// Combines this validator with another using logical OR.
    ///
    /// The resulting validator passes if either validator passes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_value::validation::Validator;
    ///
    /// let validator = Email.or(PhoneNumber);
    /// ```
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        Self: Sized,
        V: Validator<T>,
    {
        Or::new(self, other)
    }

    /// Creates a validator that passes when this validator fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_value::validation::Validator;
    ///
    /// let validator = Empty.not(); // Passes when value is not empty
    /// ```
    fn not(self) -> Not<Self>
    where
        Self: Sized,
    {
        Not::new(self)
    }

    /// Creates an optional validator that always passes for None values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_value::validation::Validator;
    ///
    /// let validator = Email.optional(); // Passes for None, validates Some values
    /// ```
    fn optional(self) -> Optional<Self, T>
    where
        Self: Sized,
    {
        Optional::new(self)
    }
}

/// A trait for types that can be validated.
///
/// This trait is automatically implemented for all types that have validators,
/// but can also be implemented directly for types that have built-in validation logic.
pub trait Validatable {
    /// Validates this value and returns a result.
    fn validate(&self) -> ValidationResult;

    /// Returns whether this value is valid.
    fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

/// A validator that combines two validators with logical AND.
///
/// Both validators must pass for the combined validator to pass.
#[derive(Debug, Clone)]
pub struct And<V1, V2> {
    first: V1,
    second: V2,
}

impl<V1, V2> And<V1, V2> {
    /// Creates a new AND validator.
    pub fn new(first: V1, second: V2) -> Self {
        Self { first, second }
    }
}

impl<T, V1, V2> Validator<T> for And<V1, V2>
where
    V1: Validator<T>,
    V2: Validator<T>,
{
    fn validate(&self, value: &T) -> ValidationResult {
        // Validate with first validator
        self.first.validate(value)?;
        // If first passes, validate with second
        self.second.validate(value)
    }

    fn name(&self) -> &'static str {
        "And"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Combines two validators with logical AND")
    }
}

/// A validator that combines two validators with logical OR.
///
/// Either validator can pass for the combined validator to pass.
#[derive(Debug, Clone)]
pub struct Or<V1, V2> {
    first: V1,
    second: V2,
}

impl<V1, V2> Or<V1, V2> {
    /// Creates a new OR validator.
    pub fn new(first: V1, second: V2) -> Self {
        Self { first, second }
    }
}

impl<T, V1, V2> Validator<T> for Or<V1, V2>
where
    V1: Validator<T>,
    V2: Validator<T>,
{
    fn validate(&self, value: &T) -> ValidationResult {
        // Try first validator
        if self.first.validate(value).is_ok() {
            return Ok(());
        }
        // If first fails, try second
        self.second.validate(value)
    }

    fn name(&self) -> &'static str {
        "Or"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Combines two validators with logical OR")
    }
}

/// A validator that negates another validator.
///
/// Passes when the inner validator fails, and fails when it passes.
#[derive(Debug, Clone)]
pub struct Not<V> {
    inner: V,
}

impl<V> Not<V> {
    /// Creates a new NOT validator.
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<T, V> Validator<T> for Not<V>
where
    V: Validator<T>,
{
    fn validate(&self, value: &T) -> ValidationResult {
        match self.inner.validate(value) {
            Ok(()) => Err(ValidationError::new(
                "not_failed",
                "Value should not pass the inner validator",
            )),
            Err(_) => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "Not"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Negates another validator")
    }
}

/// A validator that makes another validator optional.
///
/// Always passes for None values, validates Some values with the inner validator.
#[derive(Debug, Clone)]
pub struct Optional<V, T> {
    inner: V,
    _phantom: PhantomData<T>,
}

impl<V, T> Optional<V, T> {
    /// Creates a new optional validator.
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }
}

impl<T, V> Validator<Option<T>> for Optional<V, T>
where
    V: Validator<T>,
{
    fn validate(&self, value: &Option<T>) -> ValidationResult {
        match value {
            None => Ok(()),
            Some(inner_value) => self.inner.validate(inner_value),
        }
    }

    fn name(&self) -> &'static str {
        "Optional"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Makes another validator optional")
    }
}

/// A validator that always passes.
///
/// This is useful as a base case or for testing.
#[derive(Debug, Clone, Copy)]
pub struct Always;

impl<T> Validator<T> for Always {
    fn validate(&self, _value: &T) -> ValidationResult {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Always"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Always passes validation")
    }
}

/// A validator that never passes.
///
/// This is useful for testing or as a placeholder.
#[derive(Debug, Clone, Copy)]
pub struct Never {
    message: &'static str,
}

impl Never {
    /// Creates a new Never validator with a custom message.
    pub fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl Default for Never {
    fn default() -> Self {
        Self::new("Validation always fails")
    }
}

impl<T> Validator<T> for Never {
    fn validate(&self, _value: &T) -> ValidationResult {
        Err(ValidationError::new("never", self.message))
    }

    fn name(&self) -> &'static str {
        "Never"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Never passes validation")
    }
}

/// A validator that applies a function to validate values.
///
/// This allows creating validators from closures or functions.
#[derive(Clone)]
pub struct Function<F> {
    func: F,
    name: &'static str,
}

impl<F> Function<F> {
    /// Creates a new function validator.
    pub fn new(func: F, name: &'static str) -> Self {
        Self { func, name }
    }
}

impl<F> fmt::Debug for Function<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .finish()
    }
}

impl<T, F> Validator<T> for Function<F>
where
    F: Fn(&T) -> ValidationResult + Send + Sync,
{
    fn validate(&self, value: &T) -> ValidationResult {
        (self.func)(value)
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> Option<&'static str> {
        Some("Function-based validator")
    }
}

/// Creates a validator from a function or closure.
///
/// # Examples
///
/// ```rust
/// use nebula_value::validation::{validator, ValidationError};
///
/// let positive = validator(|n: &i32| {
///     if *n > 0 {
///         Ok(())
///     } else {
///         Err(ValidationError::new("not_positive", "Number must be positive"))
///     }
/// }, "positive");
///
/// assert!(positive.validate(&5).is_ok());
/// assert!(positive.validate(&-1).is_err());
/// ```
pub fn validator<T, F>(func: F, name: &'static str) -> Function<F>
where
    F: Fn(&T) -> ValidationResult + Send + Sync,
{
    Function::new(func, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test validator implementations
    #[derive(Debug, Clone, Copy)]
    struct MinLength<const N: usize>;

    impl<const N: usize> Validator<String> for MinLength<N> {
        fn validate(&self, value: &String) -> ValidationResult {
            if value.len() < N {
                Err(ValidationError::new(
                    "min_length",
                    format!("Value must be at least {} characters", N),
                )
                    .with_param("min", N.to_string())
                    .with_param("actual", value.len().to_string()))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct MaxLength<const N: usize>;

    impl<const N: usize> Validator<String> for MaxLength<N> {
        fn validate(&self, value: &String) -> ValidationResult {
            if value.len() > N {
                Err(ValidationError::new(
                    "max_length",
                    format!("Value must be at most {} characters", N),
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn test_basic_validator() {
        let validator = MinLength::<5>;

        assert!(validator.validate(&"hello".to_string()).is_ok());
        assert!(validator.validate(&"hi".to_string()).is_err());
    }

    #[test]
    fn test_and_combinator() {
        let validator = MinLength::<3>.and(MaxLength::<10>);

        assert!(validator.validate(&"hello".to_string()).is_ok());
        assert!(validator.validate(&"hi".to_string()).is_err()); // Too short
        assert!(validator.validate(&"this is too long".to_string()).is_err()); // Too long
    }

    #[test]
    fn test_or_combinator() {
        let validator = MinLength::<10>.or(MaxLength::<3>);

        assert!(validator.validate(&"hi".to_string()).is_ok()); // Short enough
        assert!(validator.validate(&"this is long enough".to_string()).is_ok()); // Long enough
        assert!(validator.validate(&"medium".to_string()).is_err()); // Neither short nor long
    }

    #[test]
    fn test_not_combinator() {
        let validator = MinLength::<5>.not();

        assert!(validator.validate(&"hi".to_string()).is_ok()); // Short, so NOT(min_length) passes
        assert!(validator.validate(&"hello".to_string()).is_err()); // Long, so NOT(min_length) fails
    }

    #[test]
    fn test_optional_validator() {
        let validator = MinLength::<5>.optional();

        assert!(validator.validate(&None).is_ok()); // None always passes
        assert!(validator.validate(&Some("hello".to_string())).is_ok()); // Valid Some
        assert!(validator.validate(&Some("hi".to_string())).is_err()); // Invalid Some
    }

    #[test]
    fn test_always_validator() {
        let validator = Always;

        assert!(validator.validate(&"anything".to_string()).is_ok());
        assert!(validator.validate(&42).is_ok());
    }

    #[test]
    fn test_never_validator() {
        let validator = Never::default();

        assert!(validator.validate(&"anything".to_string()).is_err());
        assert!(validator.validate(&42).is_err());
    }

    #[test]
    fn test_function_validator() {
        let positive = validator(
            |n: &i32| {
                if *n > 0 {
                    Ok(())
                } else {
                    Err(ValidationError::new("not_positive", "Number must be positive"))
                }
            },
            "positive",
        );

        assert!(positive.validate(&5).is_ok());
        assert!(positive.validate(&-1).is_err());
        assert!(positive.validate(&0).is_err());
    }

    #[test]
    fn test_validator_names() {
        assert_eq!(Always.name(), "Always");
        assert_eq!(Never::default().name(), "Never");
        assert_eq!(MinLength::<5>.and(MaxLength::<10>).name(), "And");
        assert_eq!(MinLength::<5>.or(MaxLength::<10>).name(), "Or");
        assert_eq!(MinLength::<5>.not().name(), "Not");
    }
}