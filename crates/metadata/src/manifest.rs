//! Plugin manifest — bundle descriptor for a plugin.
//!
//! A [`PluginManifest`] describes the *container* that bundles actions,
//! credentials, and resources under a versioned identity. It reuses the
//! shared small types from this crate ([`Icon`], [`MaturityLevel`],
//! [`DeprecationNotice`]) but deliberately does **not** compose
//! `BaseMetadata<K>`: a plugin is a container, not a schematized leaf.
//! See ADR-0018 (historical — the maintainers' private design vault).
//!
//! This module lives in `nebula-metadata` (moved here from `nebula-plugin`
//! in slice B of the plugin load-path stabilization): ADR-0018 draws the
//! container-descriptor line at the Core layer, so the bundle descriptor
//! belongs beside the other shared catalog types it reuses rather than in
//! the higher-layer `nebula-plugin`. The original rationale for the move
//! named `nebula-plugin-sdk`, an out-of-process plugin-authoring crate that
//! ADR-0091's in-process registry pivot retired; that crate no longer
//! exists, and the placement now rests on the ADR-0018 rationale above.

use nebula_core::PluginKey;
use semver::{Version, VersionReq};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::bounded::{self, PendingCollection, RAW_ENTRIES, SHARED_BYTES};
use crate::defaults::{default_version, is_default_maturity, is_default_version};
use crate::definition::{resolve_maturity, validate_name};
use crate::shared::{Discovery, SharedFields};
use crate::{CatalogCategoryKey, CatalogLink, METADATA_WIRE_VERSION};
use crate::{DeprecationNotice, Icon, MaturityLevel, MetadataError};

/// A declared dependency of one plugin on another.
///
/// A plugin may require that another plugin is loaded before it: `key` names the
/// dependency and `req` constrains which versions satisfy the requirement.
///
/// When the registry resolves load order via `PluginRegistry::resolve_load_order`,
/// it validates that every declared dependency is registered and that its version
/// matches `req`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDependency {
    /// The key of the required plugin.
    key: PluginKey,
    /// Semver version requirement that the registered plugin must satisfy.
    req: VersionReq,
}

impl<'de> Deserialize<'de> for PluginDependency {
    #[tracing::instrument(name = "metadata.deserialize_plugin_dependency", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")]
            key: String,
            #[serde(deserialize_with = "bounded::version_requirement")]
            req: VersionReq,
        }

        let fields: Fields = crate::deserialize_metadata_object(deserializer)?;
        let key = fields
            .key
            .parse()
            .map_err(|_| D::Error::custom(MetadataError::InvalidKey))?;
        Ok(Self::new(key, fields.req))
    }
}

impl PluginDependency {
    /// Construct a new dependency declaration.
    #[must_use]
    pub fn new(key: PluginKey, req: VersionReq) -> Self {
        Self { key, req }
    }

    /// The key of the required plugin.
    #[must_use]
    #[inline]
    pub fn key(&self) -> &PluginKey {
        &self.key
    }

    /// The semver requirement the registered plugin must satisfy.
    #[must_use]
    #[inline]
    pub fn req(&self) -> &VersionReq {
        &self.req
    }
}

/// Errors from [`PluginManifest::builder().build()`](PluginManifestBuilder::build).
#[derive(Debug, thiserror::Error, nebula_error::Classify, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestError {
    /// A shared catalog-definition invariant failed.
    #[classify(category = "validation", code = "MANIFEST:INVALID_METADATA")]
    #[error("invalid plugin metadata: {0}")]
    Metadata(#[from] MetadataError),

    /// Plugin key validation failed.
    #[classify(category = "validation", code = "MANIFEST:INVALID_KEY")]
    #[error("invalid plugin key")]
    InvalidKey,
}

/// Normalize a raw plugin key string: ASCII uppercase → lowercase, spaces → underscores.
///
/// Used internally by [`PluginManifestBuilder::build`] to normalize the raw key before
/// validation. Not part of the public API.
pub(crate) fn normalize_key(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "_")
}

