//! Code parameter type for code input with syntax highlighting

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for code input with syntax highlighting and validation
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = CodeParameter::builder()
///     .key("script")
///     .name("Script")
///     .description("Enter your JavaScript code")
///     .options(
///         CodeParameterOptions::builder()
///             .language(CodeLanguage::JavaScript)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CodeParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for code parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeParameterOptions {
    /// Programming language for syntax highlighting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeLanguage>,

    /// Read-only mode
    #[serde(default)]
    pub readonly: bool,
}

/// Supported programming languages
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CodeLanguage {
    #[serde(rename = "javascript")]
    JavaScript,
    #[serde(rename = "typescript")]
    TypeScript,
    #[serde(rename = "python")]
    Python,
    #[serde(rename = "rust")]
    Rust,
    #[serde(rename = "go")]
    Go,
    #[serde(rename = "java")]
    Java,
    #[serde(rename = "c")]
    C,
    #[serde(rename = "cpp")]
    Cpp,
    #[serde(rename = "csharp")]
    CSharp,
    #[serde(rename = "php")]
    Php,
    #[serde(rename = "ruby")]
    Ruby,
    #[serde(rename = "shell")]
    Shell,
    #[serde(rename = "sql")]
    Sql,
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "yaml")]
    Yaml,
    #[serde(rename = "xml")]
    Xml,
    #[serde(rename = "html")]
    Html,
    #[serde(rename = "css")]
    Css,
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "text")]
    #[default]
    PlainText,
}

// =============================================================================
// CodeParameter Builder
// =============================================================================

/// Builder for `CodeParameter`
#[derive(Debug, Default)]
pub struct CodeParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<CodeParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl CodeParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> CodeParameterBuilder {
        CodeParameterBuilder::new()
    }
}

impl CodeParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            options: None,
            display: None,
            validation: None,
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

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: impl Into<nebula_value::Text>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: CodeParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `CodeParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<CodeParameter, ParameterError> {
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

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(CodeParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// CodeParameterOptions Builder
// =============================================================================

/// Builder for `CodeParameterOptions`
#[derive(Debug, Default)]
pub struct CodeParameterOptionsBuilder {
    language: Option<CodeLanguage>,
    readonly: bool,
}

impl CodeParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> CodeParameterOptionsBuilder {
        CodeParameterOptionsBuilder::default()
    }
}

impl CodeParameterOptionsBuilder {
    /// Set the programming language
    #[must_use]
    pub fn language(mut self, language: CodeLanguage) -> Self {
        self.language = Some(language);
        self
    }

    /// Set whether the editor is read-only
    #[must_use]
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> CodeParameterOptions {
        CodeParameterOptions {
            language: self.language,
            readonly: self.readonly,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for CodeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Code
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for CodeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CodeParameter({})", self.metadata.name)
    }
}

impl Validatable for CodeParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
    }
}

impl Displayable for CodeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl CodeParameter {
    /// Get the programming language
    #[must_use]
    pub fn get_language(&self) -> CodeLanguage {
        self.options
            .as_ref()
            .and_then(|opts| opts.language.as_ref())
            .cloned()
            .unwrap_or_default()
    }

    /// Check if the editor is read-only
    #[must_use]
    pub fn is_readonly(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.readonly)
    }

    /// Count lines in code
    #[must_use]
    pub fn get_line_count(&self, code: &nebula_value::Text) -> usize {
        code.lines().count()
    }

    /// Get language file extension
    #[must_use]
    pub fn get_file_extension(&self) -> &'static str {
        match self.get_language() {
            CodeLanguage::JavaScript => ".js",
            CodeLanguage::TypeScript => ".ts",
            CodeLanguage::Python => ".py",
            CodeLanguage::Rust => ".rs",
            CodeLanguage::Go => ".go",
            CodeLanguage::Java => ".java",
            CodeLanguage::C => ".c",
            CodeLanguage::Cpp => ".cpp",
            CodeLanguage::CSharp => ".cs",
            CodeLanguage::Php => ".php",
            CodeLanguage::Ruby => ".rb",
            CodeLanguage::Shell => ".sh",
            CodeLanguage::Sql => ".sql",
            CodeLanguage::Json => ".json",
            CodeLanguage::Yaml => ".yaml",
            CodeLanguage::Xml => ".xml",
            CodeLanguage::Html => ".html",
            CodeLanguage::Css => ".css",
            CodeLanguage::Markdown => ".md",
            CodeLanguage::PlainText => ".txt",
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_parameter_builder() {
        let param = CodeParameter::builder()
            .key("script")
            .name("Script")
            .description("Enter your code")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "script");
        assert_eq!(param.metadata.name, "Script");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_code_parameter_with_options() {
        let param = CodeParameter::builder()
            .key("query")
            .name("SQL Query")
            .options(
                CodeParameterOptions::builder()
                    .language(CodeLanguage::Sql)
                    .readonly(false)
                    .build(),
            )
            .build()
            .unwrap();

        assert_eq!(param.get_language(), CodeLanguage::Sql);
        assert!(!param.is_readonly());
        assert_eq!(param.get_file_extension(), ".sql");
    }

    #[test]
    fn test_code_parameter_with_default() {
        let param = CodeParameter::builder()
            .key("template")
            .name("Template")
            .default("console.log('Hello');")
            .build()
            .unwrap();

        assert_eq!(
            param.default.as_ref().map(|t| t.as_str()),
            Some("console.log('Hello');")
        );
    }

    #[test]
    fn test_code_parameter_missing_key() {
        let result = CodeParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
