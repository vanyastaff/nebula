// =============================================================================
// Parameter Metadata - Core parameter identification and description
// =============================================================================
//!
//! Metadata provides the core identification and descriptive information
//! for parameters. It's used by all parameter types to store basic attributes
//! like key, name, description, and UI hints.

use crate::core::ParameterError;
use bon::bon;
use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Core metadata for all parameters
///
/// This structure contains the essential identification and descriptive information
/// needed by all parameter types. It follows the builder pattern for ergonomic construction.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::ParameterMetadata;
///
/// // Basic metadata
/// let metadata = ParameterMetadata::builder()
///     .key("username")
///     .name("Username")
///     .description("Your account username")
///     .build()?;
///
/// // Required parameter with hints
/// let metadata = ParameterMetadata::builder()
///     .key("email")
///     .name("Email Address")
///     .description("Your email for notifications")
///     .required(true)
///     .placeholder("user@example.com")
///     .hint("We'll never share your email")
///     .build()?;
/// ```
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct ParameterMetadata {
    /// Unique identifier for the parameter
    ///
    /// Must be a valid `ParameterKey` (lowercase, alphanumeric, underscores).
    /// Used as the stable identifier for parameter access and storage.
    pub key: ParameterKey,

    /// Human-readable display name
    ///
    /// Shown to users in UI forms. Should be concise and descriptive.
    pub name: String,

    /// Detailed description of the parameter's purpose
    ///
    /// Provides context and guidance to users. Can be multi-line.
    pub description: String,

    /// Whether this parameter must be provided
    ///
    /// When `true`, validation will fail if no value is set.
    /// Defaults to `false` (optional parameter).
    #[serde(default)]
    pub required: bool,

    /// Placeholder text for UI inputs
    ///
    /// Shown in empty input fields as a hint. Should be an example value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Additional help text or usage hint
    ///
    /// Supplementary information shown near the input, typically in smaller text.
    /// Use for format specifications, character limits, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[bon]
impl ParameterMetadata {
    /// Create metadata with validation (builder pattern)
    ///
    /// This is the primary constructor that validates the key format.
    /// Use the builder pattern for a more ergonomic API.
    ///
    /// # Errors
    ///
    /// Returns `ParameterError::InvalidKeyFormat` if the key is invalid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let metadata = ParameterMetadata::builder()
    ///     .key("api_key")
    ///     .name("API Key")
    ///     .description("Your authentication key")
    ///     .required(true)
    ///     .build()?;
    /// ```
    #[builder]
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        #[builder(default = false)] required: bool,
        placeholder: Option<String>,
        hint: Option<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required,
            placeholder,
            hint,
        })
    }

    /// Create required parameter metadata
    ///
    /// Convenience method for creating metadata with `required = true`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let metadata = ParameterMetadata::builder()
    ///     .required()
    ///     .key("password")
    ///     .name("Password")
    ///     .description("Your secure password")
    ///     .build()?;
    /// ```
    #[builder]
    pub fn required(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        placeholder: Option<String>,
        hint: Option<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required: true,
            placeholder,
            hint,
        })
    }

    /// Create optional parameter metadata
    ///
    /// Convenience method for creating basic optional metadata without UI hints.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let metadata = ParameterMetadata::builder()
    ///     .optional()
    ///     .key("middle_name")
    ///     .name("Middle Name")
    ///     .description("Your middle name (optional)")
    ///     .build()?;
    /// ```
    #[builder]
    pub fn optional(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required: false,
            placeholder: None,
            hint: None,
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
            .placeholder("user@example.com".to_string())
            .hint("We'll never share your email".to_string())
            .build()
            .unwrap();

        assert!(metadata.has_placeholder());
        assert!(metadata.has_hint());
    }

    #[test]
    fn test_metadata_invalid_key() {
        let result = ParameterMetadata::builder()
            .key("Invalid Key!") // Spaces and special chars not allowed
            .name("Test")
            .description("Test")
            .build();

        assert!(result.is_err());
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
            .placeholder("example".to_string())
            .build()
            .unwrap();

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: ParameterMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(metadata, deserialized);
    }
}