/// Static manifest describing a plugin bundle.
///
/// Built via the builder API:
///
/// ```
/// use nebula_metadata::PluginManifest;
/// use semver::Version;
///
/// let manifest = PluginManifest::builder("http_request", "HTTP Request")
///.description("Make HTTP calls to external APIs")
///.group(vec!["network".into()])
///.version(Version::new(2, 0, 0))
///.build()
///.unwrap();
///
/// assert_eq!(manifest.key().as_str(), "http_request");
/// assert_eq!(manifest.version(), &Version::new(2, 0, 0));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginManifest {
    metadata_wire_version: u32,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    categories: Vec<CatalogCategoryKey>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    links: Vec<CatalogLink>,
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
    /// Minimum Nebula engine version required by this plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nebula_version: Option<Version>,
    #[serde(default, skip_serializing_if = "is_default_maturity")]
    maturity: MaturityLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deprecation: Option<DeprecationNotice>,
    /// Other plugins this plugin depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<PluginDependency>,
}

impl<'de> Deserialize<'de> for PluginManifest {
    #[tracing::instrument(name = "metadata.deserialize_plugin_manifest", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields: ManifestFields = crate::deserialize_metadata_object(deserializer)?;
        if fields.metadata_wire_version != METADATA_WIRE_VERSION {
            return Err(D::Error::custom(MetadataError::UnsupportedWireVersion));
        }
        fields.into_builder().build().map_err(D::Error::custom)
    }
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
            discovery: Discovery::default(),
            author: None,
            license: None,
            homepage: None,
            repository: None,
            nebula_version: None,
            maturity: MaturityLevel::default(),
            deprecation: None,
            dependencies: Vec::new(),
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

    /// Canonical structured discovery categories.
    #[must_use]
    pub fn categories(&self) -> &[CatalogCategoryKey] {
        &self.categories
    }

    /// Canonical documentation links.
    #[must_use]
    pub fn links(&self) -> &[CatalogLink] {
        &self.links
    }

    /// The single Overview link target, when present.
    #[must_use]
    pub fn documentation_url(&self) -> Option<&str> {
        self.links
            .iter()
            .find(|link| link.relation() == crate::CatalogLinkRelation::Overview)
            .map(|link| link.target().as_str())
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

    /// Minimum Nebula engine version required by this plugin.
    #[inline]
    pub fn nebula_version(&self) -> Option<&Version> {
        self.nebula_version.as_ref()
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

    /// Declared plugin dependencies.
    ///
    /// Returns a slice of [`PluginDependency`] entries, each naming another
    /// plugin key and the semver requirement it must satisfy. Empty for
    /// plugins with no declared dependencies.
    #[inline]
    pub fn dependencies(&self) -> &[PluginDependency] {
        &self.dependencies
    }
}

/// Unchecked draft of a [`PluginManifest`]; [`Self::build`] checks its invariants.
///
/// Wire data decodes through a private DTO and the same final build gate.
#[derive(Debug)]
#[must_use = "a manifest draft must be built to validate its definition"]
pub struct PluginManifestBuilder {
    key: String,
    name: String,
    version: Version,
    group: Vec<String>,
    description: String,
    icon: Icon,
    color: Option<String>,
    discovery: Discovery,
    author: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    nebula_version: Option<Version>,
    maturity: MaturityLevel,
    deprecation: Option<DeprecationNotice>,
    dependencies: Vec<PluginDependency>,
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
        self.discovery.tags = PendingCollection::collect(tags);
        self
    }

    /// Replace structured categories, bounded and canonicalized by build.
    pub fn with_categories(
        mut self,
        categories: impl IntoIterator<Item = CatalogCategoryKey>,
    ) -> Self {
        self.discovery.categories = PendingCollection::collect(categories);
        self
    }

    /// Replace documentation links and clear pending errors for this field.
    pub fn with_links(mut self, links: impl IntoIterator<Item = CatalogLink>) -> Self {
        self.discovery.replace_links(links);
        self
    }

    /// Append a link; conflicting Overview targets fail at build.
    pub fn add_link(mut self, link: CatalogLink) -> Self {
        self.discovery.links.push(link);
        self
    }

    /// Replace the single Overview link, checked by the build gate.
    pub fn with_documentation_url(mut self, target: impl AsRef<str>) -> Self {
        self.discovery.set_overview(target.as_ref());
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
    ///.author("Acme Corp")
    ///.build()
    ///.unwrap();
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

    /// Set the minimum required Nebula engine version.
    pub fn nebula_version(mut self, version: Version) -> Self {
        self.nebula_version = Some(version);
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

    /// Declare that this plugin depends on another plugin.
    ///
    /// May be called multiple times to add multiple dependencies.
    pub fn dependency(mut self, dep: PluginDependency) -> Self {
        if self.dependencies.len() <= RAW_ENTRIES {
            self.dependencies.push(dep);
        }
        self
    }

    /// Set all dependency declarations at once, replacing any previously added.
    pub fn dependencies(mut self, deps: Vec<PluginDependency>) -> Self {
        self.dependencies = deps;
        self
    }

    /// Validate and build the manifest.
    ///
    /// The raw key is normalized before validation: spaces become underscores and
    /// ASCII letters are lowercased, so `"HTTP Request"` → `"http_request"`.
    ///
    /// `deprecation` always wins over `maturity`: if a deprecation notice is
    /// present the built manifest's maturity is forced to
    /// [`MaturityLevel::Deprecated`] regardless of the order in which
    /// `.deprecation()` and `.maturity()` were called on the builder.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidKey`] if the normalized key fails
    /// [`PluginKey`] validation, or
    /// [`ManifestError::Metadata`] if the name is blank or Deprecated maturity
    /// was requested without a deprecation notice.
    #[tracing::instrument(name = "metadata.build_plugin_manifest", skip_all)]
    pub fn build(mut self) -> Result<PluginManifest, ManifestError> {
        bounded::check_bytes(&self.key, SHARED_BYTES, crate::MetadataField::Key)?;
        let key: PluginKey = normalize_key(&self.key)
            .parse()
            .map_err(|_| ManifestError::InvalidKey)
            .inspect_err(|_| {
                tracing::debug!(
                    error_code = "MANIFEST:INVALID_KEY",
                    "catalog construction rejected"
                );
            })?;

        validate_name(&self.name)?;
        let name = self.name.trim().to_owned();
        let maturity = resolve_maturity(self.maturity, self.deprecation.as_ref())?;
        self.discovery.canonicalize()?;
        SharedFields {
            metadata_wire_version: METADATA_WIRE_VERSION,
            key: &key,
            name: &name,
            description: &self.description,
            schema: None,
            version: &self.version,
            icon: &self.icon,
            categories: self.discovery.categories.as_slice(),
            tags: self.discovery.tags.as_slice(),
            links: self.discovery.links.as_slice(),
            maturity,
            deprecation: self.deprecation.as_ref(),
        }
        .validate()?;
        if self.group.len() > RAW_ENTRIES {
            return Err(
                MetadataError::TooManyRawEntries(crate::MetadataField::ManifestGroup).into(),
            );
        }
        if self.dependencies.len() > RAW_ENTRIES {
            return Err(MetadataError::TooManyRawEntries(
                crate::MetadataField::ManifestDependencies,
            )
            .into());
        }
        for dependency in &self.dependencies {
            bounded::check_serialized(
                dependency.req(),
                SHARED_BYTES,
                MetadataError::FieldTooLarge(crate::MetadataField::ManifestDependencies),
            )?;
            crate::reference::checked_requirement_text(dependency.req()).map_err(|_| {
                MetadataError::InvalidVersion(crate::MetadataField::ManifestDependencies)
            })?;
        }
        if let Some(version) = &self.nebula_version {
            bounded::check_serialized(
                version,
                SHARED_BYTES,
                MetadataError::FieldTooLarge(crate::MetadataField::ManifestNebulaVersion),
            )?;
        }
        for value in &self.group {
            bounded::check_bytes(value, SHARED_BYTES, crate::MetadataField::ManifestGroup)?;
        }
        for (field, value) in [
            (crate::MetadataField::ManifestColor, self.color.as_deref()),
            (crate::MetadataField::ManifestAuthor, self.author.as_deref()),
            (
                crate::MetadataField::ManifestLicense,
                self.license.as_deref(),
            ),
            (
                crate::MetadataField::ManifestHomepage,
                self.homepage.as_deref(),
            ),
            (
                crate::MetadataField::ManifestRepository,
                self.repository.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                bounded::check_bytes(value, SHARED_BYTES, field)?;
            }
        }

        let manifest = PluginManifest {
            metadata_wire_version: METADATA_WIRE_VERSION,
            key,
            name,
            version: self.version,
            group: self.group,
            description: self.description,
            icon: self.icon,
            color: self.color,
            tags: self
                .discovery
                .tags
                .take_checked(crate::MetadataField::Tags)?,
            categories: self
                .discovery
                .categories
                .take_checked(crate::MetadataField::Categories)?,
            links: self
                .discovery
                .links
                .take_checked(crate::MetadataField::Links)?,
            author: self.author,
            license: self.license,
            homepage: self.homepage,
            repository: self.repository,
            nebula_version: self.nebula_version,
            maturity,
            deprecation: self.deprecation,
            dependencies: self.dependencies,
        };
        // Packaging is a separate container concern with its own bounded record.
        bounded::check_serialized(
            &manifest,
            bounded::MANIFEST_BYTES,
            MetadataError::ManifestBudgetExceeded,
        )?;
        crate::check_json_record(&manifest)?;
        Ok(manifest)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFields {
    metadata_wire_version: u32,
    #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")]
    key: String,
    #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")]
    name: String,
    #[serde(default = "default_version", deserialize_with = "bounded::version")]
    version: Version,
    #[serde(default, deserialize_with = "bounded::tags")]
    group: Vec<String>,
    #[serde(
        default,
        deserialize_with = "bounded::string::<_, {bounded::DESCRIPTION_BYTES}>"
    )]
    description: String,
    #[serde(default)]
    icon: Icon,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    color: Option<String>,
    #[serde(default, deserialize_with = "bounded::tags")]
    tags: Vec<String>,
    #[serde(
        default,
        deserialize_with = "bounded::sequence::<_, CatalogCategoryKey, RAW_ENTRIES>"
    )]
    categories: Vec<CatalogCategoryKey>,
    #[serde(
        default,
        deserialize_with = "bounded::sequence::<_, CatalogLink, RAW_ENTRIES>"
    )]
    links: Vec<CatalogLink>,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    author: Option<String>,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    license: Option<String>,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    homepage: Option<String>,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    repository: Option<String>,
    #[serde(default, deserialize_with = "bounded::optional_version")]
    nebula_version: Option<Version>,
    #[serde(default)]
    maturity: MaturityLevel,
    #[serde(default)]
    deprecation: Option<DeprecationNotice>,
    #[serde(
        default,
        deserialize_with = "bounded::sequence::<_, PluginDependency, RAW_ENTRIES>"
    )]
    dependencies: Vec<PluginDependency>,
}

