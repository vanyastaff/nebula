//! String transformations whose configuration is checked before use.

use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::ValidationError;

/// Value transformer applied before validation/runtime use.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transformer {
    /// Trim surrounding whitespace.
    Trim,
    /// Convert string to lowercase.
    Lowercase,
    /// Convert string to uppercase.
    Uppercase,
    /// Replace substring occurrences.
    Replace {
        /// Source string.
        from: String,
        /// Replacement string.
        to: String,
    },
    /// Extract a capture group from a checked regular expression.
    Regex(RegexCapture),
}

/// A compiled pattern bound to a capture index that exists in that pattern.
///
/// Construction and deserialization reject malformed patterns and unavailable
/// group indices. The compiled engine is retained, and diagnostic output omits
/// pattern text. Serialization preserves the authored pattern and group.
#[derive(Clone, Serialize)]
pub struct RegexCapture {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: Regex,
    group: usize,
}

impl Transformer {
    /// Construct a regex extraction transformer, compiling its pattern once.
    ///
    /// Group zero selects the entire match.
    ///
    /// # Errors
    ///
    /// Returns `transformer.invalid_pattern` for a malformed or oversized regex,
    /// or `transformer.invalid_capture_group` when the group does not exist.
    pub fn regex(pattern: &str, group: usize) -> Result<Self, ValidationError> {
        RegexCapture::new(pattern, group).map(Self::Regex)
    }

    /// Apply this transformer. String transformers pass non-string values through.
    #[must_use]
    pub fn apply(&self, value: &Value) -> Value {
        match self {
            Self::Trim => string(value, |t| t.trim().to_owned()),
            Self::Lowercase => string(value, str::to_lowercase),
            Self::Uppercase => string(value, str::to_uppercase),
            Self::Replace { from, to } => string(value, |t| t.replace(from.as_str(), to.as_str())),
            Self::Regex(capture) => string(value, |text| capture.apply(text)),
        }
    }
}

impl RegexCapture {
    /// Compile a pattern and bind an existing capture group, including group zero.
    ///
    /// # Errors
    ///
    /// Returns `transformer.invalid_pattern` for a malformed or oversized regex,
    /// or `transformer.invalid_capture_group` when the group does not exist.
    #[tracing::instrument(level = "debug", skip(pattern), fields(group))]
    pub fn new(pattern: &str, group: usize) -> Result<Self, ValidationError> {
        let pattern = Regex::new(pattern).map_err(|error| {
            ValidationError::builder("transformer.invalid_pattern")
                .message("regex transformer pattern could not be compiled")
                .private_source(error)
                .build()
        })?;
        let capture_count = pattern.captures_len();
        if group >= capture_count {
            return Err(
                ValidationError::builder("transformer.invalid_capture_group")
                    .message("regex transformer capture group does not exist")
                    .param("group", group)
                    .param("capture_count", capture_count)
                    .build(),
            );
        }
        Ok(Self { pattern, group })
    }

    /// Borrow the authored pattern for explicit configuration access.
    #[must_use]
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    /// The checked capture index; zero selects the entire match.
    #[must_use]
    pub const fn group(&self) -> usize {
        self.group
    }

    fn apply(&self, value: &str) -> String {
        self.pattern
            .captures(value)
            .and_then(|captures| captures.get(self.group))
            .map_or_else(|| value.to_owned(), |matched| matched.as_str().to_owned())
    }
}

impl std::fmt::Debug for RegexCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegexCapture")
            .field("group", &self.group)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RegexCapture {
    fn eq(&self, other: &Self) -> bool {
        self.pattern() == other.pattern() && self.group == other.group
    }
}

impl Eq for RegexCapture {}

fn serialize_pattern<S: Serializer>(pattern: &Regex, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(pattern.as_str())
}

impl<'de> Deserialize<'de> for RegexCapture {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Configuration {
            pattern: String,
            #[serde(default)]
            group: usize,
        }

        let configuration = Configuration::deserialize(deserializer)?;
        Self::new(&configuration.pattern, configuration.group).map_err(serde::de::Error::custom)
    }
}

fn string(value: &Value, f: impl FnOnce(&str) -> String) -> Value {
    value
        .as_str()
        .map_or_else(|| value.clone(), |s| Value::String(f(s)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn trim_on_string() {
        let out = Transformer::Trim.apply(&json!("  hi  "));
        assert_eq!(out, json!("hi"));
    }

    #[test]
    fn regex_extract_group() {
        let t = Transformer::regex(r"^(\d+)-", 1).unwrap();
        assert_eq!(t.apply(&json!("42-abc")), json!("42"));
        assert_eq!(t.apply(&json!("no-match")), json!("no-match"));
    }

    #[test]
    fn checked_regex_is_reusable() {
        let transformer = Transformer::regex(r"(\w+)", 0).unwrap();
        let cloned = transformer.clone();
        assert_eq!(transformer, cloned);
        assert_eq!(transformer.apply(&json!("abc")), json!("abc"));
        assert_eq!(cloned.apply(&json!("def")), json!("def"));
    }

    #[test]
    fn invalid_regex_pattern_is_rejected_at_construction() {
        let error = Transformer::regex("(", 0).unwrap_err();
        assert_eq!(error.code(), "transformer.invalid_pattern");
    }

    #[test]
    fn non_string_value_passes_through() {
        assert_eq!(Transformer::Lowercase.apply(&json!(42)), json!(42));
    }
}
