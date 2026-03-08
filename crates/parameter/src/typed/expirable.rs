//! Generic Expirable parameter for TTL-wrapped values.

use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::expirable::ExpirableOptions;
use crate::validation::ValidationRule;

/// A TTL wrapper around another parameter.
///
/// Runtime state (expires_at) belongs in the value layer, not schema.
/// Use case: OAuth tokens, temporary credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expirable {
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

impl Expirable {
    /// Create a builder for expirable parameter.
    #[must_use]
    pub fn builder(key: impl Into<String>, inner: ParameterDef) -> ExpirableBuilder {
        ExpirableBuilder::new(key, inner)
    }

    /// Create a minimal expirable parameter.
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

/// Builder for `Expirable`.
#[derive(Debug)]
pub struct ExpirableBuilder {
    metadata: ParameterMetadata,
    inner: Box<ParameterDef>,
    options: Option<ExpirableOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl ExpirableBuilder {
    fn new(key: impl Into<String>, inner: ParameterDef) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            inner: Box::new(inner),
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.metadata.name = label.into();
        self
    }

    /// Set description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }

    /// Set TTL in seconds.
    #[must_use]
    pub fn ttl_seconds(mut self, ttl: u64) -> Self {
        self.options
            .get_or_insert_with(ExpirableOptions::default)
            .ttl_seconds = Some(ttl);
        self
    }

    /// Enable/disable auto refresh.
    #[must_use]
    pub fn auto_refresh(mut self, enabled: bool) -> Self {
        self.options
            .get_or_insert_with(ExpirableOptions::default)
            .auto_refresh = enabled;
        self
    }

    /// Set refresh lead time in seconds.
    #[must_use]
    pub fn refresh_before_seconds(mut self, seconds: u64) -> Self {
        self.options
            .get_or_insert_with(ExpirableOptions::default)
            .refresh_before_seconds = Some(seconds);
        self
    }

    /// Add validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build expirable parameter.
    #[must_use]
    pub fn build(self) -> Expirable {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Expirable {
            metadata,
            inner: self.inner,
            options: self.options,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SecretParameter;

    #[test]
    fn builder_creates_expirable() {
        let inner = ParameterDef::Secret(SecretParameter::new("token", "Token"));
        let p = Expirable::builder("oauth_token", inner)
            .label("OAuth Token")
            .ttl_seconds(3600)
            .auto_refresh(true)
            .refresh_before_seconds(300)
            .build();

        assert_eq!(p.metadata.key, "oauth_token");
        assert_eq!(p.metadata.name, "OAuth Token");
        assert_eq!(p.inner.key(), "token");
        assert_eq!(p.options.as_ref().and_then(|o| o.ttl_seconds), Some(3600));
        assert_eq!(
            p.options.as_ref().and_then(|o| o.refresh_before_seconds),
            Some(300)
        );
    }
}