impl ManifestFields {
    fn into_builder(self) -> PluginManifestBuilder {
        let mut discovery = Discovery::default();
        discovery.categories = PendingCollection::collect(self.categories);
        discovery.tags = PendingCollection::collect(self.tags);
        discovery.replace_links(self.links);
        PluginManifestBuilder {
            key: self.key,
            name: self.name,
            version: self.version,
            group: self.group,
            description: self.description,
            icon: self.icon,
            color: self.color,
            discovery,
            author: self.author,
            license: self.license,
            homepage: self.homepage,
            repository: self.repository,
            nebula_version: self.nebula_version,
            maturity: self.maturity,
            deprecation: self.deprecation,
            dependencies: self.dependencies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(key: &str, req: &str) -> PluginDependency {
        PluginDependency::new(key.parse().unwrap(), req.parse().unwrap())
    }

    #[test]
    fn dependency_accessors() {
        let d = dep("auth", "^1.0.0");
        assert_eq!(d.key().as_str(), "auth");
        // semver normalizes "^1.0.0" → "^1" in Display; compare as parsed VersionReq
        assert_eq!(d.req(), &"^1.0.0".parse::<VersionReq>().unwrap());
    }

    #[test]
    fn dependencies_omitted_when_empty() {
        let manifest = PluginManifest::builder("slack", "Slack").build().unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("dependencies"),
            "\"dependencies\" key must be absent for a manifest with no deps"
        );
    }

    #[test]
    fn dependencies_round_trip() {
        let manifest = PluginManifest::builder("slack", "Slack")
            .dependency(dep("auth", "^1.0.0"))
            .dependency(dep("http_client", ">=2.0.0, <3.0.0"))
            .build()
            .unwrap();

        assert_eq!(manifest.dependencies().len(), 2);
        let json = serde_json::to_string(&manifest).unwrap();
        let back: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dependencies().len(), 2);
        assert_eq!(back.dependencies()[0].key().as_str(), "auth");
        assert_eq!(back.dependencies()[1].key().as_str(), "http_client");
    }

