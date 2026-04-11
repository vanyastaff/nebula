//! Plugin metadata and builder.

use nebula_core::PluginKey;
use serde::{Deserialize, Serialize};

use crate::PluginError;

/// Normalize a raw plugin key string: ASCII uppercase → lowercase, spaces → underscores.
///
/// This lets callers use human-readable labels like `"HTTP Request"` and receive
/// the canonical form `"http_request"`.
pub(crate) fn normalize_key(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "_")
}

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
    /// Plugin author or organization name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    /// SPDX license identifier (e.g. `"MIT"`, `"Apache-2.0"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license: Option<String>,
    /// Homepage URL for the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    homepage: Option<String>,
    /// Source repository URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    /// Minimum Nebula engine version required by this plugin (semver string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nebula_version: Option<String>,
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
            author: None,
            license: None,
            homepage: None,
            repository: None,
            nebula_version: None,
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

    /// Plugin author or organization name.
    #[inline]
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// SPDX license identifier (e.g. `"MIT"`, `"Apache-2.0"`).
    #[inline]
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// Homepage URL for the plugin.
    #[inline]
    pub fn homepage(&self) -> Option<&str> {
        self.homepage.as_deref()
    }

    /// Source repository URL.
    #[inline]
    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }

    /// Minimum Nebula engine version required by this plugin (semver string).
    #[inline]
    pub fn nebula_version(&self) -> Option<&str> {
        self.nebula_version.as_deref()
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
    author: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    nebula_version: Option<String>,
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

    /// Set the author or organization name.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::PluginMetadata;
    ///
    /// let meta = PluginMetadata::builder("slack", "Slack")
    ///     .author("Acme Corp")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(meta.author(), Some("Acme Corp"));
    /// ```
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set the SPDX license identifier (e.g. `"MIT"`, `"Apache-2.0"`).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::PluginMetadata;
    ///
    /// let meta = PluginMetadata::builder("slack", "Slack")
    ///     .license("Apache-2.0")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(meta.license(), Some("Apache-2.0"));
    /// ```
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Set the homepage URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::PluginMetadata;
    ///
    /// let meta = PluginMetadata::builder("slack", "Slack")
    ///     .homepage("https://example.com")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(meta.homepage(), Some("https://example.com"));
    /// ```
    pub fn homepage(mut self, url: impl Into<String>) -> Self {
        self.homepage = Some(url.into());
        self
    }

    /// Set the source repository URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::PluginMetadata;
    ///
    /// let meta = PluginMetadata::builder("slack", "Slack")
    ///     .repository("https://github.com/example/slack-plugin")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(
    ///     meta.repository(),
    ///     Some("https://github.com/example/slack-plugin")
    /// );
    /// ```
    pub fn repository(mut self, url: impl Into<String>) -> Self {
        self.repository = Some(url.into());
        self
    }

    /// Set the minimum required Nebula engine version (semver string).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::PluginMetadata;
    ///
    /// let meta = PluginMetadata::builder("slack", "Slack")
    ///     .nebula_version("0.5.0")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(meta.nebula_version(), Some("0.5.0"));
    /// ```
    pub fn nebula_version(mut self, version: impl Into<String>) -> Self {
        self.nebula_version = Some(version.into());
        self
    }

    /// Validate and build the metadata.
    ///
    /// The raw key is normalized before validation: spaces become underscores and
    /// ASCII letters are lowercased, so `"HTTP Request"` → `"http_request"`.
    pub fn build(self) -> Result<PluginMetadata, PluginError> {
        let key: PluginKey = normalize_key(&self.key)
            .parse()
            .map_err(PluginError::InvalidKey)?;

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
            author: self.author,
            license: self.license,
            homepage: self.homepage,
            repository: self.repository,
            nebula_version: self.nebula_version,
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

    #[test]
    fn new_optional_fields_default_to_none() {
        let meta = PluginMetadata::builder("slack", "Slack").build().unwrap();
        assert!(meta.author().is_none());
        assert!(meta.license().is_none());
        assert!(meta.homepage().is_none());
        assert!(meta.repository().is_none());
        assert!(meta.nebula_version().is_none());
    }

    #[test]
    fn new_optional_fields_via_builder() {
        let meta = PluginMetadata::builder("slack", "Slack")
            .author("Acme Corp")
            .license("MIT")
            .homepage("https://example.com")
            .repository("https://github.com/acme/slack-plugin")
            .nebula_version("0.5.0")
            .build()
            .unwrap();

        assert_eq!(meta.author(), Some("Acme Corp"));
        assert_eq!(meta.license(), Some("MIT"));
        assert_eq!(meta.homepage(), Some("https://example.com"));
        assert_eq!(
            meta.repository(),
            Some("https://github.com/acme/slack-plugin")
        );
        assert_eq!(meta.nebula_version(), Some("0.5.0"));
    }

    #[test]
    fn new_optional_fields_serde_roundtrip() {
        let meta = PluginMetadata::builder("slack", "Slack")
            .author("Acme Corp")
            .license("Apache-2.0")
            .build()
            .unwrap();

        let json = serde_json::to_string(&meta).unwrap();
        let back: PluginMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(back.author(), Some("Acme Corp"));
        assert_eq!(back.license(), Some("Apache-2.0"));
        assert!(back.homepage().is_none());
    }

    #[test]
    fn new_optional_fields_omitted_from_json_when_none() {
        let meta = PluginMetadata::builder("slack", "Slack").build().unwrap();
        let json = serde_json::to_string(&meta).unwrap();
        // None fields should be skipped entirely (skip_serializing_if = "Option::is_none")
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("author"));
        assert!(!obj.contains_key("license"));
        assert!(!obj.contains_key("homepage"));
        assert!(!obj.contains_key("repository"));
        assert!(!obj.contains_key("nebula_version"));
    }
}
