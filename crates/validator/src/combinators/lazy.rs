//! LAZY combinator - deferred validator initialization

use crate::foundation::{Validate, ValidationError};
use std::sync::OnceLock;

// ============================================================================
// LAZY COMBINATOR
// ============================================================================

/// Defers validator creation until first use.
///
/// Useful for:
/// - Expensive validator initialization (e.g., regex compilation)
/// - Breaking circular dependencies
/// - Validators that depend on runtime configuration
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::Lazy;
/// use nebula_validator::foundation::Validate;
///
/// // Validate is created only when first used
/// let validator = Lazy::new(|| {
///     println!("Creating expensive validator...");
///     RegexValidator::new(r"^\d{4}-\d{2}-\d{2}$").unwrap()
/// });
///
/// // First call triggers initialization
/// validator.validate("2024-01-15")?;
///
/// // Subsequent calls use cached validator
/// validator.validate("2024-12-25")?;
/// ```
pub struct Lazy<V, F>
where
    F: Fn() -> V,
{
    init: F,
    validator: OnceLock<V>,
}

impl<V, F> Lazy<V, F>
where
    F: Fn() -> V,
{
    /// Creates a new LAZY combinator.
    ///
    /// The `init` function is called once on first validation.
    pub fn new(init: F) -> Self {
        Self {
            init,
            validator: OnceLock::new(),
        }
    }

    /// Returns a reference to the initialized validator, if any.
    pub fn get(&self) -> Option<&V> {
        self.validator.get()
    }

    /// Returns true if the validator has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.validator.get().is_some()
    }

    /// Forces initialization and returns a reference to the validator.
    pub fn force(&self) -> &V {
        self.validator.get_or_init(&self.init)
    }
}

impl<V, F> Validate for Lazy<V, F>
where
    V: Validate,
    F: Fn() -> V,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let validator = self.validator.get_or_init(&self.init);
        validator.validate(input)
    }
}

// Manual Debug impl since F might not implement Debug
impl<V, F> std::fmt::Debug for Lazy<V, F>
where
    V: std::fmt::Debug,
    F: Fn() -> V,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lazy")
            .field("validator", &self.validator.get())
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

/// Creates a LAZY combinator.
pub fn lazy<V, F>(init: F) -> Lazy<V, F>
where
    F: Fn() -> V,
{
    Lazy::new(init)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = str;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "min_length",
                    format!("Must be at least {} characters", self.min),
                ))
            }
        }
    }

    #[test]
    fn test_lazy_not_initialized_before_use() {
        let init_count = Arc::new(AtomicUsize::new(0));
        let count = init_count.clone();

        let validator = Lazy::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            MinLength { min: 5 }
        });

        assert!(!validator.is_initialized());
        assert_eq!(init_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_lazy_initialized_on_first_validate() {
        let init_count = Arc::new(AtomicUsize::new(0));
        let count = init_count.clone();

        let validator = Lazy::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            MinLength { min: 5 }
        });

        assert!(validator.validate("hello").is_ok());
        assert!(validator.is_initialized());
        assert_eq!(init_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_lazy_only_initialized_once() {
        let init_count = Arc::new(AtomicUsize::new(0));
        let count = init_count.clone();

        let validator = Lazy::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            MinLength { min: 5 }
        });

        // Multiple validations
        validator.validate("hello").unwrap();
        validator.validate("world").unwrap();
        validator.validate("test!").unwrap();

        // Only initialized once
        assert_eq!(init_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_lazy_validation_works() {
        let validator = lazy(|| MinLength { min: 5 });

        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_lazy_force() {
        let validator = lazy(|| MinLength { min: 5 });

        assert!(!validator.is_initialized());

        let inner = validator.force();
        assert_eq!(inner.min, 5);
        assert!(validator.is_initialized());
    }

    #[test]
    fn test_lazy_get() {
        let validator = lazy(|| MinLength { min: 5 });

        assert!(validator.get().is_none());

        validator.validate("hello").unwrap();

        assert!(validator.get().is_some());
        assert_eq!(validator.get().unwrap().min, 5);
    }
}
