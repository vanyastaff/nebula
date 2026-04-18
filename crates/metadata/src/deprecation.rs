//! Structured deprecation notice attached to metadata entries.

use semver::Version;
use serde::{Deserialize, Serialize};

/// Deprecation payload describing when a catalog entity became deprecated,
/// when it will be removed, and what replaces it.
///
/// Attached to [`BaseMetadata::deprecation`](crate::BaseMetadata::deprecation);
/// present implies [`MaturityLevel::Deprecated`](crate::MaturityLevel::Deprecated)
/// is usually also set.
///
/// Fields are intentionally permissive strings where a typed value would
/// force premature precision: `sunset` can be an ISO date, a version, or a
/// milestone identifier depending on how the team plans removal.
/// `replacement` is a serialized entity key (e.g. `"http.request.v2"`),
/// not a typed key, because the replacement may live in a different
/// entity family (an `action` deprecated in favor of a `resource`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecationNotice {
    /// Version in which the deprecation was introduced.
    pub since: Version,
    /// When the entity will be removed — ISO date, version string, or
    /// free-form milestone. `None` if no removal date is set yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunset: Option<String>,
    /// Serialized key of the replacement entity, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    /// Human-readable reason shown in the catalog UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl DeprecationNotice {
    /// Build a deprecation notice with only the required `since` version set.
    #[must_use]
    pub fn new(since: Version) -> Self {
        Self {
            since,
            sunset: None,
            replacement: None,
            reason: None,
        }
    }

    /// Declare when the entity is scheduled for removal.
    #[must_use]
    pub fn sunset(mut self, sunset: impl Into<String>) -> Self {
        self.sunset = Some(sunset.into());
        self
    }

    /// Declare the replacement entity key.
    #[must_use]
    pub fn replacement(mut self, key: impl Into<String>) -> Self {
        self.replacement = Some(key.into());
        self
    }

    /// Set the human-readable reason.
    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_notice_roundtrip() {
        let notice = DeprecationNotice::new(Version::new(1, 2, 0));
        let s = serde_json::to_string(&notice).unwrap();
        // `sunset`/`replacement`/`reason` skip-serialize when None.
        assert_eq!(s, r#"{"since":"1.2.0"}"#);
        let back: DeprecationNotice = serde_json::from_str(&s).unwrap();
        assert_eq!(back, notice);
    }

    #[test]
    fn full_notice_builder() {
        let notice = DeprecationNotice::new(Version::new(2, 0, 0))
            .sunset("2026-07-01")
            .replacement("http.request.v2")
            .reason("superseded by unified HTTP node");
        assert_eq!(notice.sunset.as_deref(), Some("2026-07-01"));
        assert_eq!(notice.replacement.as_deref(), Some("http.request.v2"));
    }
}