    #[test]
    fn builder_dependencies_setter_replaces() {
        let manifest = PluginManifest::builder("x", "X")
            .dependency(dep("old", "^1"))
            .dependencies(vec![dep("new", "^2")])
            .build()
            .unwrap();
        assert_eq!(manifest.dependencies().len(), 1);
        assert_eq!(manifest.dependencies()[0].key().as_str(), "new");
    }

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
    fn builder_rejects_empty_name() {
        let result = PluginManifest::builder("slack", "").build();
        assert_eq!(
            result,
            Err(ManifestError::Metadata(MetadataError::BlankName))
        );
    }

    #[test]
    fn builder_rejects_whitespace_only_name() {
        let result = PluginManifest::builder("slack", "   ").build();
        assert_eq!(
            result,
            Err(ManifestError::Metadata(MetadataError::BlankName))
        );
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
            .nebula_version(Version::new(0, 5, 0))
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
        assert_eq!(back.nebula_version(), Some(&Version::new(0, 5, 0)));
        assert_eq!(back.maturity(), MaturityLevel::Beta);
    }

    #[test]
    fn deprecation_implies_deprecated_maturity() {
        let manifest = PluginManifest::builder("legacy", "Legacy")
            .version(Version::new(2, 0, 0))
            .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)))
            .build()
            .unwrap();

        assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
        assert_eq!(
            *manifest.deprecation().unwrap().since(),
            Version::new(2, 0, 0)
        );
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

    /// `deprecation` beats `maturity` regardless of call order.
    ///
    /// Case A: `.maturity(Stable).deprecation(notice)` — deprecation last.
    /// Case B: `.deprecation(notice).maturity(Stable)` — maturity last.
    ///
    /// Both must produce `MaturityLevel::Deprecated`.
    #[test]
    fn deprecation_forces_deprecated_maturity_regardless_of_order() {
        let notice = DeprecationNotice::new(Version::new(3, 0, 0));

        // Case A: maturity set first, deprecation second.
        let a = PluginManifest::builder("legacy_a", "Legacy A")
            .version(Version::new(3, 0, 0))
            .maturity(MaturityLevel::Stable)
            .deprecation(notice.clone())
            .build()
            .unwrap();
        assert_eq!(
            a.maturity(),
            MaturityLevel::Deprecated,
            "Case A: .maturity(Stable).deprecation(notice) must produce Deprecated"
        );

        // Case B: deprecation set first, maturity second — the tricky order
        // that the builder's `.deprecation()` setter alone cannot protect against.
        let b = PluginManifest::builder("legacy_b", "Legacy B")
            .version(Version::new(3, 0, 0))
            .deprecation(notice)
            .maturity(MaturityLevel::Stable)
            .build()
            .unwrap();
        assert_eq!(
            b.maturity(),
            MaturityLevel::Deprecated,
            "Case B: .deprecation(notice).maturity(Stable) must produce Deprecated"
        );
    }
}
