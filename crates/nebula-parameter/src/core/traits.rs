//! Core parameter traits

use crate::core::display_stub::{
    DisplayContext, ParameterCondition, ParameterDisplay, ParameterDisplayError,
};
use crate::core::validation::ParameterValidation;
use crate::core::{ParameterError, ParameterKind, ParameterMetadata};
pub use async_trait::async_trait;
use nebula_core::ParameterKey as Key;
pub use nebula_expression::{EvaluationContext, ExpressionEngine, MaybeExpression};
use nebula_value::Value;
use std::fmt::Debug;

// =============================================================================
// Base Trait
// =============================================================================

/// Base trait for all parameter types
///
/// This is the foundation trait that provides core identification
/// and metadata capabilities. All parameter types must implement this trait.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// struct MyParameter {
///     metadata: ParameterMetadata,
/// }
///
/// impl Parameter for MyParameter {
///     fn kind(&self) -> ParameterKind {
///         ParameterKind::Text
///     }
///
///     fn metadata(&self) -> &ParameterMetadata {
///         &self.metadata
///     }
/// }
/// ```
pub trait Parameter: Send + Sync {
    /// Get the kind/type of this parameter
    fn kind(&self) -> ParameterKind;

    /// Get parameter metadata
    fn metadata(&self) -> &ParameterMetadata;

    /// Get parameter key (convenience method)
    #[inline]
    fn key(&self) -> &str {
        self.metadata().key.as_str()
    }

    /// Get parameter name (convenience method)
    #[inline]
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Check if parameter is required (convenience method)
    #[inline]
    fn is_required(&self) -> bool {
        self.metadata().required
    }
}

// =============================================================================
// Value Storage Trait
// =============================================================================

/// Core trait for parameters that can store values
///
/// This trait provides fundamental get/set operations for parameter values.
/// It focuses **only on value storage**, without mixing in expression or
/// validation concerns (those are in separate traits).
///
/// # Implementation Notes
///
/// - For expression support, implement [`Expressible`] trait
/// - For validation logic, implement [`Validatable`] trait  
/// - For display conditions, implement [`Displayable`] trait
/// - For type-erased access, implement [`ParameterValue`] trait
///
/// # Type Parameters
///
/// * `Value` - The concrete value type this parameter stores. Must be cloneable,
///   comparable, debuggable, and thread-safe.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// struct TextParameter {
///     metadata: ParameterMetadata,
///     value: Option<String>,
///     default: Option<String>,
/// }
///
/// impl HasValue for TextParameter {
///     type Value = String;
///
///     fn get(&self) -> Option<&Self::Value> {
///         self.value.as_ref()
///     }
///
///     fn get_mut(&mut self) -> Option<&mut Self::Value> {
///         self.value.as_mut()
///     }
///
///     fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
///         self.value = Some(value);
///         Ok(())
///     }
///
///     fn default(&self) -> Option<&Self::Value> {
///         self.default.as_ref()
///     }
///
///     fn clear(&mut self) {
///         self.value = None;
///     }
/// }
/// ```
#[async_trait]
pub trait HasValue: Parameter + Debug {
    /// The concrete value type for this parameter
    type Value: Clone + PartialEq + Debug + Send + Sync + 'static;

    // --- Required methods (ONLY value storage) ---

    /// Gets the current value (immutable reference)
    fn get(&self) -> Option<&Self::Value>;

    /// Gets the current value (mutable reference)
    ///
    /// # Important Note for Expressible Parameters
    ///
    /// For parameters implementing [`Expressible`], this only returns `Some`
    /// when the value is concrete. If the parameter stores an expression string,
    /// this returns `None` since expressions cannot be mutated directly.
    ///
    /// To modify an expression, use [`Expressible::set_expression`] instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::prelude::*;
    /// # let mut param = TextParameter::new("test");
    /// // Concrete value - can mutate
    /// param.set("hello".to_string()).unwrap();
    /// if let Some(value) = param.get_mut() {
    ///     value.push_str(" world");
    /// }
    ///
    /// // Expression - cannot mutate, returns None
    /// param.set_expression("{{ $input.value }}").unwrap();
    /// assert!(param.get_mut().is_none());
    /// ```
    fn get_mut(&mut self) -> Option<&mut Self::Value>;

