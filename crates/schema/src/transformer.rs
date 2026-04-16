use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Value transformer applied before validation/runtime use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
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
}

impl Transformer {
    /// Apply this transformer to a JSON value.
    ///
    /// String-oriented transformers pass through non-string values unchanged.
    pub fn apply(&self, value: &Value) -> Value {
        match self {
            Self::Trim => apply_to_string(value, |text| text.trim().to_owned()),
            Self::Lowercase => apply_to_string(value, |text| text.to_lowercase()),
            Self::Uppercase => apply_to_string(value, |text| text.to_uppercase()),
            Self::Replace { from, to } => {
                apply_to_string(value, |text| text.replace(from.as_str(), to.as_str()))
            },
        }
    }
}

fn apply_to_string(value: &Value, transform: impl FnOnce(&str) -> String) -> Value {
    match value.as_str() {
        Some(text) => Value::String(transform(text)),
        None => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Transformer;

    #[test]
    fn applies_trim_and_replace_to_strings() {
        let trimmed = Transformer::Trim.apply(&json!("  hello  "));
        let replaced = Transformer::Replace {
            from: "hello".to_owned(),
            to: "nebula".to_owned(),
        }
        .apply(&trimmed);

        assert_eq!(replaced, json!("nebula"));
    }

    #[test]
    fn leaves_non_string_values_unchanged() {
        let value = json!(42);
        let transformed = Transformer::Lowercase.apply(&value);
        assert_eq!(transformed, value);
    }
}
