//! Value transformers that modify parameter values before consumption.
//!
//! Transformers operate on [`serde_json::Value`] and are composable via
//! [`Transformer::Chain`] and [`Transformer::FirstMatch`].

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Module-level cache for compiled regex patterns. Avoids recompilation
/// on every `Transformer::Regex::apply()` call.
static REGEX_CACHE: LazyLock<Mutex<HashMap<String, Option<Regex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_or_compile_regex(pattern: &str) -> Option<Regex> {
    let mut cache = REGEX_CACHE.lock().expect("regex cache poisoned");
    if let Some(cached) = cache.get(pattern) {
        return cached.clone();
    }
    let compiled = Regex::new(pattern).ok();
    cache.insert(pattern.to_string(), compiled.clone());
    compiled
}

/// Returns the default capture group index for regex transformers.
fn default_group() -> usize {
    1
}

/// A composable value transformer that modifies parameter values.
///
/// Transformers operate on [`Value`] and follow these rules:
/// - String operations (`Trim`, `Lowercase`, etc.) pass through non-string values unchanged.
/// - `Regex` compiles the pattern on each call and extracts a capture group.
/// - `JsonPath` walks dot-separated segments via [`Value::get`].
/// - `Chain` applies transformers in sequence.
/// - `FirstMatch` returns the result of the first transformer that changes the value.
///
/// # Examples
///
/// ```
/// use nebula_parameter::transformer::Transformer;
/// use serde_json::json;
///
/// let t = Transformer::Trim;
/// assert_eq!(t.apply(&json!("  hello  ")), json!("hello"));
///
/// let chain = Transformer::Chain {
///     transformers: vec![Transformer::Trim, Transformer::Lowercase],
/// };
/// assert_eq!(chain.apply(&json!("  HELLO  ")), json!("hello"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transformer {
    /// Strips leading and trailing whitespace from string values.
    Trim,
    /// Converts string values to lowercase.
    Lowercase,
    /// Converts string values to uppercase.
    Uppercase,
    /// Replaces all occurrences of `from` with `to` in string values.
    Replace {
        /// The substring to search for.
        from: String,
        /// The replacement string.
        to: String,
    },
    /// Strips a prefix from string values if present.
    StripPrefix {
        /// The prefix to remove.
        prefix: String,
    },
    /// Strips a suffix from string values if present.
    StripSuffix {
        /// The suffix to remove.
        suffix: String,
    },
    /// Extracts a capture group from a regex match on string values.
    ///
    /// If the pattern does not match or the group is missing, the original
    /// value is returned unchanged.
    Regex {
        /// The regex pattern to match against.
        pattern: String,
        /// The capture group index to extract (defaults to 1).
        #[serde(default = "default_group")]
        group: usize,
    },
    /// Walks a dot-separated path into a JSON object.
    ///
    /// For example, `path: "data.name"` extracts `value["data"]["name"]`.
    /// If any segment is missing, the original value is returned unchanged.
    JsonPath {
        /// Dot-separated path segments (e.g. `"data.name"`).
        path: String,
    },
    /// Applies a sequence of transformers left-to-right.
    Chain {
        /// The ordered list of transformers to apply.
        transformers: Vec<Transformer>,
    },
    /// Returns the result of the first transformer that changes the value.
    ///
    /// If no transformer changes the value, the original is returned.
    FirstMatch {
        /// The candidate transformers to try in order.
        transformers: Vec<Transformer>,
    },
}