    /// Sets a new value without validation
    ///
    /// This is the low-level setter. For validated setting, use
    /// [`set_validated`](Self::set_validated) method (requires [`Validatable`]).
    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError>;

    /// Gets the default value if defined
    fn default(&self) -> Option<&Self::Value>;

    /// Clears the current value
    fn clear(&mut self);

    // --- Convenience methods with default implementations ---

    /// Returns true if parameter has a value set
    #[inline]
    fn has_value(&self) -> bool {
        self.get().is_some()
    }

    /// Sets a new value with validation (requires [`Validatable`])
    ///
    /// This is the high-level setter that includes validation.
    /// The operation is transactional - if validation fails, the old
    /// value is preserved.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails.
    async fn set_validated(&mut self, value: Self::Value) -> Result<(), ParameterError>
    where
        Self: Validatable,
        Self::Value: Clone + Into<Value>,
    {
        // Validate first
        self.validate(&value).await?;

        // Use try_set for transactional behavior
        HasValueExt::try_set(self, value)?;
        Ok(())
    }

    /// Updates the current value in place using a closure
    ///
    /// # Errors
    ///
    /// Returns an error if no value is set or if the closure returns an error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::prelude::*;
    /// # let mut param = TextParameter::new("test");
    /// # param.set("hello".to_string());
    /// param.update(|value| {
    ///     value.push_str(" world");
    ///     Ok(())
    /// })?;
    /// assert_eq!(param.get(), Some(&"hello world".to_string()));
    /// ```
    fn update<F>(&mut self, f: F) -> Result<(), ParameterError>
    where
        F: FnOnce(&mut Self::Value) -> Result<(), ParameterError>,
    {
        match self.get_mut() {
            Some(value) => f(value),
            None => Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            }),
        }
    }
}

// =============================================================================
// Extension Trait for HasValue
// =============================================================================

/// Extension trait providing convenience methods for parameters with values
///
/// This trait is automatically implemented for all types that implement [`HasValue`].
/// It provides additional utility methods without cluttering the core [`HasValue`] trait.
///
/// # Design Rationale
///
/// By separating convenience methods into an extension trait, we:
/// - Keep the core trait focused and minimal
/// - Allow easy addition of new utilities
/// - Enable blanket implementations
/// - Avoid forcing implementations to think about convenience methods
pub trait HasValueExt: HasValue {
    /// Checks if the current value equals the default
    fn is_default(&self) -> bool {
        match (self.get(), self.default()) {
            (Some(current), Some(default)) => current == default,
            (None, None) => true,
            _ => false,
        }
    }

    /// Resets the parameter's value to its default
    ///
    /// If no default is set, clears the value instead.
    fn reset(&mut self) -> Result<(), ParameterError> {
        if let Some(default) = self.default().cloned() {
            self.set(default)
        } else {
            self.clear();
            Ok(())
        }
    }

    /// Takes the current value, leaving the parameter empty
    ///
    /// This is equivalent to `get().cloned()` followed by `clear()`,
    /// but only clears if a value actually exists.
    ///
    /// # Fixed Issue
    ///
    /// Previous implementation would call `clear()` even when no value
    /// was present, which could cause issues with certain parameter types.
    fn take(&mut self) -> Option<Self::Value> {
        let value = self.get().cloned();
        // Only clear if we actually have a value
        if value.is_some() {
            self.clear();
        }
        value
    }

    /// Gets the current value or the default value
    ///
    /// Returns `None` if neither current nor default value is set.
    fn get_or_default(&self) -> Option<&Self::Value> {
        self.get().or_else(|| self.default())
    }

    /// Gets the current value or a provided fallback
    fn get_or<'a>(&'a self, fallback: &'a Self::Value) -> &'a Self::Value {
        self.get().unwrap_or(fallback)
    }

