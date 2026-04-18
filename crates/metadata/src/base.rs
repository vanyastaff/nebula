//! [`BaseMetadata`] — the shared prefix of every catalog-entity metadata.

use nebula_schema::ValidSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{deprecation::DeprecationNotice, icon::Icon, maturity::MaturityLevel};

fn default_version() -> Version {
    Version::new(1, 0, 0)
}

fn is_default_version(v: &Version) -> bool {
    v == &default_version()
}

/// Shared shape held by every catalog entity's metadata.
///
/// Composed via `#[serde(flatten)] pub base: BaseMetadata<K>` on the
/// concrete metadata struct (for example `ActionMetadata`), so the wire
/// format of the shared prefix is identical across action/credential/
/// resource and any future entity kind.
///
/// Entity-specific extras (ports on an action, auth pattern on a
/// credential, pool settings on a resource) live on the outer struct
/// — `BaseMetadata` stays stable so consumers that only need the common
/// fields (API catalog, search, UI listings) can work generically
/// against any `M: Metadata`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseMetadata<K> {
    /// Unique typed identifier of this entity.
    pub key: K,
    /// Human-readable display name.
    pub name: String,
    /// Short description shown in catalog cards and tooltips.
    pub description: String,
    /// Schema describing the user-configurable inputs of this entity.
    pub schema: ValidSchema,
    /// Interface version — bumped when `schema` or any entity-specific
    /// surface breaks wire-compatibility. Default is `1.0.0`.
    #[serde(
        default = "default_version",
        skip_serializing_if = "is_default_version"
    )]
    pub version: Version,
    /// Catalog icon (inline identifier or URL).
    #[serde(default, skip_serializing_if = "Icon::is_none")]
    pub icon: Icon,
    /// Optional documentation URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    /// Freeform tags used for UI filtering and discovery.
    #[serde(default, skip_serializing_if = "<[String]>::is_empty")]
    pub tags: Box<[String]>,
    /// Declared stability level.
    #[serde(default, skip_serializing_if = "is_default_maturity")]
    pub maturity: MaturityLevel,
    /// Deprecation notice, if this entity is being phased out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<DeprecationNotice>,
}

fn is_default_maturity(m: &MaturityLevel) -> bool {
    *m == MaturityLevel::default()
}

impl<K> BaseMetadata<K> {
    /// Create a new base metadata with the minimum required fields set.
    ///
    /// Version defaults to `1.0.0` — use [`with_version`](Self::with_version)
    /// or the `bump_*` helpers on the outer metadata type to change it.
    #[must_use]
    pub fn new(
        key: K,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: ValidSchema,
    ) -> Self {
        Self {
            key,
            name: name.into(),
            description: description.into(),
            schema,
            version: default_version(),
            icon: Icon::default(),
            documentation_url: None,
            tags: Box::default(),
            maturity: MaturityLevel::default(),
            deprecation: None,
        }
    }

    /// Set the interface version.
    #[must_use]
    pub fn with_version(mut self, version: Version) -> Self {
        self.version = version;
        self
    }

    /// Set the catalog icon.
    #[must_use]
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// Set the documentation URL.
    #[must_use]
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Replace all tags with the given iterator.
    #[must_use]
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Append a single tag, preserving already-declared ones.
    #[must_use]
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        let mut v: Vec<String> = Vec::from(std::mem::take(&mut self.tags));
        v.push(tag.into());
        self.tags = v.into_boxed_slice();
        self
    }

    /// Set an inline-identifier icon (e.g. `"github"`, `"🔑"`).
    #[must_use]
    pub fn with_inline_icon(mut self, name: impl Into<String>) -> Self {
        self.icon = crate::icon::Icon::inline(name);
        self
    }

    /// Set a URL-backed icon.
    #[must_use]
    pub fn with_url_icon(mut self, url: impl Into<String>) -> Self {
        self.icon = crate::icon::Icon::url(url);
        self
    }

    /// Mark this entity as experimental.
    #[must_use]
    pub fn mark_experimental(mut self) -> Self {
        self.maturity = MaturityLevel::Experimental;
        self
    }

    /// Mark this entity as beta.
    #[must_use]
    pub fn mark_beta(mut self) -> Self {
        self.maturity = MaturityLevel::Beta;
        self
    }

    /// Mark this entity as stable (the default — explicit for clarity).
    #[must_use]
    pub fn mark_stable(mut self) -> Self {
        self.maturity = MaturityLevel::Stable;
        self
    }

    /// Shortcut for attaching a minimal [`DeprecationNotice`] that only
    /// records the version the entity was deprecated in. For a richer
    /// notice use [`Self::with_deprecation`].
    #[must_use]
    pub fn deprecate(self, since: semver::Version) -> Self {
        self.with_deprecation(DeprecationNotice::new(since))
    }

    /// Set the declared maturity level.
    #[must_use]
    pub fn with_maturity(mut self, maturity: MaturityLevel) -> Self {
        self.maturity = maturity;
        self
    }

    /// Attach a deprecation notice (also implies `maturity = Deprecated`).
    #[must_use]
    pub fn with_deprecation(mut self, notice: DeprecationNotice) -> Self {
        self.deprecation = Some(notice);
        self.maturity = MaturityLevel::Deprecated;
        self
    }
}

