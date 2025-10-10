use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for code input with syntax highlighting and validation
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct CodeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<nebula_value::Text>,

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

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct CodeParameterOptions {
    /// Programming language for syntax highlighting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeLanguage>,

    /// Read-only mode
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    PlainText,
}

impl Default for CodeLanguage {
    fn default() -> Self {
        CodeLanguage::PlainText
    }
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

impl HasValue for CodeParameter {
    type Value = nebula_value::Text;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

#[async_trait::async_trait]
impl Expressible for CodeParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .as_ref()
            .map(|s| MaybeExpression::Value(nebula_value::Value::Text(s.clone())))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Text(s)) => {
                self.value = Some(s);
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                // Allow expressions for dynamic code
                self.value = Some(nebula_value::Text::from(expr));
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for code parameter".to_string(),
            }),
        }
    }
}

impl Validatable for CodeParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.trim().is_empty()
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
    pub fn get_language(&self) -> CodeLanguage {
        self.options
            .as_ref()
            .and_then(|opts| opts.language.as_ref())
            .cloned()
            .unwrap_or_default()
    }

    /// Check if the editor is read-only
    pub fn is_readonly(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.readonly)
            .unwrap_or(false)
    }

    /// Count lines in current code
    pub fn get_line_count(&self) -> usize {
        self.value
            .as_ref()
            .map(|code| code.lines().count())
            .unwrap_or(0)
    }

    /// Get language file extension
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
