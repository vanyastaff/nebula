use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Supported code editor languages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeLanguage {
    Json,
    Javascript,
    Typescript,
    Python,
    Sql,
    Html,
    Css,
    Xml,
    Yaml,
    Toml,
    Markdown,
    Shell,
    Rust,
    Other(String),
}

/// Options specific to code parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeOptions {
    /// The language for syntax highlighting.
    pub language: CodeLanguage,

    /// Whether to display line numbers in the editor.
    #[serde(default)]
    pub line_numbers: bool,
}

/// A code editor parameter with syntax highlighting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CodeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl CodeParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_minimal_code() {
        let p = CodeParameter::new("script", "Script");
        assert_eq!(p.metadata.key, "script");
        assert!(p.default.is_none());
        assert!(p.options.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = CodeParameter {
            metadata: ParameterMetadata::new("query", "SQL Query"),
            default: Some("SELECT * FROM users".into()),
            options: Some(CodeOptions {
                language: CodeLanguage::Sql,
                line_numbers: true,
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: CodeParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "query");
        assert_eq!(
            deserialized.options.as_ref().unwrap().language,
            CodeLanguage::Sql
        );
        assert!(deserialized.options.as_ref().unwrap().line_numbers);
    }

    #[test]
    fn other_language_serde() {
        let lang = CodeLanguage::Other("graphql".into());
        let json = serde_json::to_string(&lang).unwrap();
        let deserialized: CodeLanguage = serde_json::from_str(&json).unwrap();
        assert_eq!(lang, deserialized);
    }
}
