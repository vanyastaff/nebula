//! Plugin metadata and builder.

use nebula_core::PluginKey;
use serde::{Deserialize, Serialize};

use crate::PluginError;

/// Static metadata describing a plugin type.
///
/// Built via the builder API:
///
/// ```
/// use nebula_plugin::PluginMetadata;
///
/// let meta = PluginMetadata::builder("http_request", "HTTP Request")
///     .description("Make HTTP calls to external APIs")
///     .group(vec!["network".into()])
///     .version(2)
///     .build()
///     .unwrap();
///
/// assert_eq!(meta.key().as_str(), "http_request");
/// assert_eq!(meta.version(), 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    key: PluginKey,
    name: String,
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    group: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn default_version() -> u32 {
    1
}

impl PluginMetadata {
    /// Start building metadata with the minimum required fields.
    pub fn builder(key: impl AsRef<str>, name: impl Into<String>) -> PluginMetadataBuilder {
        PluginMetadataBuilder {
            key: key.as_ref().to_owned(),
            name: name.into(),
            version: 1,
            group: Vec::new(),
            description: String::new(),
            icon: None,
            icon_url: None,
            documentation_url: None,
            color: None,
            tags: Vec::new(),
        }
    }

    /// The normalized key.
    #[inline]
    pub fn key(&self) -> &PluginKey {
        &self.key
    }

    /// Human-readable name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version number (1-based).
    #[inline]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Group hierarchy for UI categorization.
    #[inline]
    pub fn group(&self) -> &[String] {
        &self.group
    }

    /// Short description.
    #[inline]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Optional icon identifier.
    #[inline]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Optional icon URL.
    #[inline]
    pub fn icon_url(&self) -> Option<&str> {
        self.icon_url.as_deref()
    }

    /// Optional documentation URL.
    #[inline]
    pub fn documentation_url(&self) -> Option<&str> {
        self.documentation_url.as_deref()
    }

    /// Optional UI color.
    #[inline]
    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    /// Tags for filtering and categorization.
    #[inline]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// Builder for [`PluginMetadata`].
pub struct PluginMetadataBuilder {
    key: String,
    name: String,
    version: u32,
    group: Vec<String>,
    description: String,
    icon: Option<String>,
    icon_url: Option<String>,
    documentation_url: Option<String>,
    color: Option<String>,
    tags: Vec<String>,
}

impl PluginMetadataBuilder {
    /// Set the version number (defaults to 1).
    pub fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// Set the group hierarchy.
    pub fn group(mut self, group: Vec<String>) -> Self {
        self.group = group;
        self
    }

    /// Set the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set an icon identifier.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set an icon URL.
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon_url = Some(url.into());
        self
    }

    /// Set a documentation URL.
    pub fn documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Set the UI color.
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Set the tags.
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Validate and build the metadata.
    pub fn build(self) -> Result<PluginMetadata, PluginError> {
        let key: PluginKey = self.key.parse().map_err(PluginError::InvalidKey)?;

        Ok(PluginMetadata {
            key,
            name: self.name,
            version: self.version,
            group: self.group,
            description: self.description,
            icon: self.icon,
            icon_url: self.icon_url,
            documentation_url: self.documentation_url,
            color: self.color,
            tags: self.tags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_minimal() {
        let meta = PluginMetadata::builder("slack", "Slack").build().unwrap();
        assert_eq!(meta.key().as_str(), "slack");
        assert_eq!(meta.name(), "Slack");
        assert_eq!(meta.version(), 1);
        assert!(meta.group().is_empty());
        assert!(meta.description().is_empty());
    }

    #[test]
    fn builder_full() {
        let meta = PluginMetadata::builder("http_request", "HTTP Request")
            .version(2)
            .group(vec!["network".into(), "api".into()])
            .description("Make HTTP calls")
            .icon("globe")
            .icon_url("https://example.com/icon.png")
            .documentation_url("https://docs.example.com/http")
            .build()
            .unwrap();

        assert_eq!(meta.version(), 2);
        assert_eq!(meta.group(), &["network", "api"]);
        assert_eq!(meta.icon(), Some("globe"));
        assert_eq!(meta.icon_url(), Some("https://example.com/icon.png"));
        assert_eq!(
            meta.documentation_url(),
            Some("https://docs.example.com/http")
        );
    }

    #[test]
    fn builder_normalizes_key() {
        let meta = PluginMetadata::builder("HTTP Request", "HTTP Request")
            .build()
            .unwrap();
        assert_eq!(meta.key().as_str(), "http_request");
    }

    #[test]
    fn builder_rejects_invalid_key() {
        let result = PluginMetadata::builder("", "Empty").build();
        assert!(result.is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let meta = PluginMetadata::builder("slack", "Slack")
            .version(3)
            .description("Send messages")
            .build()
            .unwrap();

        let json = serde_json::to_string(&meta).unwrap();
        let back: PluginMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(back.key().as_str(), "slack");
        assert_eq!(back.version(), 3);
        assert_eq!(back.description(), "Send messages");
    }
}
