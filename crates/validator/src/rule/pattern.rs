//! Checked regular expressions used by declarative rules.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{MAX_RULE_TEXT_BYTES, RuleBuildError};

/// A compiled rule pattern. Construction and deserialization reject invalid regex.
#[derive(Clone)]
pub struct RulePattern(regex::Regex);

impl std::fmt::Debug for RulePattern {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RulePattern(<protected>)")
    }
}

impl RulePattern {
    /// Compiles a rule pattern.
    ///
    /// # Errors
    /// Returns a redacted error for a malformed or oversized pattern.
    pub fn new(pattern: &str) -> Result<Self, RuleBuildError> {
        if pattern.len() > MAX_RULE_TEXT_BYTES {
            return Err(RuleBuildError::TextLimit {
                limit: MAX_RULE_TEXT_BYTES,
            });
        }
        regex::Regex::new(pattern)
            .map(Self)
            .map_err(|_| RuleBuildError::InvalidPattern)
    }

    /// The original regular expression.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(super) fn is_match(&self, value: &str) -> bool {
        self.0.is_match(value)
    }
}

impl PartialEq for RulePattern {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for RulePattern {}

impl Serialize for RulePattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RulePattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pattern = String::deserialize(deserializer)?;
        Self::new(&pattern).map_err(serde::de::Error::custom)
    }
}