    /// Gets the current value or returns an error
    ///
    /// This is useful when a value is required for an operation.
    fn get_or_err(&self) -> Result<&Self::Value, ParameterError> {
        self.get().ok_or_else(|| ParameterError::MissingValue {
            key: self.metadata().key.clone(),
        })
    }

    /// Gets the current value or computes it lazily
    ///
    /// This is useful when the fallback is expensive to compute.
    fn get_or_else<F>(&self, f: F) -> Self::Value
    where
        F: FnOnce() -> Self::Value,
    {
        self.get().cloned().unwrap_or_else(f)
    }

    /// Maps the current value to another type
    fn map<U, F>(&self, f: F) -> Option<U>
    where
        F: FnOnce(&Self::Value) -> U,
    {
        self.get().map(f)
    }

    /// Sets value by cloning (convenience for owned values)
    ///
    /// This is useful when you have a reference but need to set by value.
    fn set_clone(&mut self, value: &Self::Value) -> Result<(), ParameterError> {
        self.set(value.clone())
    }

    /// Sets value using Into conversion
    ///
    /// This allows setting from any type that can convert into the parameter's value type.
    fn set_into<T>(&mut self, value: T) -> Result<(), ParameterError>
    where
        T: Into<Self::Value>,
    {
        self.set(value.into())
    }

    /// Try to set a value, returning the old value on success
    ///
    /// This is useful when you need to atomically swap values and keep the old one.
    /// The operation is transactional - if setting fails, the old value is restored.
    ///
    /// # Errors
    ///
    /// Returns `ParameterError::InvalidValue` if both the set operation fails
    /// AND the restoration of the old value fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::prelude::*;
    /// # let mut param = TextParameter::new("test");
    /// param.set("hello".to_string()).unwrap();
    ///
    /// let old = param.try_set("world".to_string()).unwrap();
    /// assert_eq!(old, Some("hello".to_string()));
    /// assert_eq!(param.get(), Some(&"world".to_string()));
    /// ```
    fn try_set(&mut self, value: Self::Value) -> Result<Option<Self::Value>, ParameterError> {
        let old = self.take();

        match self.set(value) {
            Ok(()) => Ok(old),
            Err(original_error) => {
                // Try to restore old value
                if let Some(old_val) = old
                    && let Err(_restore_error) = self.set(old_val)
                {
                    // Critical: both operations failed, parameter is poisoned
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata().key.clone(),
                        reason: format!(
                            "Failed to set value and restore old value: {original_error}"
                        ),
                    });
                }
                Err(original_error)
            }
        }
    }
}

// Automatic blanket implementation for all HasValue types
impl<T: HasValue + ?Sized> HasValueExt for T {}

// =============================================================================
// Type-Erased Parameter Value Trait
// =============================================================================

/// Type-erased access to parameter values
///
/// This trait allows working with parameters without knowing their concrete
/// value type. Useful for:
/// - Storing heterogeneous parameters in collections (`Vec<Box<dyn ParameterValue>>`)
/// - UI code that needs to display/edit values generically
/// - Serialization/deserialization of parameter collections
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// let mut params: Vec<Box<dyn ParameterValue>> = vec![
///     Box::new(TextParameter::new("name")),
///     Box::new(NumberParameter::new("age")),
/// ];
///
/// // Type-erased operations
/// for param in &mut params {
///     if !param.has_value_erased() {
///         param.set_erased(Value::String("default".to_string()))?;
///     }
/// }
/// ```
pub trait ParameterValue: Parameter {
    /// Check if parameter has a value (type-erased)
    fn has_value_erased(&self) -> bool;

    /// Clear the value (type-erased)
    fn clear_erased(&mut self);

    /// Get the value as generic `Value` (type-erased)
    fn get_erased(&self) -> Option<Value>;

    /// Set the value from generic `Value` (type-erased)
    ///
    /// # Errors
    ///
    /// Returns `ParameterError::InvalidValue` if the value cannot be
    /// converted to the parameter's concrete type.
    fn set_erased(&mut self, value: Value) -> Result<(), ParameterError>;

