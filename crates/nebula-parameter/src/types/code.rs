use serde::{Deserialize, Serialize};

use crate::core::traits::ParameterValue;
use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Parameter for code input with syntax highlighting and validation
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct CodeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<CodeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct CodeParameterOptions {
    /// Programming language for syntax highlighting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeLanguage>,

    /// Read-only mode
    #[builder(default)]
    #[serde(default)]
    pub readonly: bool,
}

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

impl Parameter for CodeParameter {
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
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Check required
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Type check - allow null or text
        if !value.is_null() && value.as_text().is_none() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected text value".to_string(),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
            || value
                .as_text()
                .map(|s| s.trim().is_empty())
                .unwrap_or(false)
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

impl ParameterValue for CodeParameter {
    fn validate_value(
        &self,
        value: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>
    {
        let value = value.clone();
        Box::pin(async move { self.validate(&value).await })
    }

    fn accepts_value(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some()
    }

    fn expected_type(&self) -> &'static str {
        "text"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
