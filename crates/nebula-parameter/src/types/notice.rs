use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ParameterError;
use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterKind, ParameterMetadata, Validatable,
};
use nebula_value::Value;

/// Type of notice
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum NoticeType {
    #[serde(rename = "info")]
    #[default]
    Info,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "success")]
    Success,
}

/// Configuration options for notice parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoticeParameterOptions {
    /// Type of notice (info, warning, error, success)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_type: Option<NoticeType>,

    /// Whether the notice can be dismissed by the user
    #[serde(default)]
    pub dismissible: bool,
}

/// Builder for NoticeParameterOptions
#[derive(Debug, Default)]
pub struct NoticeParameterOptionsBuilder {
    notice_type: Option<NoticeType>,
    dismissible: bool,
}

impl NoticeParameterOptionsBuilder {
    /// Create a new options builder
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the notice type
    #[must_use]
    pub fn notice_type(mut self, notice_type: NoticeType) -> Self {
        self.notice_type = Some(notice_type);
        self
    }

    /// Set whether the notice is dismissible
    #[must_use]
    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> NoticeParameterOptions {
        NoticeParameterOptions {
            notice_type: self.notice_type,
            dismissible: self.dismissible,
        }
    }
}

impl NoticeParameterOptions {
    /// Create a new options builder
    #[must_use]
    pub fn builder() -> NoticeParameterOptionsBuilder {
        NoticeParameterOptionsBuilder::new()
    }
}

/// Parameter for displaying a notice or information to the user.
///
/// Notice parameters are display-only and do not accept user input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoticeParameter {
    /// Parameter metadata (flattened for cleaner JSON)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The text content of the notice
    pub content: String,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<NoticeParameterOptions>,

    /// Display configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

/// Builder for NoticeParameter
#[derive(Debug, Default)]
pub struct NoticeParameterBuilder {
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    content: Option<String>,
    options: Option<NoticeParameterOptions>,
    display: Option<ParameterDisplay>,
}

impl NoticeParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the parameter name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the parameter description
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

    /// Set the placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the notice content (required)
    #[must_use]
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Set the notice type
    #[must_use]
    pub fn notice_type(mut self, notice_type: NoticeType) -> Self {
        let options = self
            .options
            .get_or_insert_with(NoticeParameterOptions::default);
        options.notice_type = Some(notice_type);
        self
    }

    /// Set whether the notice is dismissible
    #[must_use]
    pub fn dismissible(mut self, dismissible: bool) -> Self {
        let options = self
            .options
            .get_or_insert_with(NoticeParameterOptions::default);
        options.dismissible = dismissible;
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: NoticeParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set the display configuration
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Build the NoticeParameter
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing or invalid
    pub fn build(self) -> Result<NoticeParameter, ParameterError> {
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
            .maybe_placeholder(self.placeholder)
            .maybe_hint(self.hint)
            .build()?;

        let content = self
            .content
            .ok_or_else(|| ParameterError::BuilderMissingField {
                field: "content".into(),
            })?;

        Ok(NoticeParameter {
            metadata,
            content,
            options: self.options,
            display: self.display,
        })
    }
}

impl NoticeParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> NoticeParameterBuilder {
        NoticeParameterBuilder::new()
    }

    /// Get the notice content
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Get the notice type
    #[must_use]
    pub fn notice_type(&self) -> NoticeType {
        self.options
            .as_ref()
            .and_then(|o| o.notice_type.clone())
            .unwrap_or_default()
    }

    /// Check if the notice is dismissible
    #[must_use]
    pub fn is_dismissible(&self) -> bool {
        self.options.as_ref().is_some_and(|o| o.dismissible)
    }
}

impl Describable for NoticeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Notice
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl Validatable for NoticeParameter {
    fn is_empty(&self, _value: &Value) -> bool {
        false // Notice parameters don't have values
    }
}

impl fmt::Display for NoticeParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NoticeParameter({})", self.metadata.name)
    }
}

impl Displayable for NoticeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notice_parameter_builder() {
        let param = NoticeParameter::builder()
            .key("info_notice")
            .name("Information")
            .description("Important information")
            .content("Please read this carefully.")
            .notice_type(NoticeType::Info)
            .dismissible(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "info_notice");
        assert_eq!(param.metadata.name, "Information");
        assert_eq!(param.content(), "Please read this carefully.");
        assert_eq!(param.notice_type(), NoticeType::Info);
        assert!(param.is_dismissible());
    }

    #[test]
    fn test_notice_parameter_missing_key() {
        let result = NoticeParameter::builder()
            .name("Test")
            .content("Content")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_notice_parameter_missing_content() {
        let result = NoticeParameter::builder().key("test").name("Test").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_notice_types() {
        assert_eq!(NoticeType::default(), NoticeType::Info);

        let warning_param = NoticeParameter::builder()
            .key("warn")
            .name("Warning")
            .content("Warning message")
            .notice_type(NoticeType::Warning)
            .build()
            .unwrap();

        assert_eq!(warning_param.notice_type(), NoticeType::Warning);
    }

    #[test]
    fn test_notice_options_builder() {
        let options = NoticeParameterOptions::builder()
            .notice_type(NoticeType::Error)
            .dismissible(true)
            .build();

        assert_eq!(options.notice_type, Some(NoticeType::Error));
        assert!(options.dismissible);
    }

    #[test]
    fn test_notice_serialization() {
        let param = NoticeParameter::builder()
            .key("test")
            .name("Test Notice")
            .description("A test notice")
            .content("Notice content")
            .notice_type(NoticeType::Success)
            .build()
            .unwrap();

        let json = serde_json::to_string_pretty(&param).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("Notice content"));
        assert!(json.contains("success"));
    }
}