    /// Downcast to concrete type
    fn as_any(&self) -> &dyn std::any::Any;

    /// Downcast to concrete type (mutable)
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Validate the current value (type-erased, async)
    ///
    /// Returns a boxed future to allow trait objects
    fn validate_erased(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>;
}

// Blanket implementation for all HasValue types with proper conversion support
impl<T> ParameterValue for T
where
    T: HasValue + Validatable + 'static,
    T::Value: Clone + Into<Value> + TryFrom<Value>,
    <T::Value as TryFrom<Value>>::Error: std::fmt::Display,
{
    fn has_value_erased(&self) -> bool {
        self.has_value()
    }

    fn clear_erased(&mut self) {
        self.clear();
    }

    fn get_erased(&self) -> Option<Value> {
        self.get().cloned().map(Into::into)
    }

    fn set_erased(&mut self, value: Value) -> Result<(), ParameterError> {
        let typed = T::Value::try_from(value).map_err(|e| ParameterError::InvalidValue {
            key: self.metadata().key.clone(),
            reason: format!(
                "Cannot convert Value to {}: {}",
                std::any::type_name::<T::Value>(),
                e
            ),
        })?;

        self.set(typed)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn validate_erased(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>
    {
        Box::pin(self.validate_current())
    }
}

// =============================================================================
// Validation Trait
// =============================================================================

/// Trait for parameters that support validation
///
/// Provides both synchronous (fast) and asynchronous (complex) validation.
/// Most parameters should implement synchronous validation, with async
/// validation reserved for cases requiring I/O (database checks, API calls, etc.).
///
/// # Validation Flow
///
/// When [`validate`](Self::validate) is called, it runs in this order:
/// 1. Synchronous validation ([`validate_sync`](Self::validate_sync))
/// 2. Asynchronous validation ([`validate_async`](Self::validate_async))
/// 3. Custom validation rules (from [`validation`](Self::validation) config)
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// struct NumberParameter {
///     // ... fields ...
///     min: Option<f64>,
///     max: Option<f64>,
/// }
///
/// #[async_trait]
/// impl Validatable for NumberParameter {
///     // Fast synchronous validation
///     fn validate_sync(&self, value: &Self::Value) -> Result<(), ParameterError> {
///         if let Some(min) = self.min {
///             if *value < min {
///                 return Err(ParameterError::InvalidValue {
///                     key: self.metadata().key.clone(),
///                     reason: format!("Value {} below minimum {}", value, min),
///                 });
///             }
///         }
///         if let Some(max) = self.max {
///             if *value > max {
///                 return Err(ParameterError::InvalidValue {
///                     key: self.metadata().key.clone(),
///                     reason: format!("Value {} above maximum {}", value, max),
///                 });
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Validatable: HasValue + Send + Sync {
    /// Synchronous validation (fast, local checks)
    ///
    /// Override this for basic validation like:
    /// - Range checks
    /// - Regex matching
    /// - Required field validation
    /// - Format validation
    ///
    /// Default implementation only checks if required field is empty.
    fn validate_sync(&self, value: &Self::Value) -> Result<(), ParameterError> {
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            });
        }
        Ok(())
    }

    /// Asynchronous validation (slow, I/O-bound checks)
    ///
    /// Override this for validation requiring external resources:
    /// - Database uniqueness checks
    /// - External API validation  
    /// - Rate limiting checks
    /// - Cross-parameter validation with async dependencies
    ///
    /// Default implementation does nothing (returns Ok).
    async fn validate_async(&self, _value: &Self::Value) -> Result<(), ParameterError> {
        Ok(())
    }

