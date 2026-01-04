//! Parameter Metadata - Core parameter identification and description
//!
//! Metadata provides the core identification and descriptive information
//! for parameters. It's used by all parameter types to store basic attributes
//! like key, name, description, and UI hints.

use crate::core::ParameterError;
use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};

/// Core metadata for all parameters
///
/// This structure contains the essential identification and descriptive information
/// needed by all parameter types.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::ParameterMetadata;
///
/// let metadata = ParameterMetadata::builder()
///     .key("username")
///     .name("Username")
///     .description("Your account username")
///     .required(true)
///     .placeholder("john_doe")
///     .build()?;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParameterMetadata {
    /// Unique identifier for the parameter
    ///
    /// Must be a valid `ParameterKey` (lowercase, alphanumeric, underscores).
    pub key: ParameterKey,

    /// Human-readable display name
    pub name: String,

    /// Detailed description of the parameter's purpose
    #[serde(default)]
    pub description: String,

    /// Whether this parameter must be provided
    #[serde(default)]
    pub required: bool,

    /// Placeholder text for UI inputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Additional help text or usage hint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

// =============================================================================
// Builder
// =============================================================================

/// Builder for `ParameterMetadata`
#[derive(Debug, Default)]
pub struct ParameterMetadataBuilder {
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
}

impl ParameterMetadata {
    /// Create a new builder for `ParameterMetadata`
    #[must_use]
    pub fn builder() -> ParameterMetadataBuilder {
        ParameterMetadataBuilder::default()
    }
}

impl ParameterMetadataBuilder {
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

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set placeholder text if Some
    #[must_use]
    pub fn maybe_placeholder(mut self, placeholder: Option<String>) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Set hint text if Some
    #[must_use]
    pub fn maybe_hint(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }

    /// Build the `ParameterMetadata`
    ///
    /// # Errors
    ///
    /// Returns error if `key` or `name` is not set, or if key format is invalid.
    pub fn build(self) -> Result<ParameterMetadata, ParameterError> {
        let key_str = self
            .key
            .ok_or_else(|| ParameterError::BuilderMissingField {
                field: "key".into(),
            })?;

        let name = self
            .name
            .ok_or_else(|| ParameterError::BuilderMissingField {
                field: "name".into(),
            })?;

        let key = ParameterKey::new(key_str)?;

        Ok(ParameterMetadata {
            key,
            name,
            description: self.description,
            required: self.required,
            placeholder: self.placeholder,
            hint: self.hint,
        })
    }
}

// =============================================================================
// Helper methods
// =============================================================================

impl ParameterMetadata {
    /// Get the parameter key as a string slice
    #[inline]
    #[must_use]
    pub fn key_str(&self) -> &str {
        self.key.as_str()
    }

    /// Check if this parameter is required
    #[inline]
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// Check if this parameter is optional
    #[inline]
    #[must_use]
    pub fn is_optional(&self) -> bool {
        !self.required
    }

    /// Check if placeholder text is set
    #[inline]
    #[must_use]
    pub fn has_placeholder(&self) -> bool {
        self.placeholder.is_some()
    }

    /// Check if hint text is set
    #[inline]
    #[must_use]
    pub fn has_hint(&self) -> bool {
        self.hint.is_some()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_builder_basic() {
        let metadata = ParameterMetadata::builder()
            .key("test_param")
            .name("Test Parameter")
            .description("A test parameter")
            .build()
            .unwrap();

        assert_eq!(metadata.key_str(), "test_param");
        assert_eq!(metadata.name, "Test Parameter");
        assert!(!metadata.is_required());
    }

    #[test]
    fn test_metadata_required() {
        let metadata = ParameterMetadata::builder()
            .key("required_field")
            .name("Required Field")
            .description("This field is required")
            .required(true)
            .build()
            .unwrap();

        assert!(metadata.is_required());
        assert!(!metadata.is_optional());
    }

    #[test]
    fn test_metadata_with_hints() {
        let metadata = ParameterMetadata::builder()
            .key("email")
            .name("Email")
            .description("Your email address")
            .placeholder("user@example.com")
            .hint("We'll never share your email")
            .build()
            .unwrap();

        assert!(metadata.has_placeholder());
        assert!(metadata.has_hint());
    }

    #[test]
    fn test_metadata_missing_key() {
        let result = ParameterMetadata::builder()
            .name("Test")
            .description("Test")
            .build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_metadata_missing_name() {
        let result = ParameterMetadata::builder()
            .key("test")
            .description("Test")
            .build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "name"
        ));
    }

    #[test]
    fn test_metadata_invalid_key() {
        let result = ParameterMetadata::builder()
            .key("Invalid Key!")
            .name("Test")
            .description("Test")
            .build();

        assert!(matches!(result, Err(ParameterError::InvalidKeyFormat(_))));
    }

    #[test]
    fn test_metadata_equality() {
        let m1 = ParameterMetadata::builder()
            .key("test")
            .name("Test")
            .description("Test")
            .build()
            .unwrap();

        let m2 = ParameterMetadata::builder()
            .key("test")
            .name("Test")
            .description("Test")
            .build()
            .unwrap();

        assert_eq!(m1, m2);
    }

    #[test]
    fn test_metadata_serialization() {
        let metadata = ParameterMetadata::builder()
            .key("test")
            .name("Test")
            .description("Test")
            .required(true)
            .placeholder("example")
            .build()
            .unwrap();

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: ParameterMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(metadata, deserialized);
    }
}
