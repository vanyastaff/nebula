//! Base parameter structure to reduce code duplication across parameter types.

use serde::{Deserialize, Serialize};

use crate::core::{ParameterDisplay, ParameterMetadata, ParameterValidation};

/// Common fields shared by all parameter types.
///
/// This struct contains the fields that every parameter type needs,
/// eliminating duplication across the 24+ parameter implementations.
///
/// # Usage
///
/// Parameter types should include this as a flattened field:
///
/// ```rust,ignore
/// use nebula_parameter::core::ParameterBase;
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub struct TextParameter {
///     #[serde(flatten)]
///     pub base: ParameterBase,
///
///     pub default: Option<String>,
///     pub options: Option<TextParameterOptions>,
/// }
/// ```
///
/// # Serialization
///
/// The `metadata` field is flattened during serialization, so JSON looks like:
/// ```json
/// {
///   "key": "username",
///   "name": "Username",
///   "description": "Enter username",
///   "required": true,
///   "display": { ... },
///   "validation": { ... }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterBase {
    /// Core parameter metadata (key, name, description, required, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Display conditions controlling when this parameter is shown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl ParameterBase {
    /// Create a new base with just metadata
    #[inline]
    #[must_use]
    pub fn new(metadata: ParameterMetadata) -> Self {
        Self {
            metadata,
            display: None,
            validation: None,
        }
    }

    /// Builder: set display configuration
    #[must_use]
    pub fn with_display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Builder: set validation configuration
    #[must_use]
    pub fn with_validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_base_new() {
        let metadata = ParameterMetadata::builder()
            .key("test")
            .name("Test")
            .description("A test parameter")
            .build()
            .unwrap();

        let base = ParameterBase::new(metadata);

        assert_eq!(base.metadata.key.as_str(), "test");
        assert!(base.display.is_none());
        assert!(base.validation.is_none());
    }

    #[test]
    fn test_parameter_base_with_display() {
        let metadata = ParameterMetadata::builder()
            .key("test")
            .name("Test")
            .description("")
            .build()
            .unwrap();

        let base = ParameterBase::new(metadata).with_display(ParameterDisplay::new());

        assert!(base.display.is_some());
    }

    #[test]
    fn test_parameter_base_serialization() {
        let metadata = ParameterMetadata::builder()
            .key("username")
            .name("Username")
            .description("Enter username")
            .required(true)
            .build()
            .unwrap();

        let base = ParameterBase::new(metadata);

        let json = serde_json::to_string(&base).unwrap();

        // Verify metadata is flattened
        assert!(json.contains("\"key\":\"username\""));
        assert!(json.contains("\"name\":\"Username\""));
        assert!(json.contains("\"required\":true"));

        // Verify optional fields are not serialized when None
        assert!(!json.contains("display"));
        assert!(!json.contains("validation"));
    }

    #[test]
    fn test_parameter_base_deserialization() {
        let json = r#"{
            "key": "email",
            "name": "Email",
            "description": "Your email",
            "required": false
        }"#;

        let base: ParameterBase = serde_json::from_str(json).unwrap();

        assert_eq!(base.metadata.key.as_str(), "email");
        assert_eq!(base.metadata.name, "Email");
        assert!(!base.metadata.required);
        assert!(base.display.is_none());
        assert!(base.validation.is_none());
    }
}