/// Interface every catalog entity's metadata exposes.
///
/// Impls only need to provide [`base`](Metadata::base); the remaining
/// accessors are defaulted to delegate through it. This keeps each
/// per-entity impl to a single line and eliminates the copy-paste bugs
/// that happen when every type has eight getters delegating to
/// different field names.
///
/// Prefer generic bounds `fn f<M: Metadata>(m: &M)` over `dyn Metadata` —
/// the associated [`Key`](Metadata::Key) type makes dyn dispatch awkward
/// and in Nebula's architecture we never actually need a heterogeneous
/// runtime collection of actions + credentials + resources in one vec.
pub trait Metadata {
    /// Typed identifier of the entity (e.g. `ActionKey`, `CredentialKey`).
    type Key;

    /// Borrow the shared metadata block.
    fn base(&self) -> &BaseMetadata<Self::Key>;

    /// Typed entity key.
    fn key(&self) -> &Self::Key {
        &self.base().key
    }

    /// Human-readable display name.
    fn name(&self) -> &str {
        &self.base().name
    }

    /// Short description.
    fn description(&self) -> &str {
        &self.base().description
    }

    /// Canonical input schema.
    fn schema(&self) -> &ValidSchema {
        &self.base().schema
    }

    /// Interface version.
    fn version(&self) -> &Version {
        &self.base().version
    }

    /// Cheap `Arc`-clone of the input schema — useful for consumers
    /// that need to own a `ValidSchema` without borrowing.
    fn schema_arc(&self) -> ValidSchema {
        self.base().schema.clone()
    }

    /// Catalog icon.
    fn icon(&self) -> &Icon {
        &self.base().icon
    }

    /// Documentation URL, if any.
    fn documentation_url(&self) -> Option<&str> {
        self.base().documentation_url.as_deref()
    }

    /// Tags for filtering / discovery.
    fn tags(&self) -> &[String] {
        &self.base().tags
    }

    /// Declared maturity level.
    fn maturity(&self) -> MaturityLevel {
        self.base().maturity
    }

    /// Deprecation notice, if any.
    fn deprecation(&self) -> Option<&DeprecationNotice> {
        self.base().deprecation.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use nebula_schema::Schema;

    use super::*;

    fn empty_schema() -> ValidSchema {
        Schema::builder()
            .build()
            .expect("empty schema always valid")
    }

    struct DummyKey(&'static str);

    struct DummyMetadata {
        base: BaseMetadata<DummyKey>,
    }

    impl Metadata for DummyMetadata {
        type Key = DummyKey;
        fn base(&self) -> &BaseMetadata<Self::Key> {
            &self.base
        }
    }

    #[test]
    fn defaults_delegate_through_base() {
        let md = DummyMetadata {
            base: BaseMetadata::new(DummyKey("k"), "Name", "Desc", empty_schema()),
        };
        assert_eq!(md.key().0, "k");
        assert_eq!(md.name(), "Name");
        assert_eq!(md.description(), "Desc");
        assert!(md.icon().is_none());
        assert!(md.documentation_url().is_none());
        assert_eq!(md.tags().len(), 0);
        assert_eq!(md.maturity(), MaturityLevel::Stable);
        assert!(md.deprecation().is_none());
    }

    #[test]
    fn with_tags_accepts_strings_and_literals() {
        let base =
            BaseMetadata::new(DummyKey("k"), "n", "d", empty_schema()).with_tags(["http", "io"]);
        assert_eq!(&*base.tags, &["http".to_owned(), "io".to_owned()]);
    }

    #[test]
    fn deprecation_forces_maturity() {
        let base = BaseMetadata::new(DummyKey("k"), "n", "d", empty_schema())
            .with_deprecation(DeprecationNotice::new(semver::Version::new(1, 0, 0)));
        assert_eq!(base.maturity, MaturityLevel::Deprecated);
    }
}
