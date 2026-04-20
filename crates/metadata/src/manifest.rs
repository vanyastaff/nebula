//! Plugin manifest — bundle descriptor for a plugin (ADR-0018).
//!
//! A [`PluginManifest`] describes the *container* that bundles actions,
//! credentials, and resources under a versioned identity. It reuses the
//! shared small types from this crate ([`Icon`], [`MaturityLevel`],
//! [`DeprecationNotice`]) but deliberately does **not** compose
//! `BaseMetadata<K>`: a plugin is a container, not a schematized leaf.
//! See `docs/adr/0018-plugin-metadata-to-manifest.md`.
//!
//! This module lives in `nebula-metadata` (moved here from `nebula-plugin`
//! in slice B of the plugin load-path stabilization) so that
//! `nebula-plugin-sdk` — which must have zero engine-side deps per canon
//! §7.1 — can import the canonical bundle descriptor on the plugin-author
//! side.

use nebula_core::PluginKey;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{DeprecationNotice, Icon, MaturityLevel};

/// Errors from [`PluginManifest::builder().build()`](PluginManifestBuilder::build).
#[derive(Debug, thiserror::Error, nebula_error::Classify, PartialEq, Eq)]
pub enum ManifestError {
    /// A required field was missing during construction.
    #[classify(category = "validation", code = "MANIFEST:MISSING_FIELD")]
    #[error("missing required field '{field}' for plugin manifest")]
    MissingRequiredField {
        /// The missing field name.
        field: &'static str,
    },

    /// Plugin key validation failed.
    #[classify(category = "validation", code = "MANIFEST:INVALID_KEY")]
    #[error("invalid plugin key: {0}")]
    InvalidKey(<PluginKey as std::str::FromStr>::Err),
}

/// Normalize a raw plugin key string: ASCII uppercase → lowercase, spaces → underscores.
///
/// This lets callers use human-readable labels like `"HTTP Request"` and receive
/// the canonical form `"http_request"`.
pub fn normalize_key(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "_")
}

fn default_version() -> Version {
    Version::new(1, 0, 0)
}

fn is_default_version(v: &Version) -> bool {
    v == &default_version()
}

fn is_default_maturity(m: &MaturityLevel) -> bool {
    *m == MaturityLevel::default()
}

/// Static manifest describing a plugin bundle (ADR-0018).
///
/// Built via the builder API:
///
/// ```
/// use nebula_metadata::PluginManifest;
/// use semver::Version;
///
/// let manifest = PluginManifest::builder("http_request", "HTTP Request")
///     .description("Make HTTP calls to external APIs")
///     .group(vec!["network".into()])
///     .version(Version::new(2, 0, 0))
///     .build()
///     .unwrap();
///
/// assert_eq!(manifest.key().as_str(), "http_request");
/// assert_eq!(manifest.version(), &Version::new(2, 0, 0));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    key: PluginKey,
    name: String,
    #[serde(
        default = "default_version",
        skip_serializing_if = "is_default_version"
    )]
    version: Version,
    #[serde(default)]
    group: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Icon::is_none")]
    icon: Icon,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "is_default_maturity")]
    maturity: MaturityLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deprecation: Option<DeprecationNotice>,
}