    /// Complete validation (runs both sync and async)
    ///
    /// This is the main validation entry point. It runs:
    /// 1. Synchronous validation ([`validate_sync`](Self::validate_sync))
    /// 2. Asynchronous validation ([`validate_async`](Self::validate_async))
    /// 3. Custom validation rules (from [`validation`](Self::validation))
    ///
    /// You typically don't need to override this method.
    ///
    /// # Generic Bounds Note
    ///
    /// The `Clone + Into<Value>` bounds are only required for custom validation rules.
    /// If your parameter doesn't use custom validation configuration, use
    /// [`validate_simple`](Self::validate_simple) instead.
    async fn validate(&self, value: &Self::Value) -> Result<(), ParameterError>
    where
        Self::Value: Clone + Into<Value>,
    {
        // 1. Fast synchronous checks
        self.validate_sync(value)?;

        // 2. Slow asynchronous checks
        self.validate_async(value).await?;

        // 3. Custom validation from configuration
        if let Some(validation) = self.validation() {
            let nebula_value = value.clone().into();
            validation
                .validate(&nebula_value, None)
                .await
                .map_err(|e| ParameterError::InvalidValue {
                    key: self.metadata().key.clone(),
                    reason: format!("{e}"),
                })?;
        }

        Ok(())
    }

    /// Validates without custom validation rules
    ///
    /// This is useful for parameters that don't implement `Clone + Into<Value>`
    /// or don't use custom validation configuration. It only runs
    /// sync and async validation, skipping the custom rules.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::prelude::*;
    /// # let param = SimpleParameter::new("test");
    /// // For simple parameters without custom validation
    /// param.validate_simple(param.get().unwrap()).await?;
    /// ```
    async fn validate_simple(&self, value: &Self::Value) -> Result<(), ParameterError> {
        self.validate_sync(value)?;
        self.validate_async(value).await?;
        Ok(())
    }

    /// Get the validation configuration
    ///
    /// Returns custom validation rules if configured.
    /// Default implementation returns `None` (no custom validation).
    fn validation(&self) -> Option<&ParameterValidation> {
        None
    }

    /// Check if a value is considered empty
    ///
    /// # ⚠️ CRITICAL for String/Collection Parameters
    ///
    /// **You MUST override this method** for parameters storing:
    /// - Strings (empty string `""`)
    /// - Vectors/Arrays (empty collection `[]`)
    /// - Maps/Objects (empty map `{}`)
    ///
    /// If not overridden, `is_empty` always returns `false`, which means
    /// **empty strings/collections will pass required field validation!**
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::prelude::*;
    /// impl Validatable for TextParameter {
    ///     fn is_empty(&self, value: &String) -> bool {
    ///         value.is_empty()  // ✅ REQUIRED
    ///     }
    /// }
    ///
    /// impl Validatable for ArrayParameter {
    ///     fn is_empty(&self, value: &Vec<Value>) -> bool {
    ///         value.is_empty()  // ✅ REQUIRED
    ///     }
    /// }
    /// ```
    fn is_empty(&self, _value: &Self::Value) -> bool {
        false // Default: most types don't have "empty" concept
    }

    /// Validates the current value of the parameter
    ///
    /// Convenience method that validates whatever value is currently set.
    /// Returns an error if the parameter is required but has no value.
    async fn validate_current(&self) -> Result<(), ParameterError>
    where
        Self::Value: Clone + Into<Value>,
    {
        match self.get() {
            Some(value) => self.validate(value).await,
            None if self.is_required() => Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            }),
            None => Ok(()),
        }
    }
}

// =============================================================================
// Expression Evaluation Trait
// =============================================================================

/// Trait for parameters that can store and evaluate expressions
///
/// This trait extends [`HasValue`] to support dynamic expression evaluation.
/// Parameters implementing this trait can store expressions like
/// `{{ $node.data.value * 2 }}` and evaluate them at runtime.
///
/// # Expression Storage
///
/// Parameters can store values in two forms:
/// - **Concrete values**: `MaybeExpression::Value(v)`
/// - **Expression strings**: `MaybeExpression::Expression("{{ ... }}")`
///
/// # Async Note
///
/// The `evaluate` method is async by design to support future expression engines
/// that may perform I/O (e.g., fetching data from external sources).
/// Current implementations may be synchronous (fast-path), but the async
/// contract ensures forward compatibility.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// // Create parameter with expression
/// let mut param = TextParameter::new("user_name");
/// param.set_expression("{{ $input.user.name }}").unwrap();
///
/// assert!(param.is_expression());
///
/// // Evaluate at runtime
/// let engine = ExpressionEngine::new();
/// let mut context = EvaluationContext::new();
/// context.set_input(json!({"user": {"name": "Alice"}}));
///
/// let value = param.resolve(&engine, &context).await.unwrap();
/// assert_eq!(value, "Alice");
/// ```
#[async_trait::async_trait]
pub trait Expressible: HasValue {
    // --- Required methods ---

