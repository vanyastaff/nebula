//! Pre-validation value transformers with regex cache.

use std::sync::{Arc, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// Value transformer applied before validation/runtime use.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Extract a regex capture group.
    Regex {
        /// The regex pattern to apply.
        pattern: String,
        /// Which capture group to extract (0 = full match).
        #[serde(default)]
        group: usize,
        /// Lazily compiled regex — skipped by serde.
        #[serde(skip)]
        cache: Arc<OnceLock<Option<Regex>>>,
    },
}

impl PartialEq for Transformer {
    fn eq(&self, other: &Self) -> bool {
        use Transformer::{Lowercase, Regex, Replace, Trim, Uppercase};
        match (self, other) {
            (Trim, Trim) | (Lowercase, Lowercase) | (Uppercase, Uppercase) => true,
            (Replace { from: a1, to: a2 }, Replace { from: b1, to: b2 }) => a1 == b1 && a2 == b2,
            (
                Regex {
                    pattern: p1,
                    group: g1,
                    ..
                },
                Regex {
                    pattern: p2,
                    group: g2,
                    ..
                },
            ) => p1 == p2 && g1 == g2,
            _ => false,
        }
    }
}

impl Transformer {
    /// Apply this transformer. String transformers pass non-string values through.
    pub fn apply(&self, value: &Value) -> Value {
        match self {
            Self::Trim => string(value, |t| t.trim().to_owned()),
            Self::Lowercase => string(value, str::to_lowercase),
            Self::Uppercase => string(value, str::to_uppercase),
            Self::Replace { from, to } => string(value, |t| t.replace(from.as_str(), to.as_str())),
            Self::Regex {
                pattern,
                group,
                cache,
            } => string(value, |t| {
                let re = cache.get_or_init(|| match Regex::new(pattern) {
                    Ok(compiled) => Some(compiled),
                    Err(error) => {
                        warn!(
                            regex_pattern = %pattern,
                            %error,
                            "invalid regex pattern in transformer; falling back to original value"
                        );
                        None
                    },
                });
                re.as_ref()
                    .and_then(|compiled| compiled.captures(t))
                    .and_then(|captures| captures.get(*group))
                    .map_or_else(|| t.to_owned(), |matched| matched.as_str().to_owned())
            }),
        }
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
        let t = Transformer::Regex {
            pattern: r"^(\d+)-".into(),
            group: 1,
            cache: Arc::default(),
        };
        assert_eq!(t.apply(&json!("42-abc")), json!("42"));
        assert_eq!(t.apply(&json!("no-match")), json!("no-match"));
    }

    #[test]
    fn regex_cache_compiles_once() {
        let t = Transformer::Regex {
            pattern: r"(\w+)".into(),
            group: 0,
            cache: Arc::default(),
        };
        let _ = t.apply(&json!("abc"));
        let _ = t.apply(&json!("def"));
    }

    #[test]
    fn invalid_regex_pattern_falls_back_to_original_value() {
        let t = Transformer::Regex {
            pattern: "(".into(),
            group: 0,
            cache: Arc::default(),
        };

        assert_eq!(t.apply(&json!("kept")), json!("kept"));
    }

    #[test]
    fn non_string_value_passes_through() {
        assert_eq!(Transformer::Lowercase.apply(&json!(42)), json!(42));
    }
}
