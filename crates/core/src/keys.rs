use std::fmt;
use std::str::FromStr;

pub use domain_key::KeyParseError;
use domain_key::{define_domain, key_type};
use serde::{Deserialize, Serialize};

define_domain!(PrameterDomain, "parameter");
key_type!(ParameterKey, PrameterDomain);

define_domain!(CredentialDomain, "credential");
key_type!(CredentialKey, CredentialDomain);

/// Maximum allowed length for a [`PluginKey`].
const PLUGIN_KEY_MAX_LEN: usize = 64;

/// Errors from constructing a [`PluginKey`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginKeyError {
    /// The input was empty or contained only whitespace.
    #[error("plugin key cannot be empty or whitespace")]
    Empty,
    /// The normalized key contains characters other than `a-z` and `_`.
    #[error("plugin key contains invalid characters (only a-z and _ allowed)")]
    InvalidCharacters,
    /// The normalized key exceeds [`PLUGIN_KEY_MAX_LEN`] characters.
    #[error("plugin key exceeds maximum length of {PLUGIN_KEY_MAX_LEN} characters")]
    TooLong,
}

/// A normalized, validated identifier for a plugin type.
///
/// Normalization rules:
/// - Leading/trailing whitespace is trimmed.
/// - The string is lowercased.
/// - Whitespace and hyphens are replaced with underscores.
/// - Consecutive underscores are collapsed to one.
/// - Leading/trailing underscores are stripped.
///
/// After normalization the key must:
/// - Be non-empty.
/// - Contain only `a-z` and `_`.
/// - Be at most 64 characters long.
///
/// # Examples
///
/// ```
/// use nebula_core::PluginKey;
///
/// let key: PluginKey = "HTTP Request".parse().unwrap();
/// assert_eq!(key.as_str(), "http_request");
///
/// let key: PluginKey = " My--Cool  Plugin ".parse().unwrap();
/// assert_eq!(key.as_str(), "my_cool_plugin");
/// ```
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginKey(String);

impl PluginKey {
    /// Create a new `PluginKey`, normalizing and validating the input.
    pub fn new(raw: &str) -> Result<Self, PluginKeyError> {
        let normalized: String = raw
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_whitespace() || c == '-' {
                    '_'
                } else {
                    c
                }
            })
            .collect();

        // Collapse consecutive underscores and strip leading/trailing ones.
        let collapsed = collapse_underscores(&normalized);

        if collapsed.is_empty() {
            return Err(PluginKeyError::Empty);
        }
        if !collapsed
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
        {
            return Err(PluginKeyError::InvalidCharacters);
        }
        if collapsed.len() > PLUGIN_KEY_MAX_LEN {
            return Err(PluginKeyError::TooLong);
        }

        Ok(Self(collapsed))
    }

    /// Return the inner string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Collapse runs of underscores and trim leading/trailing underscores.
fn collapse_underscores(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_underscore = true; // treat start as "previous was _" to skip leading
    for c in s.chars() {
        if c == '_' {
            if !prev_underscore {
                out.push('_');
            }
            prev_underscore = true;
        } else {
            out.push(c);
            prev_underscore = false;
        }
    }
    // Strip trailing underscore.
    if out.ends_with('_') {
        out.pop();
    }
    out
}

impl fmt::Display for PluginKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PluginKey {
    type Err = PluginKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for PluginKey {
    type Error = PluginKeyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for PluginKey {
    type Error = PluginKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<PluginKey> for String {
    fn from(key: PluginKey) -> Self {
        key.0
    }
}

impl AsRef<str> for PluginKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for PluginKey {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for PluginKey {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for PluginKey {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace_and_case() {
        let key: PluginKey = "HTTP Request".parse().unwrap();
        assert_eq!(key.as_str(), "http_request");
    }

    #[test]
    fn normalizes_hyphens() {
        let key: PluginKey = "my-cool-plugin".parse().unwrap();
        assert_eq!(key.as_str(), "my_cool_plugin");
    }

    #[test]
    fn collapses_underscores() {
        let key: PluginKey = "a___b".parse().unwrap();
        assert_eq!(key.as_str(), "a_b");
    }

    #[test]
    fn strips_leading_trailing_underscores() {
        let key: PluginKey = "___hello___".parse().unwrap();
        assert_eq!(key.as_str(), "hello");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let key: PluginKey = "  slack  ".parse().unwrap();
        assert_eq!(key.as_str(), "slack");
    }

    #[test]
    fn complex_normalization() {
        let key: PluginKey = " My--Cool  Plugin ".parse().unwrap();
        assert_eq!(key.as_str(), "my_cool_plugin");
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(PluginKey::new(""), Err(PluginKeyError::Empty));
        assert_eq!(PluginKey::new("   "), Err(PluginKeyError::Empty));
        assert_eq!(PluginKey::new("___"), Err(PluginKeyError::Empty));
    }

    #[test]
    fn rejects_invalid_characters() {
        assert_eq!(
            PluginKey::new("hello!"),
            Err(PluginKeyError::InvalidCharacters)
        );
        assert_eq!(
            PluginKey::new("plugin@1"),
            Err(PluginKeyError::InvalidCharacters)
        );
        assert_eq!(
            PluginKey::new("a.b"),
            Err(PluginKeyError::InvalidCharacters)
        );
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(65);
        assert_eq!(PluginKey::new(&long), Err(PluginKeyError::TooLong));
    }

    #[test]
    fn accepts_max_length() {
        let exact = "a".repeat(64);
        assert!(PluginKey::new(&exact).is_ok());
    }

    #[test]
    fn display_and_equality() {
        let key: PluginKey = "slack".parse().unwrap();
        assert_eq!(key.to_string(), "slack");
        assert_eq!(key, "slack");
        assert_eq!(key, *"slack");
        assert_eq!(key, "slack".to_string());
    }

    #[test]
    fn serde_roundtrip() {
        let key: PluginKey = "http_request".parse().unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, "\"http_request\"");

        let back: PluginKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, key);
    }

    #[test]
    fn serde_normalizes_on_deserialize() {
        let back: PluginKey = serde_json::from_str("\"HTTP Request\"").unwrap();
        assert_eq!(back.as_str(), "http_request");
    }

    #[test]
    fn serde_rejects_invalid() {
        let result: Result<PluginKey, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn try_from_str() {
        let key = PluginKey::try_from("hello_world").unwrap();
        assert_eq!(key.as_str(), "hello_world");
    }

    #[test]
    fn try_from_string() {
        let key = PluginKey::try_from("Hello World".to_string()).unwrap();
        assert_eq!(key.as_str(), "hello_world");
    }

    #[test]
    fn into_string() {
        let key: PluginKey = "slack".parse().unwrap();
        let s: String = key.into();
        assert_eq!(s, "slack");
    }
}
