use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to expirable parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpirableOptions {
    /// Time-to-live in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,

    /// Whether to automatically refresh before expiry.
    #[serde(default)]
    pub auto_refresh: bool,

    /// Seconds before expiry to trigger a refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_before_seconds: Option<u64>,
}

/// A TTL wrapper around another parameter.
///
/// Runtime state (expires_at) belongs in the value layer, not schema.
/// Use case: OAuth tokens, temporary credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpirableParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The wrapped parameter (boxed for recursion).
    pub inner: Box<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ExpirableOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl ExpirableParameter {
    /// Create a new expirable parameter wrapping an inner parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>, inner: ParameterDef) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            inner: Box::new(inner),
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SecretParameter;

    #[test]
    fn new_creates_minimal_expirable() {
        let inner = ParameterDef::Secret(SecretParameter::new("token", "Token"));
        let p = ExpirableParameter::new("access_token", "Access Token", inner);
        assert_eq!(p.metadata.key, "access_token");
        assert_eq!(p.metadata.name, "Access Token");
        assert_eq!(p.inner.key(), "token");
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let inner = ParameterDef::Secret(SecretParameter::new("token", "Token"));
        let p = ExpirableParameter {
            metadata: ParameterMetadata::new("oauth_token", "OAuth Token"),
            inner: Box::new(inner),
            options: Some(ExpirableOptions {
                ttl_seconds: Some(3600),
                auto_refresh: true,
                refresh_before_seconds: Some(300),
            }),
            display: None,
            validation: Vec::new(),
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: ExpirableParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "oauth_token");
        assert_eq!(deserialized.inner.key(), "token");
        let opts = deserialized.options.unwrap();
        assert_eq!(opts.ttl_seconds, Some(3600));
        assert!(opts.auto_refresh);
        assert_eq!(opts.refresh_before_seconds, Some(300));
    }
}