impl PluginManifest {
    /// Start building a manifest with the minimum required fields.
    pub fn builder(key: impl AsRef<str>, name: impl Into<String>) -> PluginManifestBuilder {
        PluginManifestBuilder {
            key: key.as_ref().to_owned(),
            name: name.into(),
            version: default_version(),
            group: Vec::new(),
            description: String::new(),
            icon: Icon::default(),
            color: None,
            tags: Vec::new(),
            author: None,
            license: None,
            homepage: None,
            repository: None,
            nebula_version: None,
            maturity: MaturityLevel::default(),
            deprecation: None,
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

    /// Semver version of the plugin bundle.
    #[inline]
    pub fn version(&self) -> &Version {
        &self.version
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

    /// Catalog icon.
    #[inline]
    pub fn icon(&self) -> &Icon {
        &self.icon
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

    /// Declared maturity level.
    #[inline]
    pub fn maturity(&self) -> MaturityLevel {
        self.maturity
    }

    /// Deprecation notice, if this plugin is being phased out.
    #[inline]
    pub fn deprecation(&self) -> Option<&DeprecationNotice> {
        self.deprecation.as_ref()
    }
}

/// Builder for [`PluginManifest`].
pub struct PluginManifestBuilder {
    key: String,
    name: String,
    version: Version,
    group: Vec<String>,
    description: String,
    icon: Icon,
    color: Option<String>,
    tags: Vec<String>,
    author: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    nebula_version: Option<String>,
    maturity: MaturityLevel,
    deprecation: Option<DeprecationNotice>,
}

impl PluginManifestBuilder {
    /// Set the bundle semver version (defaults to `1.0.0`).
    pub fn version(mut self, version: Version) -> Self {
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

    /// Set the icon directly.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// Convenience: set an inline-identifier icon (e.g. `"github"`, `"🔑"`).
    pub fn inline_icon(mut self, name: impl Into<String>) -> Self {
        self.icon = Icon::inline(name);
        self
    }

    /// Convenience: set a URL-backed icon.
    pub fn url_icon(mut self, url: impl Into<String>) -> Self {
        self.icon = Icon::url(url);
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
    /// use nebula_metadata::PluginManifest;
    ///
    /// let manifest = PluginManifest::builder("slack", "Slack")
    ///     .author("Acme Corp")
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(manifest.author(), Some("Acme Corp"));
    /// ```
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set the SPDX license identifier (e.g. `"MIT"`, `"Apache-2.0"`).
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Set the homepage URL.
    pub fn homepage(mut self, url: impl Into<String>) -> Self {
        self.homepage = Some(url.into());
        self
    }

    /// Set the source repository URL.
    pub fn repository(mut self, url: impl Into<String>) -> Self {
        self.repository = Some(url.into());
        self
    }

    /// Set the minimum required Nebula engine version (semver string).
    pub fn nebula_version(mut self, version: impl Into<String>) -> Self {
        self.nebula_version = Some(version.into());
        self
    }

    /// Set the declared maturity level.
    pub fn maturity(mut self, maturity: MaturityLevel) -> Self {
        self.maturity = maturity;
        self
    }

    /// Attach a deprecation notice (also implies `maturity = Deprecated`).
    pub fn deprecation(mut self, notice: DeprecationNotice) -> Self {
        self.deprecation = Some(notice);
        self.maturity = MaturityLevel::Deprecated;
        self
    }

    /// Validate and build the manifest.
    ///
    /// The raw key is normalized before validation: spaces become underscores and
    /// ASCII letters are lowercased, so `"HTTP Request"` → `"http_request"`.
    pub fn build(self) -> Result<PluginManifest, ManifestError> {
        let key: PluginKey = normalize_key(&self.key)
            .parse()
            .map_err(ManifestError::InvalidKey)?;

        Ok(PluginManifest {
            key,
            name: self.name,
            version: self.version,
            group: self.group,
            description: self.description,
            icon: self.icon,
            color: self.color,
            tags: self.tags,
            author: self.author,
            license: self.license,
            homepage: self.homepage,
            repository: self.repository,
            nebula_version: self.nebula_version,
            maturity: self.maturity,
            deprecation: self.deprecation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_minimal() {
        let manifest = PluginManifest::builder("slack", "Slack").build().unwrap();
        assert_eq!(manifest.key().as_str(), "slack");
        assert_eq!(manifest.name(), "Slack");
        assert_eq!(manifest.version(), &Version::new(1, 0, 0));
        assert!(manifest.group().is_empty());
        assert!(manifest.description().is_empty());
        assert!(manifest.icon().is_none());
        assert_eq!(manifest.maturity(), MaturityLevel::Stable);
        assert!(manifest.deprecation().is_none());
    }

    #[test]
    fn builder_full_inline_icon() {
        let manifest = PluginManifest::builder("http_request", "HTTP Request")
            .version(Version::new(2, 0, 0))
            .group(vec!["network".into(), "api".into()])
            .description("Make HTTP calls")
            .inline_icon("globe")
            .build()
            .unwrap();

        assert_eq!(manifest.version(), &Version::new(2, 0, 0));
        assert_eq!(manifest.group(), &["network", "api"]);
        assert_eq!(manifest.icon().as_inline(), Some("globe"));
        assert!(manifest.icon().as_url().is_none());
    }

    #[test]
    fn builder_full_url_icon() {
        let manifest = PluginManifest::builder("slack", "Slack")
            .url_icon("https://example.com/icon.svg")
            .build()
            .unwrap();

        assert_eq!(
            manifest.icon().as_url(),
            Some("https://example.com/icon.svg")
        );
        assert!(manifest.icon().as_inline().is_none());
    }

    #[test]
    fn builder_normalizes_key() {
        let manifest = PluginManifest::builder("HTTP Request", "HTTP Request")
            .build()
            .unwrap();
        assert_eq!(manifest.key().as_str(), "http_request");
    }

    #[test]
    fn builder_rejects_invalid_key() {
        let result = PluginManifest::builder("", "Empty").build();
        assert!(result.is_err());
    }

    #[test]
    fn serde_roundtrip_default_fields_omitted() {
        let manifest = PluginManifest::builder("slack", "Slack").build().unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object().unwrap();

        // default version (1.0.0), default maturity, default icon — all omitted.
        assert!(!obj.contains_key("version"));
        assert!(!obj.contains_key("maturity"));
        assert!(!obj.contains_key("icon"));
        assert!(!obj.contains_key("deprecation"));
        assert!(!obj.contains_key("color"));
        assert!(!obj.contains_key("author"));
        assert!(!obj.contains_key("license"));
        assert!(!obj.contains_key("homepage"));
        assert!(!obj.contains_key("repository"));
        assert!(!obj.contains_key("nebula_version"));

        let back: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key().as_str(), "slack");
        assert_eq!(back.version(), &Version::new(1, 0, 0));
    }

    #[test]
    fn serde_roundtrip_full() {
        let manifest = PluginManifest::builder("slack", "Slack")
            .version(Version::new(3, 1, 0))
            .description("Send messages")
            .inline_icon("slack-logo")
            .color("#4A154B")
            .tags(vec!["chat".into(), "messaging".into()])
            .author("Acme Corp")
            .license("Apache-2.0")
            .homepage("https://example.com")
            .repository("https://github.com/acme/slack-plugin")
            .nebula_version("0.5.0")
            .maturity(MaturityLevel::Beta)
            .build()
            .unwrap();

        let json = serde_json::to_string(&manifest).unwrap();
        let back: PluginManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(back.version(), &Version::new(3, 1, 0));
        assert_eq!(back.description(), "Send messages");
        assert_eq!(back.icon().as_inline(), Some("slack-logo"));
        assert_eq!(back.color(), Some("#4A154B"));
        assert_eq!(back.tags(), &["chat", "messaging"]);
        assert_eq!(back.author(), Some("Acme Corp"));
        assert_eq!(back.license(), Some("Apache-2.0"));
        assert_eq!(back.homepage(), Some("https://example.com"));
        assert_eq!(
            back.repository(),
            Some("https://github.com/acme/slack-plugin")
        );
        assert_eq!(back.nebula_version(), Some("0.5.0"));
        assert_eq!(back.maturity(), MaturityLevel::Beta);
    }

    #[test]
    fn deprecation_implies_deprecated_maturity() {
        let manifest = PluginManifest::builder("legacy", "Legacy")
            .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)))
            .build()
            .unwrap();

        assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
        assert_eq!(manifest.deprecation().unwrap().since, Version::new(2, 0, 0));
    }

    #[test]
    fn maturity_default_is_stable() {
        let manifest = PluginManifest::builder("k", "K").build().unwrap();
        assert_eq!(manifest.maturity(), MaturityLevel::Stable);
    }

    #[test]
    fn maturity_override() {
        let manifest = PluginManifest::builder("k", "K")
            .maturity(MaturityLevel::Experimental)
            .build()
            .unwrap();
        assert_eq!(manifest.maturity(), MaturityLevel::Experimental);
    }
}