    /// Converts parameter value to generic `MaybeExpression<Value>`
    ///
    /// This allows uniform access to parameter values regardless of whether
    /// they're expressions or concrete values.
    fn to_expression(&self) -> Option<MaybeExpression<Value>>;

    /// Gets the expression string without allocation (if available)
    ///
    /// This is more efficient than `get_expression()` when you just need
    /// to read the expression without owning it.
    ///
    /// Returns `None` if the parameter stores a concrete value or has no value.
    fn expression_ref(&self) -> Option<&str> {
        None // Default: parameter doesn't store expressions
    }

    /// Sets parameter value from generic `MaybeExpression<Value>`
    ///
    /// Accepts either concrete values or expression strings.
    /// The parameter implementation decides how to handle expressions.
    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError>;

    // --- Convenience methods with default implementations ---

    /// Check if the current value is an expression
    fn is_expression(&self) -> bool {
        matches!(self.to_expression(), Some(MaybeExpression::Expression(_)))
    }

    /// Get the raw expression string (allocating)
    ///
    /// For better performance, use [`expression_ref`](Self::expression_ref) instead.
    ///
    /// Returns `None` if the parameter stores a concrete value or has no value.
    fn get_expression(&self) -> Option<String> {
        self.expression_ref().map(std::string::ToString::to_string)
    }

    /// Set an expression string
    ///
    /// This is a convenience method that wraps the expression in `MaybeExpression::Expression`.
    fn set_expression(&mut self, expr: impl Into<String>) -> Result<(), ParameterError> {
        use nebula_expression::CachedExpression;
        use once_cell::sync::OnceCell;

        let cached_expr = CachedExpression {
            source: expr.into(),
            ast: OnceCell::new(),
        };
        self.from_expression(MaybeExpression::Expression(cached_expr))
    }

    /// Evaluate the expression and return the result as `Value`
    ///
    /// If the parameter stores a concrete value, returns it directly.
    /// If it stores an expression, evaluates it using the provided engine and context.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Expression evaluation fails
    /// - No value is set
    async fn evaluate(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Value, ParameterError> {
        match self.to_expression() {
            Some(MaybeExpression::Expression(expr)) => engine
                .evaluate(&expr.source, context)
                .map_err(|e| ParameterError::InvalidValue {
                    key: self.metadata().key.clone(),
                    reason: format!("Expression evaluation failed: {e}"),
                }),
            Some(MaybeExpression::Value(v)) => Ok(v),
            None => Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            }),
        }
    }

    /// Resolve the parameter value - evaluate if expression, return value otherwise
    ///
    /// This is the main method to use when you need the actual typed value.
    /// It handles both static values and expressions transparently.
    ///
    /// # Type Conversion
    ///
    /// The result is automatically converted from `Value` to the parameter's
    /// concrete type using `TryFrom`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Expression evaluation fails
    /// - Type conversion fails
    /// - No value is set
    async fn resolve(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Self::Value, ParameterError>
    where
        Self::Value: TryFrom<Value>,
        <Self::Value as TryFrom<Value>>::Error: std::fmt::Display,
    {
        let value = self.evaluate(engine, context).await?;

        value.try_into().map_err(|e| ParameterError::InvalidValue {
            key: self.metadata().key.clone(),
            reason: format!(
                "Cannot convert Value to {}: {}",
                std::any::type_name::<Self::Value>(),
                e
            ),
        })
    }

    /*
    /// Resolve and replace expression with its evaluated result
    ///
    /// Evaluates the expression once and stores the result as a concrete value.
    /// Subsequent calls to `get()` will return the cached value without re-evaluation.
    ///
    /// # Important Note
    ///
    /// This method **replaces the expression with its evaluated result**.
    /// After calling this, `is_expression()` will return `false` because
    /// the expression has been converted to a concrete value.
    ///
    /// If you need to preserve the expression, use [`resolve`](Self::resolve)
    /// instead and cache the result yourself.
    ///
    /// # Use Cases
    ///
    /// This is useful when:
    /// - The same expression will be accessed multiple times
    /// - Expression evaluation is expensive
    /// - The context doesn't change between accesses
    /// - You want to "freeze" the expression result
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation or caching fails.
    ///
    /// # Note
    /// This method is commented out due to lifetime/borrow checker issues.
    /// Use `resolve()` instead and then call `set()` manually if needed.
    async fn resolve_and_cache(
        &mut self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<&Self::Value, ParameterError>
    where
        Self::Value: TryFrom<Value>,
        <Self::Value as TryFrom<Value>>::Error: std::fmt::Display,
    {
        // If already a concrete value, return it
        if !self.is_expression() {
            if let Some(v) = self.get() {
                return Ok(v);
            }
        }

        // Evaluate and cache
        let resolved = self.resolve(engine, context).await?;
        self.set(resolved)?;

        // Return the just-set value
        // Safe to unwrap because we just set it
        Ok(self.get().expect("value was just set but is missing"))
    }
    */
}

