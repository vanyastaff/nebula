//! EACH combinator - validates each element of a collection

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// EACH COMBINATOR
// ============================================================================

/// Validates each element of a collection.
///
/// Applies a validator to every element in a slice, Vec, or other iterable.
/// Collects all errors with their indices.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Each;
/// use nebula_validator::foundation::Validate;
///
/// let validator = Each::new(MinLength { min: 3 });
///
/// // All elements valid
/// assert!(validator.validate(&["foo", "bar", "baz"]).is_ok());
///
/// // Some elements invalid
/// let result = validator.validate(&["foo", "ab", "x"]);
/// assert!(result.is_err());
/// // Error contains indices: [1, 2]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Each<V> {
    inner: V,
    fail_fast: bool,
}

impl<V> Each<V> {
    /// Creates a new EACH combinator.
    ///
    /// By default, validates all elements and collects all errors.
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            fail_fast: false,
        }
    }

    /// Creates an EACH combinator that stops on first error.
    pub fn fail_fast(inner: V) -> Self {
        Self {
            inner,
            fail_fast: true,
        }
    }

    /// Sets whether to stop on first error.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
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

impl<V, T> Validate for Each<V>
where
    V: Validate<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let mut errors: Vec<(usize, ValidationError)> = Vec::new();

        for (index, element) in input.iter().enumerate() {
            if let Err(e) = self.inner.validate(element) {
                if self.fail_fast {
                    return Err(ValidationError::new(
                        "each_failed",
                        format!("Element at index {} failed: {}", index, e.message),
                    )
                    .with_param("index", index.to_string())
                    .with_nested_error(e));
                }
                errors.push((index, e));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let indices: Vec<String> = errors.iter().map(|(i, _)| i.to_string()).collect();
            let messages: Vec<String> = errors
                .iter()
                .map(|(i, e)| format!("[{}]: {}", i, e.message))
                .collect();

            let mut error = ValidationError::new(
                "each_failed",
                format!(
                    "{} of {} elements failed validation: {}",
                    errors.len(),
                    input.len(),
                    messages.join("; ")
                ),
            )
            .with_param("failed_count", errors.len().to_string())
            .with_param("total_count", input.len().to_string())
            .with_param("failed_indices", indices.join(","));

            // Add nested errors
            for (_, e) in errors {
                error = error.with_nested_error(e);
            }

            Err(error)
        }
    }
}

/// Creates an EACH combinator that validates all elements.
pub fn each<V>(validator: V) -> Each<V> {
    Each::new(validator)
}

/// Creates an EACH combinator that stops on first error.
pub fn each_fail_fast<V>(validator: V) -> Each<V> {
    Each::fail_fast(validator)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    struct Positive;

    impl Validate for Positive {
        type Input = i32;

        fn validate(&self, input: &i32) -> Result<(), ValidationError> {
            if *input > 0 {
                Ok(())
            } else {
                Err(ValidationError::new("positive", "Must be positive"))
            }
        }
    }

    #[test]
    fn test_each_all_valid() {
        let validator = Each::new(Positive);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
    }

    #[test]
    fn test_each_some_invalid() {
        let validator = Each::new(Positive);
        let result = validator.validate(&[1, -2, -3]);
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.message.contains("2 of 3"));
    }

    #[test]
    fn test_each_all_invalid() {
        let validator = Each::new(Positive);
        let result = validator.validate(&[-1, -2, -3]);
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.message.contains("3 of 3"));
    }

    #[test]
    fn test_each_empty() {
        let validator = Each::new(Positive);
        let input: [i32; 0] = [];
        assert!(validator.validate(&input).is_ok());
    }

    #[test]
    fn test_each_fail_fast() {
        let validator = Each::fail_fast(Positive);
        let result = validator.validate(&[1, -2, -3]);
        assert!(result.is_err());

        let error = result.unwrap_err();
        // Should only report first error (index 1)
        assert!(error.message.contains("index 1"));
        assert!(!error.message.contains("index 2"));
    }

    #[test]
    fn test_each_helper_functions() {
        let v1 = each(Positive);
        let v2 = each_fail_fast(Positive);

        assert!(v1.validate(&[1, 2, 3]).is_ok());
        assert!(v2.validate(&[1, 2, 3]).is_ok());
    }
}
