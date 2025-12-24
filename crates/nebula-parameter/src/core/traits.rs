//! Core parameter traits

use crate::core::display::{DisplayContext, ParameterDisplay};
use crate::core::validation::ParameterValidation;
use crate::core::{ParameterError, ParameterKind, ParameterMetadata};
pub use async_trait::async_trait;
use downcast_rs::{Downcast, impl_downcast};
use nebula_core::ParameterKey as Key;
pub use nebula_expression::{EvaluationContext, ExpressionEngine, MaybeExpression};
use nebula_value::{Value, ValueKind};

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
/// ```rust,ignore
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
pub trait Parameter: Downcast + Send + Sync {
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

impl_downcast!(Parameter);

// =============================================================================
// Validation Trait
// =============================================================================

/// Trait for parameters that support validation
///
/// Provides both synchronous (fast) and asynchronous (complex) validation
/// of external values. Parameters implementing this trait define the schema
/// and validation rules, but don't store values themselves.
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
/// ```rust,ignore
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
///     fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
///         let num = value.as_number().ok_or_else(|| ParameterError::InvalidValue {
///             key: self.metadata().key.clone(),
///             reason: "Expected number".to_string(),
///         })?;
///
///         if let Some(min) = self.min {
///             if num < min {
///                 return Err(ParameterError::InvalidValue {
///                     key: self.metadata().key.clone(),
///                     reason: format!("Value {} below minimum {}", num, min),
///                 });
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Validatable: Parameter + Send + Sync {
    /// Get the expected value kind for this parameter
    ///
    /// Returns the `ValueKind` that this parameter expects.
    /// The default implementation returns `None`, meaning
    /// no type checking is performed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// impl Validatable for TextParameter {
    ///     fn expected_kind(&self) -> Option<ValueKind> {
    ///         Some(ValueKind::String)
    ///     }
    /// }
    ///
    /// impl Validatable for NumberParameter {
    ///     fn expected_kind(&self) -> Option<ValueKind> {
    ///         Some(ValueKind::Float) // Numbers stored as float
    ///     }
    /// }
    /// ```
    fn expected_kind(&self) -> Option<ValueKind> {
        None // No type checking by default
    }

    /// Synchronous validation (fast, local checks)
    ///
    /// Override this for basic validation like:
    /// - Type checks
    /// - Range checks
    /// - Regex matching
    /// - Required field validation
    /// - Format validation
    ///
    /// Default implementation checks:
    /// 1. Type matches `expected_kind()` (if specified)
    /// 2. Required field is not empty
    ///
    /// # Parameters
    ///
    /// * `value` - The value to validate (passed by reference)
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // 1. Type checking (if expected_kind is specified)
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            // Allow Null for optional parameters
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata().key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // 2. Required field check
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
    ///
    /// # Parameters
    ///
    /// * `value` - The value to validate (passed by reference)
    async fn validate_async(&self, _value: &Value) -> Result<(), ParameterError> {
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
    /// # Parameters
    ///
    /// * `value` - The value to validate (passed by reference)
    async fn validate(&self, value: &Value) -> Result<(), ParameterError> {
        // 1. Fast synchronous checks
        self.validate_sync(value)?;

        // 2. Slow asynchronous checks
        self.validate_async(value).await?;

        // 3. Custom validation from configuration
        if let Some(validation) = self.validation() {
            validation
                .validate(value, None)
                .await
                .map_err(|e| ParameterError::InvalidValue {
                    key: self.metadata().key.clone(),
                    reason: format!("{e}"),
                })?;
        }

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
    /// **You MUST override this method** for parameters expecting:
    /// - Strings (empty string `""`)
    /// - Vectors/Arrays (empty collection `[]`)
    /// - Maps/Objects (empty map `{}`)
    ///
    /// If not overridden, `is_empty` always returns `false`, which means
    /// **empty strings/collections will pass required field validation!**
    ///
    /// # Parameters
    ///
    /// * `value` - The value to check for emptiness
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use nebula_parameter::prelude::*;
    /// impl Validatable for TextParameter {
    ///     fn is_empty(&self, value: &Value) -> bool {
    ///         matches!(value, Value::String(s) if s.is_empty())
    ///     }
    /// }
    ///
    /// impl Validatable for ArrayParameter {
    ///     fn is_empty(&self, value: &Value) -> bool {
    ///         matches!(value, Value::Array(arr) if arr.is_empty())
    ///     }
    /// }
    /// ```
    fn is_empty(&self, _value: &Value) -> bool {
        false // Default: most types don't have "empty" concept
    }
}

// =============================================================================
// Expression Evaluation Trait
// =============================================================================

/// Trait for parameters that can evaluate expressions
///
/// This trait provides methods for detecting and evaluating expression values.
/// Unlike the previous design, parameters no longer store values - they only
/// define the schema and provide evaluation capabilities.
///
/// # Expression Detection
///
/// Values can be in two forms:
/// - **Concrete values**: `Value::String("hello")`
/// - **Expression strings**: `Value::String("{{ $input.value }}")`
///
/// Use `is_expression_value` to detect if a value contains an expression.
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
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = TextParameter::new("user_name");
/// let value = Value::String("{{ $input.user.name }}".to_string());
///
/// assert!(param.is_expression_value(&value));
///
/// // Evaluate at runtime
/// let engine = ExpressionEngine::new();
/// let mut context = EvaluationContext::new();
/// context.set_input(json!({"user": {"name": "Alice"}}));
///
/// let result = param.evaluate(&value, &engine, &context).await.unwrap();
/// assert_eq!(result, Value::String("Alice".to_string()));
/// ```
#[async_trait::async_trait]
pub trait Expressible: Parameter {
    /// Check if a value contains an expression
    ///
    /// This should detect expression syntax in the value.
    /// For string-based parameters, this typically checks for `{{ ... }}` markers.
    ///
    /// # Parameters
    ///
    /// * `value` - The value to check for expression syntax
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use nebula_parameter::prelude::*;
    /// let param = TextParameter::new("test");
    /// assert!(param.is_expression_value(&Value::String("{{ 1 + 1 }}".to_string())));
    /// assert!(!param.is_expression_value(&Value::String("hello".to_string())));
    /// ```
    fn is_expression_value(&self, value: &Value) -> bool;

    /// Evaluate an expression value and return the result
    ///
    /// If the value is a concrete value (not an expression), returns it directly.
    /// If it contains an expression, evaluates it using the provided engine and context.
    ///
    /// # Parameters
    ///
    /// * `value` - The value to evaluate (may be concrete or expression)
    /// * `engine` - The expression engine to use for evaluation
    /// * `context` - The evaluation context containing variables
    ///
    /// # Errors
    ///
    /// Returns an error if expression evaluation fails.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use nebula_parameter::prelude::*;
    /// # use nebula_value::Value;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let param = TextParameter::builder()
    ///     .metadata(ParameterMetadata::builder()
    ///         .key("test")
    ///         .name("Test")
    ///         .description("")
    ///         .build()?)
    ///     .build();
    /// let engine = ExpressionEngine::new();
    /// let context = EvaluationContext::new();
    ///
    /// // Concrete value
    /// let value = Value::text("hello");
    /// let result = param.evaluate(&value, &engine, &context).await?;
    /// assert_eq!(result, value);
    ///
    /// // Expression value
    /// let expr_value = Value::text("{{ 1 + 1 }}");
    /// let result = param.evaluate(&expr_value, &engine, &context).await?;
    /// assert_eq!(result, Value::integer(2));
    /// # Ok(())
    /// # }
    /// ```
    async fn evaluate(
        &self,
        value: &Value,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Value, ParameterError>;
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
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
/// use nebula_value::Value;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut param = TextParameter::new("apiKey");
///
/// // Show only when authentication type is "api_key"
/// param.add_condition(
///     "authType".into(),
///     ParameterCondition::equals("api_key")
/// );
///
/// let context = DisplayContext::new()
///     .with_value("authType", Value::text("api_key"));
///
/// assert!(param.should_display(&context));
/// # Ok(())
/// # }
/// ```
pub trait Displayable: Parameter {
    /// Get the display configuration
    fn display(&self) -> Option<&ParameterDisplay>;

    /// Update display conditions
    fn set_display(&mut self, display: Option<ParameterDisplay>);

    /// Check if the parameter should be displayed given the current context
    fn should_display(&self, context: &DisplayContext) -> bool {
        match self.display() {
            Some(display_config) => display_config.should_display(context),
            None => true,
        }
    }

    // TODO: Re-enable once ParameterDisplayError is implemented
    // /// Validate display conditions and return detailed error if hidden
    // ///
    // /// This is useful for providing user-friendly error messages when
    // /// trying to access a hidden parameter.
    // fn validate_display(&self, context: &DisplayContext) -> Result<(), ParameterDisplayError> {
    //     match self.display() {
    //         Some(display_config) => display_config.validate_display(context),
    //         None => Ok(()),
    //     }
    // }

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
            Some(display_config) => display_config.dependencies(),
            None => Vec::new(),
        }
    }
}

/// Extension trait for mutable display operations
///
/// Separated from [`Displayable`] to keep the core trait immutable-friendly.
/// This is automatically implemented for all `Displayable` types.
pub trait DisplayableMut: Displayable {
    // TODO: Re-enable once ParameterCondition is implemented
    // /// Add a display condition
    // ///
    // /// Multiple conditions can be added for the same or different properties.
    // /// All conditions must be satisfied for the parameter to be displayed.
    // fn add_condition(&mut self, property: Key, condition: ParameterCondition) {
    //     let mut display = self.display().cloned().unwrap_or_default();
    //     display.add_show_condition(property, condition);
    //     self.set_display(Some(display));
    // }

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
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// struct SensitiveParameter {
///     // ... fields ...
/// }
///
/// impl DisplayableReactive for SensitiveParameter {
///     fn on_hide(&mut self, _context: &DisplayContext) {
///         // Maybe clear cached data or reset state
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
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
/// use nebula_value::Value;
///
/// let mut param = SensitiveParameter::new("secret");
/// let old_ctx = DisplayContext::new().with_value("show_secrets", Value::boolean(false));
/// let new_ctx = DisplayContext::new().with_value("show_secrets", Value::boolean(true));
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
    pub async fn assert_valid<P>(param: &P, value: &Value)
    where
        P: Validatable,
    {
        param.validate(value).await.expect("validation should pass");
    }

    /// Assert that a value is invalid for a parameter
    pub async fn assert_invalid<P>(param: &P, value: &Value)
    where
        P: Validatable,
    {
        assert!(
            param.validate(value).await.is_err(),
            "validation should fail but passed"
        );
    }

    /// Assert that validation fails with a specific error type
    pub async fn assert_invalid_with<P, F>(param: &P, value: &Value, check: F)
    where
        P: Validatable,
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
