//! Validation Context for cross-field validation
//!
//! This module provides a context system that allows validators to access
//! multiple fields and perform cross-field validation logic.
//!
//! # Use Cases
//!
//! - Password confirmation matching
//! - Date range validation (start < end)
//! - Conditional requirements (if A then B required)
//! - Complex business rules involving multiple fields
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::core::{ValidationContext, ContextualValidator};
//!
//! struct PasswordConfirmation;
//!
//! impl ContextualValidator for PasswordConfirmation {
//!     type Input = User;
//!     type Output = ();
//!     type Error = ValidationError;
//!
//!     fn validate_with_context(
//!         &self,
//!         input: &User,
//!         ctx: &ValidationContext
//!     ) -> Result<(), ValidationError> {
//!         if input.password != input.password_confirmation {
//!             return Err(ValidationError::new(
//!                 "password_mismatch",
//!                 "Passwords do not match"
//!             ));
//!         }
//!         Ok(())
//!     }
//! }
//! ```

use crate::core::{TypedValidator, ValidatorMetadata};
use std::any::Any;
use std::collections::HashMap;

// ============================================================================
// VALIDATION CONTEXT
// ============================================================================

/// Context for validation operations.
///
/// Provides access to additional data during validation, enabling
/// cross-field validation and complex business rules.
#[derive(Debug, Default)]
pub struct ValidationContext {
    /// Named values accessible during validation.
    data: HashMap<String, Box<dyn Any + Send + Sync>>,

    /// Parent context for nested validation.
    parent: Option<Box<ValidationContext>>,

    /// Current field path for nested object validation.
    field_path: Vec<String>,
}

impl ValidationContext {
    /// Creates a new empty validation context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a context with a parent.
    #[must_use]
    pub fn with_parent(parent: ValidationContext) -> Self {
        Self {
            data: HashMap::new(),
            parent: Some(Box::new(parent)),
            field_path: Vec::new(),
        }
    }

    /// Adds a value to the context.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::core::ValidationContext;
    ///
    /// let mut ctx = ValidationContext::new();
    /// ctx.insert("max_length", 100usize);
    /// ```
    pub fn insert<T: Send + Sync + 'static>(&mut self, key: impl Into<String>, value: T) {
        self.data.insert(key.into(), Box::new(value));
    }

    /// Gets a value from the context.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::core::ValidationContext;
    ///
    /// let mut ctx = ValidationContext::new();
    /// ctx.insert("max_length", 100usize);
    ///
    /// let max: Option<&usize> = ctx.get("max_length");
    /// assert_eq!(max, Some(&100));
    /// ```
    #[must_use]
    pub fn get<T: 'static>(&self, key: &str) -> Option<&T> {
        // Try local data first
        if let Some(value) = self.data.get(key) {
            return value.downcast_ref::<T>();
        }

        // Try parent context
        if let Some(parent) = &self.parent {
            return parent.get(key);
        }

        None
    }

    /// Gets a mutable value from the context.
    pub fn get_mut<T: 'static>(&mut self, key: &str) -> Option<&mut T> {
        self.data
            .get_mut(key)
            .and_then(|value| value.downcast_mut::<T>())
    }

    /// Checks if a key exists in the context.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.data.contains_key(key) || self.parent.as_ref().is_some_and(|p| p.contains(key))
    }

    /// Pushes a field name onto the path for nested validation.
    ///
    /// This is useful for tracking the full path in nested object validation.
    pub fn push_field(&mut self, field: impl Into<String>) {
        self.field_path.push(field.into());
    }

    /// Pops a field name from the path.
    pub fn pop_field(&mut self) -> Option<String> {
        self.field_path.pop()
    }

    /// Gets the current field path as a dot-separated string.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::core::ValidationContext;
    ///
    /// let mut ctx = ValidationContext::new();
    /// ctx.push_field("user");
    /// ctx.push_field("address");
    /// ctx.push_field("zipcode");
    ///
    /// assert_eq!(ctx.field_path(), "user.address.zipcode");
    /// ```
    #[must_use]
    pub fn field_path(&self) -> String {
        self.field_path.join(".")
    }

    /// Clears the current field path.
    pub fn clear_path(&mut self) {
        self.field_path.clear();
    }

    /// Creates a child context with this context as parent.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            data: HashMap::new(),
            parent: Some(Box::new(self.clone())),
            field_path: self.field_path.clone(),
        }
    }

    /// Returns the number of items in the local context (excluding parent).
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Checks if the local context is empty (excluding parent).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Clone for ValidationContext {
    fn clone(&self) -> Self {
        // Note: We can't clone Box<dyn Any>, so we create a new context
        // with the same parent and field path
        Self {
            data: HashMap::new(),
            parent: self.parent.clone(),
            field_path: self.field_path.clone(),
        }
    }
}

// ============================================================================
// CONTEXTUAL VALIDATOR TRAIT
// ============================================================================