impl Transformer {
    /// Applies this transformer to a JSON value, returning the transformed result.
    ///
    /// String-oriented transformers pass through non-string values unchanged.
    /// Composite transformers (`Chain`, `FirstMatch`) delegate to their children.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::transformer::Transformer;
    /// use serde_json::json;
    ///
    /// let t = Transformer::Replace {
    ///     from: "world".to_string(),
    ///     to: "rust".to_string(),
    /// };
    /// assert_eq!(t.apply(&json!("hello world")), json!("hello rust"));
    ///
    /// // Non-strings pass through unchanged.
    /// assert_eq!(t.apply(&json!(42)), json!(42));
    /// ```
    #[must_use]
    pub fn apply(&self, value: &Value) -> Value {
        match self {
            Self::Trim => apply_to_string(value, |s| s.trim().to_owned()),
            Self::Lowercase => apply_to_string(value, |s| s.to_lowercase()),
            Self::Uppercase => apply_to_string(value, |s| s.to_uppercase()),
            Self::Replace { from, to } => {
                apply_to_string(value, |s| s.replace(from.as_str(), to.as_str()))
            }
            Self::StripPrefix { prefix } => apply_to_string(value, |s| {
                s.strip_prefix(prefix.as_str()).unwrap_or(s).to_owned()
            }),
            Self::StripSuffix { suffix } => apply_to_string(value, |s| {
                s.strip_suffix(suffix.as_str()).unwrap_or(s).to_owned()
            }),
            Self::Regex { pattern, group } => {
                let Some(s) = value.as_str() else {
                    return value.clone();
                };
                let Some(re) = get_or_compile_regex(pattern) else {
                    return value.clone();
                };
                match re.captures(s) {
                    Some(caps) => caps
                        .get(*group)
                        .map(|m| Value::String(m.as_str().to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    None => value.clone(),
                }
            }
            Self::JsonPath { path } => {
                let mut current = value;
                for segment in path.split('.') {
                    match current.get(segment) {
                        Some(next) => current = next,
                        None => return value.clone(),
                    }
                }
                current.clone()
            }
            Self::Chain { transformers } => {
                let mut current = value.clone();
                for t in transformers {
                    current = t.apply(&current);
                }
                current
            }
            Self::FirstMatch { transformers } => {
                for t in transformers {
                    let result = t.apply(value);
                    if result != *value {
                        return result;
                    }
                }
                value.clone()
            }
        }
    }
}

/// Helper that applies a string transformation, passing non-strings through.
fn apply_to_string(value: &Value, f: impl FnOnce(&str) -> String) -> Value {
    match value.as_str() {
        Some(s) => Value::String(f(s)),
        None => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn trim_strips_whitespace() {
        let t = Transformer::Trim;
        assert_eq!(t.apply(&json!("  hello  ")), json!("hello"));
    }

    #[test]
    fn trim_passes_through_non_string() {
        let t = Transformer::Trim;
        assert_eq!(t.apply(&json!(42)), json!(42));
    }

    #[test]
    fn lowercase_converts_string() {
        let t = Transformer::Lowercase;
        assert_eq!(t.apply(&json!("HELLO")), json!("hello"));
    }

    #[test]
    fn uppercase_converts_string() {
        let t = Transformer::Uppercase;
        assert_eq!(t.apply(&json!("hello")), json!("HELLO"));
    }

    #[test]
    fn replace_substitutes_substring() {
        let t = Transformer::Replace {
            from: "world".into(),
            to: "rust".into(),
        };
        assert_eq!(t.apply(&json!("hello world")), json!("hello rust"));
    }

    #[test]
    fn replace_passes_through_non_string() {
        let t = Transformer::Replace {
            from: "a".into(),
            to: "b".into(),
        };
        assert_eq!(t.apply(&json!(true)), json!(true));
    }

    #[test]
    fn strip_prefix_removes_matching_prefix() {
        let t = Transformer::StripPrefix {
            prefix: "https://".into(),
        };
        assert_eq!(t.apply(&json!("https://example.com")), json!("example.com"));
    }

    #[test]
    fn strip_prefix_no_match_returns_original() {
        let t = Transformer::StripPrefix {
            prefix: "https://".into(),
        };
        assert_eq!(
            t.apply(&json!("http://example.com")),
            json!("http://example.com")
        );
    }

    #[test]
    fn strip_suffix_removes_matching_suffix() {
        let t = Transformer::StripSuffix {
            suffix: ".json".into(),
        };
        assert_eq!(t.apply(&json!("data.json")), json!("data"));
    }

    #[test]
    fn strip_suffix_no_match_returns_original() {
        let t = Transformer::StripSuffix {
            suffix: ".json".into(),
        };
        assert_eq!(t.apply(&json!("data.xml")), json!("data.xml"));
    }

    #[test]
    fn regex_extracts_capture_group() {
        let t = Transformer::Regex {
            pattern: r"(\d+)-(\w+)".into(),
            group: 2,
        };
        assert_eq!(t.apply(&json!("42-hello")), json!("hello"));
    }

    #[test]
    fn regex_default_group_extracts_first_capture() {
        let t = Transformer::Regex {
            pattern: r"id:(\d+)".into(),
            group: 1,
        };
        assert_eq!(t.apply(&json!("id:123")), json!("123"));
    }

    #[test]
    fn regex_no_match_returns_original() {
        let t = Transformer::Regex {
            pattern: r"(\d+)".into(),
            group: 1,
        };
        assert_eq!(t.apply(&json!("no digits here")), json!("no digits here"));
    }

    #[test]
    fn regex_invalid_pattern_returns_original() {
        let t = Transformer::Regex {
            pattern: r"[invalid".into(),
            group: 1,
        };
        assert_eq!(t.apply(&json!("test")), json!("test"));
    }

    #[test]
    fn regex_passes_through_non_string() {
        let t = Transformer::Regex {
            pattern: r"(\d+)".into(),
            group: 1,
        };
        assert_eq!(t.apply(&json!(null)), json!(null));
    }

    #[test]
    fn json_path_walks_nested_object() {
        let t = Transformer::JsonPath {
            path: "data.name".into(),
        };
        let value = json!({"data": {"name": "Alice"}});
        assert_eq!(t.apply(&value), json!("Alice"));
    }

    #[test]
    fn json_path_missing_segment_returns_original() {
        let t = Transformer::JsonPath {
            path: "data.missing".into(),
        };
        let value = json!({"data": {"name": "Alice"}});
        assert_eq!(t.apply(&value), value);
    }

    #[test]
    fn json_path_single_segment() {
        let t = Transformer::JsonPath { path: "key".into() };
        let value = json!({"key": 42});
        assert_eq!(t.apply(&value), json!(42));
    }

    #[test]
    fn chain_applies_in_sequence() {
        let t = Transformer::Chain {
            transformers: vec![Transformer::Trim, Transformer::Uppercase],
        };
        assert_eq!(t.apply(&json!("  hello  ")), json!("HELLO"));
    }

    #[test]
    fn chain_empty_returns_original() {
        let t = Transformer::Chain {
            transformers: vec![],
        };
        assert_eq!(t.apply(&json!("hello")), json!("hello"));
    }

    #[test]
    fn first_match_returns_first_change() {
        let t = Transformer::FirstMatch {
            transformers: vec![
                Transformer::StripPrefix {
                    prefix: "http://".into(),
                },
                Transformer::StripPrefix {
                    prefix: "https://".into(),
                },
            ],
        };
        assert_eq!(t.apply(&json!("https://example.com")), json!("example.com"));
    }

    #[test]
    fn first_match_returns_original_if_nothing_changes() {
        let t = Transformer::FirstMatch {
            transformers: vec![Transformer::StripPrefix {
                prefix: "ftp://".into(),
            }],
        };
        assert_eq!(t.apply(&json!("hello")), json!("hello"));
    }

    #[test]
    fn serde_round_trip() {
        let t = Transformer::Chain {
            transformers: vec![
                Transformer::Trim,
                Transformer::Replace {
                    from: "a".into(),
                    to: "b".into(),
                },
            ],
        };
        let json_str = serde_json::to_string(&t).expect("serialize");
        let deserialized: Transformer = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(t, deserialized);
    }

    #[test]
    fn serde_regex_default_group() {
        let json_str = r#"{"type":"regex","pattern":"(\\d+)"}"#;
        let t: Transformer = serde_json::from_str(json_str).expect("deserialize");
        assert_eq!(
            t,
            Transformer::Regex {
                pattern: r"(\d+)".into(),
                group: 1,
            }
        );
    }
}
