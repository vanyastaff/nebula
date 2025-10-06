use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

/// Parameter for code input with syntax highlighting and validation
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct CodeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<CodeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
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

impl ParameterType for CodeParameter {
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
    type Value = String;

    fn get_value(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_value_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default_value(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear_value(&mut self) {
        self.value = None;
    }

    fn get_parameter_value(&self) -> Option<ParameterValue> {
        self.value
            .as_ref()
            .map(|s| ParameterValue::Value(nebula_value::Value::text(s.clone())))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                let code = s.to_string();
                // Perform language-specific validation if possible
                if self.is_valid_code(&code) {
                    self.value = Some(code);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: "Code contains syntax errors".to_string(),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic code
                self.value = Some(expr);
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
    fn is_empty_value(&self, value: &Self::Value) -> bool {
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
    /// Validate code syntax based on language
    fn is_valid_code(&self, code: &str) -> bool {
        if code.is_empty() {
            return true; // Empty code is valid
        }

        // Check for expressions (start with {{ and end with }})
        if code.starts_with("{{") && code.ends_with("}}") {
            return true;
        }

        // Basic validation based on language
        if let Some(options) = &self.options {
            if let Some(language) = &options.language {
                return self.validate_language_syntax(code, language);
            }
        }

        // No specific language validation, accept all
        true
    }

    /// Basic language-specific syntax validation
    fn validate_language_syntax(&self, code: &str, language: &CodeLanguage) -> bool {
        match language {
            CodeLanguage::Json => {
                // Try to parse as JSON
                serde_json::from_str::<serde_json::Value>(code).is_ok()
            }
            CodeLanguage::JavaScript | CodeLanguage::TypeScript => {
                // Basic JS/TS validation - check for unclosed braces/brackets
                self.validate_balanced_brackets(code)
            }
            CodeLanguage::Python => {
                // Basic Python validation - check indentation consistency
                self.validate_python_indentation(code)
            }
            _ => {
                // For other languages, just check balanced brackets
                self.validate_balanced_brackets(code)
            }
        }
    }

    /// Check if brackets, braces, and parentheses are balanced
    fn validate_balanced_brackets(&self, code: &str) -> bool {
        let mut stack = Vec::new();
        let mut in_string = false;
        let mut in_char = false;
        let mut escaped = false;

        for ch in code.chars() {
            if escaped {
                escaped = false;
                continue;
            }

            if ch == '\\' {
                escaped = true;
                continue;
            }

            if in_string {
                if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            if in_char {
                if ch == '\'' {
                    in_char = false;
                }
                continue;
            }

            match ch {
                '"' => in_string = true,
                '\'' => in_char = true,
                '(' => stack.push(')'),
                '[' => stack.push(']'),
                '{' => stack.push('}'),
                ')' | ']' | '}' => {
                    if stack.pop() != Some(ch) {
                        return false;
                    }
                }
                _ => {}
            }
        }

        stack.is_empty() && !in_string && !in_char
    }

    /// Basic Python indentation validation
    fn validate_python_indentation(&self, code: &str) -> bool {
        let mut indent_stack = vec![0];

        for line in code.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let indent_level = line.len() - line.trim_start().len();

            // Check if indentation is consistent with stack
            if indent_level > *indent_stack.last().unwrap() {
                indent_stack.push(indent_level);
            } else {
                // Pop stack until we find matching indentation
                while let Some(&last_indent) = indent_stack.last() {
                    if last_indent <= indent_level {
                        break;
                    }
                    indent_stack.pop();
                }

                // Check if we found a matching indentation level
                if indent_stack.last() != Some(&indent_level) {
                    return false;
                }
            }
        }

        true
    }

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
