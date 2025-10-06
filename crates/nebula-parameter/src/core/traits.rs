//! Core parameter traits

use crate::core::ParameterValue;
use crate::core::condition::ParameterCondition;
use crate::core::display::{DisplayContext, ParameterDisplay, ParameterDisplayError};
use crate::core::validation::ParameterValidation;
use crate::core::{ParameterError, ParameterKind, ParameterMetadata};
use nebula_core::ParameterKey as Key;
use std::fmt::{Debug, Display};

/// Base trait for all parameter types
pub trait ParameterType: Send + Sync {
    /// Get the kind of this parameter
    fn kind(&self) -> ParameterKind;

    /// Get parameter metadata
    fn metadata(&self) -> &ParameterMetadata;

    /// Get parameter key
    #[inline]
    fn key(&self) -> &str {
        self.metadata().key.as_str()
    }

    /// Get parameter name
    #[inline]
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Check if parameter is required
    #[inline]
    fn is_required(&self) -> bool {
        self.metadata().required
    }
}

/// Core trait for parameters that can store values
pub trait HasValue: ParameterType + Debug + Display {
    /// The concrete value type for this parameter
    type Value: Clone + PartialEq + Debug + Send + Sync + 'static;

    // --- Required methods (must be implemented) ---

    /// Gets the current value (immutable reference)
    fn get_value(&self) -> Option<&Self::Value>;

    /// Gets the current value (mutable reference)
    fn get_value_mut(&mut self) -> Option<&mut Self::Value>;

    /// Sets a new value without validation
    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError>;

    /// Gets the default value if defined
    fn default_value(&self) -> Option<&Self::Value>;

    /// Clears the current value
    fn clear_value(&mut self);

    /// Converts to generic ParameterValue
    fn get_parameter_value(&self) -> Option<ParameterValue>;

    /// Sets from generic ParameterValue or any type that converts to it
    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError>;

    // --- Default implementations (convenience methods) ---

    /// Returns true if parameter has a value set
    #[inline]
    fn has_value(&self) -> bool {
        self.get_value().is_some()
    }

    /// Sets a new value with validation (requires Validatable)
    async fn set_value(&mut self, value: Self::Value) -> Result<(), ParameterError>
    where
        Self: Validatable,
    {
        self.validate(&value).await?;
        self.set_value_unchecked(value)
    }

    /// Updates the current value in place using a closure
    fn try_update<F>(&mut self, f: F) -> Result<(), ParameterError>
    where
        F: FnOnce(&mut Self::Value) -> Result<(), ParameterError>,
    {
        match self.get_value_mut() {
            Some(value) => f(value),
            None => Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            }),
        }
    }

    /// Checks if the current value equals the default
    fn is_default(&self) -> bool {
        match (self.get_value(), self.default_value()) {
            (Some(current), Some(default)) => current == default,
            (None, None) => true,
            _ => false,
        }
    }

    /// Resets the parameter's value to its default
    fn reset_to_default(&mut self) -> Result<(), ParameterError> {
        match self.default_value().cloned() {
            Some(default) => self.set_value_unchecked(default),
            None => {
                self.clear_value();
                Ok(())
            }
        }
    }

    /// Takes the current value, leaving the parameter empty
    fn take_value(&mut self) -> Option<Self::Value> {
        let value = self.get_value().cloned();
        self.clear_value();
        value
    }

    /// Gets the current value or the default value
    fn value_or_default(&self) -> Option<&Self::Value> {
        self.get_value().or_else(|| self.default_value())
    }

    /// Gets the current value or a provided fallback
    fn value_or<'a>(&'a self, fallback: &'a Self::Value) -> &'a Self::Value {
        self.get_value().unwrap_or(fallback)
    }

    /// Maps the current value to another type
    fn map_value<U, F>(&self, f: F) -> Option<U>
    where
        F: FnOnce(&Self::Value) -> U,
    {
        self.get_value().map(f)
    }

    /// Sets the default value for this parameter
    fn set_default(&mut self, default: Self::Value) {
        // Default implementation does nothing - override in concrete types
        let _ = default;
    }

    /// Validates the current value of the parameter
    async fn validate_current_value(&self) -> Result<(), ParameterError>
    where
        Self: Validatable,
    {
        match self.get_value() {
            Some(value) => self.validate(value).await,
            None if self.is_required() => Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            }),
            None => Ok(()),
        }
    }
}

/// Trait for parameters that support validation
#[async_trait::async_trait]
pub trait Validatable: HasValue + Send + Sync {
    /// Validates a value for this parameter (async)
    ///
    /// Default implementation provides common validation pattern.
    /// Override this method for custom validation logic.
    async fn validate(&self, value: &Self::Value) -> Result<(), ParameterError> {
        // Use custom validation if available
        if let Some(validation) = self.validation() {
            let nebula_value = self.value_to_nebula_value(value);
            if let Err(validation_error) = validation.validate(&nebula_value, None).await {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata().key.clone(),
                    reason: format!("{}", validation_error),
                });
            }
        }

        // Basic validation - required field check
        if self.is_empty_value(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata().key.clone(),
            });
        }

        Ok(())
    }

    /// Get the validation configuration (default: no validation)
    fn validation(&self) -> Option<&ParameterValidation> {
        None
    }

    /// Convert value to nebula_value::Value for validation (default: null)
    fn value_to_nebula_value(&self, _value: &Self::Value) -> nebula_value::Value {
        nebula_value::Value::Null
    }

    /// Check if a value is considered empty (default: false)
    fn is_empty_value(&self, _value: &Self::Value) -> bool {
        false // Most types don't have an "empty" concept
    }
}

/// Unified trait for parameters that support conditional display
pub trait Displayable: ParameterType {
    // --- Required methods ---

    /// Get the display configuration
    fn display(&self) -> Option<&ParameterDisplay>;

    /// Update display conditions
    fn set_display(&mut self, display: Option<ParameterDisplay>);

    // --- Default implementations ---

    /// Check if the parameter should be displayed given the current context
    fn should_display(&self, context: &DisplayContext) -> bool {
        match self.display() {
            Some(display_config) => display_config.should_display(&context.values),
            None => true, // Display by default if no conditions
        }
    }

    /// Validate display conditions and return detailed error if hidden
    fn validate_display(&self, context: &DisplayContext) -> Result<(), ParameterDisplayError> {
        match self.display() {
            Some(display_config) => display_config.validate_display(&context.values),
            None => Ok(()), // No conditions means always visible
        }
    }

    /// Check if this parameter has any display conditions
    fn has_display_conditions(&self) -> bool {
        match self.display() {
            Some(display_config) => !display_config.is_empty(),
            None => false,
        }
    }

    /// Get all property keys that this parameter's display depends on
    fn display_dependencies(&self) -> Vec<Key> {
        match self.display() {
            Some(display_config) => display_config.get_dependencies(),
            None => Vec::new(),
        }
    }

    /// Add a display condition
    fn add_display_condition(&mut self, property: Key, condition: ParameterCondition) {
        let mut display = self.display().cloned().unwrap_or_default();
        display.add_show_condition(property, condition);
        self.set_display(Some(display));
    }

    /// Clear all display conditions
    fn clear_display_conditions(&mut self) {
        self.set_display(None);
    }


    // --- Optional reactive methods (default empty implementations) ---

    /// Called when parameter becomes visible
    fn on_show(&mut self, _context: &DisplayContext) {}

    /// Called when parameter becomes hidden
    fn on_hide(&mut self, _context: &DisplayContext) {}

    /// Called when display conditions change
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