// =============================================================================
// Display Trait
// =============================================================================

/// Trait for parameters with conditional display logic
///
/// Controls when and how parameters are shown in the UI based on
/// the values of other parameters.
///
/// # Use Cases
///
/// - Show API key field only when auth type is "`api_key`"
/// - Hide advanced options unless "advanced mode" is enabled
/// - Display region-specific fields based on selected region
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// let mut param = TextParameter::new("apiKey");
///
/// // Show only when authentication type is "api_key"
/// param.add_condition(
///     "authType".into(),
///     ParameterCondition::equals("api_key")
/// );
///
/// let context = DisplayContext::new()
///     .with_value("authType", "api_key");
///
/// assert!(param.should_display(&context));
/// ```
pub trait Displayable: Parameter {
    /// Get the display configuration
    fn display(&self) -> Option<&ParameterDisplay>;

    /// Update display conditions
    fn set_display(&mut self, display: Option<ParameterDisplay>);

    /// Check if the parameter should be displayed given the current context
    fn should_display(&self, context: &DisplayContext) -> bool {
        match self.display() {
            Some(display_config) => display_config.should_display(&context.values),
            None => true,
        }
    }

    /// Validate display conditions and return detailed error if hidden
    ///
    /// This is useful for providing user-friendly error messages when
    /// trying to access a hidden parameter.
    fn validate_display(&self, context: &DisplayContext) -> Result<(), ParameterDisplayError> {
        match self.display() {
            Some(display_config) => display_config.validate_display(context),
            None => Ok(()),
        }
    }

    /// Check if this parameter has any display conditions
    fn has_conditions(&self) -> bool {
        match self.display() {
            Some(display_config) => !display_config.is_empty(),
            None => false,
        }
    }

    /// Get all property keys that this parameter's display depends on
    ///
    /// This is useful for building dependency graphs and determining
    /// which parameters need to be re-evaluated when values change.
    fn dependencies(&self) -> Vec<Key> {
        match self.display() {
            Some(display_config) => display_config.get_dependencies(),
            None => Vec::new(),
        }
    }
}

/// Extension trait for mutable display operations
///
/// Separated from [`Displayable`] to keep the core trait immutable-friendly.
/// This is automatically implemented for all `Displayable` types.
pub trait DisplayableMut: Displayable {
    /// Add a display condition
    ///
    /// Multiple conditions can be added for the same or different properties.
    /// All conditions must be satisfied for the parameter to be displayed.
    fn add_condition(&mut self, property: Key, condition: ParameterCondition) {
        let mut display = self.display().cloned().unwrap_or_default();
        display.add_show_condition(property, condition);
        self.set_display(Some(display));
    }