/// Trait for validators that need access to validation context.
///
/// This enables cross-field validation and complex business rules.
///
/// # Examples
///
/// ```rust,ignore
/// struct DateRangeValidator;
///
/// impl ContextualValidator for DateRangeValidator {
///     type Input = DateRange;
///     type Output = ();
///     type Error = ValidationError;
///
///     fn validate_with_context(
///         &self,
///         input: &DateRange,
///         ctx: &ValidationContext,
///     ) -> Result<(), ValidationError> {
///         if input.start >= input.end {
///             return Err(ValidationError::new(
///                 "invalid_date_range",
///                 "Start date must be before end date"
///             ));
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait ContextualValidator {
    /// The type of input being validated.
    type Input: ?Sized;

    /// The type returned on successful validation.
    type Output;

    /// The error type returned on validation failure.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Validates the input with access to the validation context.
    fn validate_with_context(
        &self,
        input: &Self::Input,
        ctx: &ValidationContext,
    ) -> Result<Self::Output, Self::Error>;

    /// Returns metadata about this validator.
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::default()
    }
}

// ============================================================================
// ADAPTER: TypedValidator -> ContextualValidator
// ============================================================================

/// Adapter that allows any `TypedValidator` to be used as a `ContextualValidator`.
///
/// This provides backward compatibility - existing validators can be used
/// in contexts that expect `ContextualValidator`.
pub struct ContextAdapter<V> {
    validator: V,
}

impl<V> ContextAdapter<V> {
    /// Creates a new context adapter.
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Consumes the adapter and returns the inner validator.
    pub fn into_inner(self) -> V {
        self.validator
    }
}

impl<V> ContextualValidator for ContextAdapter<V>
where
    V: TypedValidator,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = V::Error;

    fn validate_with_context(
        &self,
        input: &Self::Input,
        _ctx: &ValidationContext,
    ) -> Result<Self::Output, Self::Error> {
        self.validator.validate(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.validator.metadata()
    }
}

// ============================================================================
// BUILDER FOR CONTEXT
// ============================================================================

/// Builder for creating `ValidationContext` with fluent API.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::core::ValidationContextBuilder;
///
/// let ctx = ValidationContextBuilder::new()
///     .with("max_items", 100usize)
///     .with("min_length", 5usize)
///     .build();
/// ```
pub struct ValidationContextBuilder {
    context: ValidationContext,
}

impl ValidationContextBuilder {
    /// Creates a new context builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            context: ValidationContext::new(),
        }
    }

    /// Adds a value to the context being built.
    #[must_use = "builder methods must be chained or built"]
    pub fn with<T: Send + Sync + 'static>(mut self, key: impl Into<String>, value: T) -> Self {
        self.context.insert(key, value);
        self
    }

    /// Sets the parent context.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_parent(mut self, parent: ValidationContext) -> Self {
        self.context.parent = Some(Box::new(parent));
        self
    }

    /// Builds the validation context.
    #[must_use]
    pub fn build(self) -> ValidationContext {
        self.context
    }
}

impl Default for ValidationContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ValidationError;

    #[test]
    fn test_context_insert_get() {
        let mut ctx = ValidationContext::new();
        ctx.insert("key", 42usize);

        assert_eq!(ctx.get::<usize>("key"), Some(&42));
        assert_eq!(ctx.get::<String>("key"), None); // Wrong type
        assert_eq!(ctx.get::<usize>("missing"), None);
    }

    #[test]
    fn test_context_contains() {
        let mut ctx = ValidationContext::new();
        ctx.insert("key", 42usize);

        assert!(ctx.contains("key"));
        assert!(!ctx.contains("missing"));
    }

    #[test]
    fn test_context_field_path() {
        let mut ctx = ValidationContext::new();
        ctx.push_field("user");
        ctx.push_field("address");
        ctx.push_field("zipcode");

        assert_eq!(ctx.field_path(), "user.address.zipcode");

        ctx.pop_field();
        assert_eq!(ctx.field_path(), "user.address");

        ctx.clear_path();
        assert_eq!(ctx.field_path(), "");
    }

    #[test]
    fn test_context_parent() {
        let mut parent = ValidationContext::new();
        parent.insert("parent_key", 100usize);

        let mut child = ValidationContext::with_parent(parent);
        child.insert("child_key", 200usize);

        assert_eq!(child.get::<usize>("child_key"), Some(&200));
        assert_eq!(child.get::<usize>("parent_key"), Some(&100));
    }

    #[test]
    fn test_context_builder() {
        let ctx = ValidationContextBuilder::new()
            .with("max", 100usize)
            .with("min", 5usize)
            .build();

        assert_eq!(ctx.get::<usize>("max"), Some(&100));
        assert_eq!(ctx.get::<usize>("min"), Some(&5));
    }

    #[test]
    fn test_context_len_empty() {
        let mut ctx = ValidationContext::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);

        ctx.insert("key", 42);
        assert!(!ctx.is_empty());
        assert_eq!(ctx.len(), 1);
    }

    struct TestValidator;

    impl TypedValidator for TestValidator {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.is_empty() {
                Err(ValidationError::new("empty", "String is empty"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn test_context_adapter() {
        let validator = ContextAdapter::new(TestValidator);
        let ctx = ValidationContext::new();

        assert!(validator.validate_with_context("hello", &ctx).is_ok());
        assert!(validator.validate_with_context("", &ctx).is_err());
    }
}
