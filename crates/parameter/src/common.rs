//! Common traits for parameter types.

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Common interface that all parameter types implement.
///
/// This trait provides unified access to metadata, display rules,
/// and validation rules across all parameter variants.
pub trait ParameterType {
    /// Get immutable reference to metadata.
    fn metadata(&self) -> &ParameterMetadata;

    /// Get mutable reference to metadata.
    fn metadata_mut(&mut self) -> &mut ParameterMetadata;

    /// Get display rules, if any.
    fn display(&self) -> Option<&ParameterDisplay>;

    /// Get mutable reference to display rules.
    fn display_mut(&mut self) -> &mut Option<ParameterDisplay>;

    /// Get validation rules.
    fn validation_rules(&self) -> &[ValidationRule];

    /// Get mutable reference to validation rules.
    fn validation_rules_mut(&mut self) -> &mut Vec<ValidationRule>;

    /// Convenience: get parameter key.
    #[must_use]
    fn key(&self) -> &str {
        &self.metadata().key
    }

    /// Convenience: get parameter name.
    #[must_use]
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Convenience: check if required.
    #[must_use]
    fn is_required(&self) -> bool {
        self.metadata().required
    }

    /// Convenience: check if sensitive.
    #[must_use]
    fn is_sensitive(&self) -> bool {
        self.metadata().sensitive
    }

    /// Builder-style: mark as required.
    fn required(mut self) -> Self
    where
        Self: Sized,
    {
        self.metadata_mut().required = true;
        self
    }

    /// Builder-style: mark as sensitive.
    fn sensitive(mut self) -> Self
    where
        Self: Sized,
    {
        self.metadata_mut().sensitive = true;
        self
    }

    /// Builder-style: set description.
    fn description(mut self, desc: impl Into<String>) -> Self
    where
        Self: Sized,
    {
        self.metadata_mut().description = Some(desc.into());
        self
    }

    /// Builder-style: set placeholder.
    fn placeholder(mut self, text: impl Into<String>) -> Self
    where
        Self: Sized,
    {
        self.metadata_mut().placeholder = Some(text.into());
        self
    }

    /// Builder-style: set hint.
    fn hint(mut self, text: impl Into<String>) -> Self
    where
        Self: Sized,
    {
        self.metadata_mut().hint = Some(text.into());
        self
    }

    /// Builder-style: add a validation rule.
    fn with_validation(mut self, rule: ValidationRule) -> Self
    where
        Self: Sized,
    {
        self.validation_rules_mut().push(rule);
        self
    }

    /// Builder-style: set display rules.
    fn with_display(mut self, display: ParameterDisplay) -> Self
    where
        Self: Sized,
    {
        *self.display_mut() = Some(display);
        self
    }
}

/// Macro to implement `ParameterType` for a parameter struct.
///
/// # Example
///
/// ```ignore
/// impl_parameter_type!(TextParameter);
/// ```
#[macro_export]
macro_rules! impl_parameter_type {
    ($type:ty) => {
        impl $crate::common::ParameterType for $type {
            fn metadata(&self) -> &$crate::metadata::ParameterMetadata {
                &self.metadata
            }

            fn metadata_mut(&mut self) -> &mut $crate::metadata::ParameterMetadata {
                &mut self.metadata
            }

            fn display(&self) -> Option<&$crate::display::ParameterDisplay> {
                self.display.as_ref()
            }

            fn display_mut(&mut self) -> &mut Option<$crate::display::ParameterDisplay> {
                &mut self.display
            }

            fn validation_rules(&self) -> &[$crate::validation::ValidationRule] {
                &self.validation
            }

            fn validation_rules_mut(&mut self) -> &mut Vec<$crate::validation::ValidationRule> {
                &mut self.validation
            }
        }
    };
}
