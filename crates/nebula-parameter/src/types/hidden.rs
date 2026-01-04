//! Hidden parameter type for storing values not shown in UI

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    Validatable,
};
use nebula_value::Value;

/// Parameter that is hidden from the user interface but can store values
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = HiddenParameter::builder()
///     .key("internal_id")
///     .name("Internal ID")
///     .default("auto-generated")
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenParameter {
    /// Parameter metadata (key, name, description, etc.)
    /// Note: display and validation are ignored for hidden parameters
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

// =============================================================================
// HiddenParameter Builder
// =============================================================================

/// Builder for `HiddenParameter`
#[derive(Debug, Default)]
pub struct HiddenParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    // Parameter fields
    default: Option<String>,
}

impl HiddenParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> HiddenParameterBuilder {
        HiddenParameterBuilder::new()
    }
}

impl HiddenParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            default: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `HiddenParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<HiddenParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description)
            .required(self.required)
            .build()?;

        Ok(HiddenParameter {
            metadata,
            default: self.default,
        })
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for HiddenParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Hidden
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for HiddenParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HiddenParameter({})", self.metadata.name)
    }
}

// Hidden parameters implement minimal Validatable and Displayable for blanket Parameter impl
impl Validatable for HiddenParameter {
    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
    }
}

impl Displayable for HiddenParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        None // Hidden parameters are never displayed
    }

    fn set_display(&mut self, _display: Option<ParameterDisplay>) {
        // No-op for hidden parameters
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hidden_parameter_builder() {
        let param = HiddenParameter::builder()
            .key("internal_id")
            .name("Internal ID")
            .description("Auto-generated internal identifier")
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "internal_id");
        assert_eq!(param.metadata.name, "Internal ID");
    }

    #[test]
    fn test_hidden_parameter_with_default() {
        let param = HiddenParameter::builder()
            .key("version")
            .name("Version")
            .default("1.0.0")
            .build()
            .unwrap();

        assert_eq!(param.default, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_hidden_parameter_missing_key() {
        let result = HiddenParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_hidden_parameter_display_always_none() {
        let param = HiddenParameter::builder()
            .key("hidden")
            .name("Hidden")
            .build()
            .unwrap();

        assert!(param.display().is_none());
    }
}