    /// Clear all display conditions
    ///
    /// After calling this, the parameter will always be displayed.
    fn clear_conditions(&mut self) {
        self.set_display(None);
    }
}

// Automatic blanket implementation
impl<T: Displayable + ?Sized> DisplayableMut for T {}

/// Trait for reactive display behavior (optional)
///
/// Implement this trait if your parameter needs to react to visibility changes.
/// This is rarely needed - only for special cases like:
/// - Clearing sensitive data when hidden (security)
/// - Triggering validation when shown
/// - Loading dynamic options when displayed
/// - Resetting to defaults when hidden
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// struct SensitiveParameter {
///     // ... fields ...
/// }
///
/// impl DisplayableReactive for SensitiveParameter {
///     fn on_hide(&mut self, _context: &DisplayContext) {
///         // Clear sensitive data when parameter is hidden
///         self.clear();
///     }
///
///     fn on_show(&mut self, _context: &DisplayContext) {
///         // Maybe load fresh data or validate
///     }
/// }
/// ```
pub trait DisplayableReactive: Displayable {
    /// Called when parameter becomes visible
    fn on_show(&mut self, context: &DisplayContext);

    /// Called when parameter becomes hidden
    fn on_hide(&mut self, context: &DisplayContext);

    /// Called when display state changes
    ///
    /// This is a convenience method that dispatches to `on_show` or `on_hide`.
    /// You typically don't need to override this.
    fn on_display_change(
        &mut self,
        old_visible: bool,
        new_visible: bool,
        context: &DisplayContext,
    ) {
        match (old_visible, new_visible) {
            (false, true) => self.on_show(context),
            (true, false) => self.on_hide(context),
            _ => {} // No change
        }
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Apply display visibility change with lifecycle hooks
///
/// This is a helper function that properly calls `on_display_change`
/// when visibility changes between contexts.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// let mut param = SensitiveParameter::new("secret");
/// let old_ctx = DisplayContext::new().with_value("show_secrets", false);
/// let new_ctx = DisplayContext::new().with_value("show_secrets", true);
///
/// apply_display_change(&mut param, &old_ctx, &new_ctx);
/// // This will call param.on_show() since visibility changed from false to true
/// ```
pub fn apply_display_change<P>(
    param: &mut P,
    old_context: &DisplayContext,
    new_context: &DisplayContext,
) where
    P: DisplayableReactive,
{
    let old_visible = param.should_display(old_context);
    let new_visible = param.should_display(new_context);

    if old_visible != new_visible {
        param.on_display_change(old_visible, new_visible, new_context);
    }
}

// =============================================================================
// Testing Utilities
// =============================================================================

#[cfg(test)]
pub mod testing {
    use super::*;

    /// Assert that a value is valid for a parameter
    pub async fn assert_valid<P>(param: &P, value: &P::Value)
    where
        P: Validatable,
        P::Value: Clone + Into<Value>,
    {
        param.validate(value).await.expect("validation should pass");
    }

    /// Assert that a value is invalid for a parameter
    pub async fn assert_invalid<P>(param: &P, value: &P::Value)
    where
        P: Validatable,
        P::Value: Clone + Into<Value>,
    {
        assert!(
            param.validate(value).await.is_err(),
            "validation should fail but passed"
        );
    }

    /// Assert that validation fails with a specific error type
    pub async fn assert_invalid_with<P, F>(param: &P, value: &P::Value, check: F)
    where
        P: Validatable,
        P::Value: Clone + Into<Value>,
        F: FnOnce(&ParameterError) -> bool,
    {
        match param.validate(value).await {
            Err(e) if check(&e) => {}
            Err(e) => panic!("validation failed with wrong error: {:?}", e),
            Ok(_) => panic!("validation should fail but passed"),
        }
    }

    /// Create a test context for display
    pub fn test_display_context() -> DisplayContext {
        DisplayContext::new()
    }
}
