//! Draft, admitted, and recorded forms of shared catalog-leaf metadata.

use nebula_schema::ValidSchema;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use std::str::FromStr;

use crate::bounded::{self, DESCRIPTION_BYTES, PendingCollection, RAW_ENTRIES, SHARED_BYTES};
use crate::defaults::default_version;
use crate::definition::resolve_maturity;
use crate::shared::{Discovery, SharedFields};
use crate::{CatalogCategoryKey, CatalogLink, METADATA_WIRE_VERSION, MetadataBuildError};
use crate::{
    MetadataError, MetadataName, MetadataReadmissionError, deprecation::DeprecationNotice,
    icon::Icon, maturity::MaturityLevel,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveMaturity {
    Experimental,
    Beta,
    Stable,
}

impl From<ActiveMaturity> for MaturityLevel {
    fn from(maturity: ActiveMaturity) -> Self {
        match maturity {
            ActiveMaturity::Experimental => Self::Experimental,
            ActiveMaturity::Beta => Self::Beta,
            ActiveMaturity::Stable => Self::Stable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MetadataLifecycle {
    Active(ActiveMaturity),
    Deprecated(DeprecationNotice),
}

impl MetadataLifecycle {
    fn from_wire(
        maturity: MaturityLevel,
        deprecation: Option<DeprecationNotice>,
    ) -> Result<Self, MetadataError> {
        let maturity = resolve_maturity(maturity, deprecation.as_ref())?;
        if let Some(notice) = deprecation {
            return Ok(Self::Deprecated(notice));
        }

        let active = match maturity {
            MaturityLevel::Experimental => ActiveMaturity::Experimental,
            MaturityLevel::Beta => ActiveMaturity::Beta,
            MaturityLevel::Stable => ActiveMaturity::Stable,
            MaturityLevel::Deprecated => return Err(MetadataError::MissingDeprecationNotice),
        };
        Ok(Self::Active(active))
    }

    fn maturity(&self) -> MaturityLevel {
        match self {
            Self::Active(maturity) => (*maturity).into(),
            Self::Deprecated(_) => MaturityLevel::Deprecated,
        }
    }

    fn deprecation(&self) -> Option<&DeprecationNotice> {
        match self {
            Self::Active(_) => None,
            Self::Deprecated(notice) => Some(notice),
        }
    }

    fn mark_active(self, maturity: ActiveMaturity) -> Self {
        match self {
            Self::Active(_) => Self::Active(maturity),
            deprecated @ Self::Deprecated(_) => deprecated,
        }
    }
}

/// Authored catalog metadata before its canonical input schema is bound.
///
/// A draft owns every shared field except the schema. Its fluent methods are
/// the authoring surface; [`Self::bind_schema`] is the only transition to the
/// getter-only [`BaseMetadata`]. Dynamic names use [`Self::try_new`], while a
/// pre-validated [`MetadataName`] makes [`Self::new`] infallible.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a metadata draft must be bound to a schema"]
pub struct MetadataDraft<K> {
    key: K,
    name: MetadataName,
    description: String,
    version: Version,
    version_error: Option<MetadataError>,
    icon: Icon,
    discovery: Discovery,
    lifecycle: MetadataLifecycle,
}

impl<K> MetadataDraft<K> {
    /// Create a draft from an already validated display name.
    #[tracing::instrument(name = "metadata.construct_draft", skip_all)]
    pub fn new(key: K, name: MetadataName, description: impl Into<String>) -> Self {
        Self {
            key,
            name,
            description: description.into(),
            version: default_version(),
            version_error: None,
            icon: Icon::default(),
            discovery: Discovery::default(),
            lifecycle: MetadataLifecycle::Active(ActiveMaturity::Stable),
        }
    }

    /// Create a draft after validating a dynamic display name.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataError::BlankName`] for empty or whitespace-only text.
    #[tracing::instrument(name = "metadata.try_construct_draft", skip_all, err)]
    pub fn try_new(
        key: K,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, MetadataError> {
        Ok(Self::new(
            key,
            MetadataName::try_from(name.into())?,
            description,
        ))
    }

    /// Set the interface version without checking revision compatibility.
    pub fn with_version(mut self, version: Version) -> Self {
        self.version = version;
        self.version_error = None;
        self
    }

    /// Set a macro-generated literal version, retaining failures until binding.
    ///
    /// This accepts the complete SemVer syntax, including prerelease and build
    /// metadata. A later checked version setter replaces any pending error.
    #[doc(hidden)]
    pub fn with_version_literal(mut self, version: &'static str) -> Self {
        if version.len() > SHARED_BYTES {
            self.version_error = Some(MetadataError::FieldTooLarge(crate::MetadataField::Version));
            return self;
        }
        match version.parse() {
            Ok(version) => self = self.with_version(version),
            Err(_) => {
                self.version_error =
                    Some(MetadataError::InvalidVersion(crate::MetadataField::Version));
            },
        }
        self
    }

    /// Replace the description; its UTF-8 byte budget is checked at binding.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Replace all structured categories, canonicalized at binding.
    ///
    /// Consumes at most 65 raw entries; more than 64 is rejected at binding.
    pub fn with_categories(
        mut self,
        categories: impl IntoIterator<Item = CatalogCategoryKey>,
    ) -> Self {
        self.discovery.categories = PendingCollection::collect(categories);
        self
    }

    /// Replace all documentation links and any pending Overview error.
    pub fn with_links(mut self, links: impl IntoIterator<Item = CatalogLink>) -> Self {
        self.discovery.replace_links(links);
        self
    }

    /// Append a checked link; conflicting Overview targets fail at binding.
    pub fn add_link(mut self, link: CatalogLink) -> Self {
        self.discovery.links.push(link);
        self
    }

    /// Set the catalog icon.
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// Set an inline-identifier icon.
    pub fn with_inline_icon(self, name: impl Into<String>) -> Self {
        self.with_icon(Icon::inline(name))
    }

    /// Set a URL-backed icon.
    pub fn with_url_icon(self, url: impl Into<String>) -> Self {
        self.with_icon(Icon::url(url))
    }

    /// Set or replace the single Overview link, checked at binding.
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.discovery.set_overview(&url.into());
        self
    }

    /// Replace all tags with values from an iterator.
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.discovery.tags = PendingCollection::collect(tags.into_iter().map(Into::into));
        self
    }

    /// Append one tag while preserving existing tags.
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.discovery.tags.push(tag.into());
        self
    }

    /// Mark an active draft as experimental.
    ///
    /// An attached deprecation notice takes precedence.
    pub fn mark_experimental(mut self) -> Self {
        self.lifecycle = self.lifecycle.mark_active(ActiveMaturity::Experimental);
        self
    }

    /// Mark an active draft as beta.
    ///
    /// An attached deprecation notice takes precedence.
    pub fn mark_beta(mut self) -> Self {
        self.lifecycle = self.lifecycle.mark_active(ActiveMaturity::Beta);
        self
    }

    /// Mark an active draft as stable.
    ///
    /// An attached deprecation notice takes precedence.
    pub fn mark_stable(mut self) -> Self {
        self.lifecycle = self.lifecycle.mark_active(ActiveMaturity::Stable);
        self
    }

    /// Attach a deprecation notice and make the draft deprecated.
    pub fn with_deprecation(mut self, notice: DeprecationNotice) -> Self {
        self.lifecycle = MetadataLifecycle::Deprecated(notice);
        self
    }

    /// Bind the canonical input schema and finish technical metadata construction.
    ///
    /// Persisted or wire data deserializes as [`RecordedBaseMetadata`] and must
    /// be re-admitted against a freshly built definition.
    /// Generic keys must serialize deterministically as JSON strings and recover
    /// identically through `FromStr`; trait bounds alone do not prove that law.
    ///
    /// # Errors
    /// Returns a payload-free error for collection, field, chronology, schema
    /// or exact JSON budget violations. `K: Serialize` permits exact accounting.
    #[tracing::instrument(name = "metadata.bind_schema", skip_all, err)]
    pub fn bind_schema(mut self, schema: ValidSchema) -> Result<BaseMetadata<K>, MetadataBuildError>
    where
        K: Serialize,
    {
        self.validate()?;
        bounded::check_serialized(
            &schema,
            bounded::SCHEMA_BYTES,
            MetadataError::SchemaBudgetExceeded,
        )?;
        let metadata = BaseMetadata {
            draft: self,
            schema,
        };
        crate::check_json_record(&metadata)?;
        Ok(metadata)
    }

    fn validate(&mut self) -> Result<(), MetadataError>
    where
        K: Serialize,
    {
        if let Some(error) = self.version_error {
            return Err(error);
        }
        self.discovery.canonicalize()?;
        shared_fields(self, None).validate()
    }

    fn with_wire_lifecycle(
        mut self,
        maturity: MaturityLevel,
        deprecation: Option<DeprecationNotice>,
    ) -> Result<Self, MetadataError> {
        self.lifecycle = MetadataLifecycle::from_wire(maturity, deprecation)?;
        Ok(self)
    }
}

/// Shared shape held by every catalog entity's metadata.
///
/// Leaf crates compose this as a private nested `base` field on their
/// concrete admitted metadata (for example `base: BaseMetadata<ActionKey>`)
/// and expose the shared view through [`Metadata::base`]. This keeps the wire
/// format of the shared prefix identical across action, credential, resource,
/// and any future entity kind without exposing the field for direct access.
///
/// Entity-specific extras (ports on an action, auth pattern on a
/// credential, pool settings on a resource) live on the outer struct
/// — `BaseMetadata` stays stable so consumers that only need the common
/// fields (API catalog, search, UI listings) can work generically
/// against any `M: Metadata`.
///
/// The key type owns its identity rules; use the existing `nebula_core` typed
/// keys for catalog leaves. Names are nonblank, and a deprecation notice always
/// implies Deprecated maturity. Private fields keep callers from bypassing
/// those checks.
///
/// ```compile_fail
/// use nebula_metadata::MetadataDraft;
/// use nebula_schema::ValidSchema;
/// let mut metadata = MetadataDraft::try_new("example", "Example", "").unwrap()
///     .bind_schema(ValidSchema::empty()).expect("valid bounded metadata");
/// metadata.name.clear();
/// ```
///
/// Wire data is recorded evidence, not an admitted static definition:
///
/// ```compile_fail
/// use nebula_metadata::BaseMetadata;
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<BaseMetadata<String>>();
/// ```
///
/// Drafts are authoring state and cannot be reconstructed from wire data either:
///
/// ```compile_fail
/// use nebula_metadata::MetadataDraft;
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<MetadataDraft<String>>();
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseMetadata<K> {
    draft: MetadataDraft<K>,
    schema: ValidSchema,
}

fn shared_fields<'a, K>(
    draft: &'a MetadataDraft<K>,
    schema: Option<&'a ValidSchema>,
) -> SharedFields<'a, K> {
    SharedFields {
        metadata_wire_version: METADATA_WIRE_VERSION,
        key: &draft.key,
        name: draft.name.as_str(),
        description: &draft.description,
        schema,
        version: &draft.version,
        icon: &draft.icon,
        categories: draft.discovery.categories.as_slice(),
        tags: draft.discovery.tags.as_slice(),
        links: draft.discovery.links.as_slice(),
        maturity: draft.lifecycle.maturity(),
        deprecation: draft.lifecycle.deprecation(),
    }
}

impl<K: Serialize> Serialize for BaseMetadata<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        shared_fields(&self.draft, Some(&self.schema)).serialize(serializer)
    }
}

/// Deserialized metadata evidence awaiting comparison with a fresh definition.
///
/// This DTO preserves the serialized [`BaseMetadata`] shape, but it is not an
/// admitted catalog definition. Call [`Self::readmit_against`] with metadata
/// freshly built from a static Rust definition. Recorded fields are evidence
/// only and are never returned as the admitted value.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedBaseMetadata<K> {
    draft: MetadataDraft<K>,
    schema: ValidSchema,
}

impl<K: Serialize> Serialize for RecordedBaseMetadata<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        shared_fields(&self.draft, Some(&self.schema)).serialize(serializer)
    }
}

impl<'de, K: FromStr + Serialize> Deserialize<'de> for RecordedBaseMetadata<K> {
    #[tracing::instrument(name = "metadata.deserialize_recorded", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            metadata_wire_version: u32,
            #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")]
            key: String,
            #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")]
            name: String,
            #[serde(deserialize_with = "bounded::string::<_, DESCRIPTION_BYTES>")]
            description: String,
            schema: ValidSchema,
            #[serde(default = "default_version", deserialize_with = "bounded::version")]
            version: Version,
            #[serde(default)]
            icon: Icon,
            #[serde(default)]
            #[serde(deserialize_with = "bounded::sequence::<_, CatalogCategoryKey, RAW_ENTRIES>")]
            categories: Vec<CatalogCategoryKey>,
            #[serde(default, deserialize_with = "bounded::tags")]
            tags: Vec<String>,
            #[serde(
                default,
                deserialize_with = "bounded::sequence::<_, CatalogLink, RAW_ENTRIES>"
            )]
            links: Vec<CatalogLink>,
            #[serde(default)]
            maturity: MaturityLevel,
            #[serde(default)]
            deprecation: Option<DeprecationNotice>,
        }

        let fields: Fields = crate::deserialize_metadata_object(deserializer)?;
        if fields.metadata_wire_version != METADATA_WIRE_VERSION {
            return Err(D::Error::custom(MetadataError::UnsupportedWireVersion));
        }
        let key = fields
            .key
            .parse()
            .map_err(|_| D::Error::custom(MetadataError::InvalidKey))?;
        let mut draft = MetadataDraft::try_new(key, fields.name, fields.description)
            .map_err(D::Error::custom)?
            .with_version(fields.version)
            .with_icon(fields.icon)
            .with_categories(fields.categories)
            .with_tags(fields.tags)
            .with_links(fields.links)
            .with_wire_lifecycle(fields.maturity, fields.deprecation)
            .map_err(D::Error::custom)?;
        draft.validate().map_err(D::Error::custom)?;
        bounded::check_serialized(
            &fields.schema,
            bounded::SCHEMA_BYTES,
            MetadataError::SchemaBudgetExceeded,
        )
        .map_err(D::Error::custom)?;
        let recorded = Self {
            draft,
            schema: fields.schema,
        };
        crate::check_json_record(&recorded).map_err(D::Error::custom)?;
        Ok(recorded)
    }
}

impl<K: Clone + PartialEq> RecordedBaseMetadata<K> {
    /// Re-admit a fresh static definition when all recorded evidence matches.
    ///
    /// The returned value is always cloned from `fresh_definition`; fields
    /// deserialized into this recorded DTO never become admitted metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataReadmissionError::DefinitionMismatch`] when any shared
    /// field, lifecycle value, or schema differs.
    #[tracing::instrument(name = "metadata.readmit_recorded", skip_all, err)]
    pub fn readmit_against(
        &self,
        fresh_definition: &BaseMetadata<K>,
    ) -> Result<BaseMetadata<K>, MetadataReadmissionError> {
        if self.draft == fresh_definition.draft && self.schema == fresh_definition.schema {
            Ok(fresh_definition.clone())
        } else {
            tracing::warn!(
                error_code = "METADATA:RECORDED_DEFINITION_MISMATCH",
                "recorded metadata rejected"
            );
            Err(MetadataReadmissionError::DefinitionMismatch)
        }
    }
}

impl<K> BaseMetadata<K> {
    /// Typed entity key.
    #[must_use]
    pub fn key(&self) -> &K {
        &self.draft.key
    }

    /// Human-readable, nonblank display name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.draft.name.as_str()
    }

    /// Short description, which may be empty.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.draft.description
    }

    /// Canonical input schema.
    #[must_use]
    pub fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Interface version.
    #[must_use]
    pub fn version(&self) -> &Version {
        &self.draft.version
    }

    /// Catalog icon.
    #[must_use]
    pub fn icon(&self) -> &Icon {
        &self.draft.icon
    }

    /// Documentation URL, if any.
    #[must_use]
    pub fn documentation_url(&self) -> Option<&str> {
        self.draft.discovery.documentation_url()
    }

    /// Canonical, sorted structured category keys.
    #[must_use]
    pub fn categories(&self) -> &[CatalogCategoryKey] {
        self.draft.discovery.categories.as_slice()
    }

    /// Canonical documentation links sorted by relation and target.
    #[must_use]
    pub fn links(&self) -> &[CatalogLink] {
        self.draft.discovery.links.as_slice()
    }

    /// Tags for filtering and discovery.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        self.draft.discovery.tags.as_slice()
    }

    /// Maturity, always Deprecated when a notice is present.
    #[must_use]
    pub fn maturity(&self) -> MaturityLevel {
        self.draft.lifecycle.maturity()
    }

    /// Deprecation notice, if any.
    #[must_use]
    pub fn deprecation(&self) -> Option<&DeprecationNotice> {
        self.draft.lifecycle.deprecation()
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
        self.base().key()
    }

    /// Human-readable display name.
    fn name(&self) -> &str {
        self.base().name()
    }

    /// Short description.
    fn description(&self) -> &str {
        self.base().description()
    }

    /// Canonical input schema.
    fn schema(&self) -> &ValidSchema {
        self.base().schema()
    }

    /// Interface version.
    fn version(&self) -> &Version {
        self.base().version()
    }

    /// Catalog icon.
    fn icon(&self) -> &Icon {
        self.base().icon()
    }

    /// Documentation URL, if any.
    fn documentation_url(&self) -> Option<&str> {
        self.base().documentation_url()
    }

    /// Canonical structured categories.
    fn categories(&self) -> &[CatalogCategoryKey] {
        self.base().categories()
    }

    /// Canonical documentation links.
    fn links(&self) -> &[CatalogLink] {
        self.base().links()
    }

    /// Tags for filtering / discovery.
    fn tags(&self) -> &[String] {
        self.base().tags()
    }

    /// Declared maturity level.
    fn maturity(&self) -> MaturityLevel {
        self.base().maturity()
    }

    /// Deprecation notice, if any.
    fn deprecation(&self) -> Option<&DeprecationNotice> {
        self.base().deprecation()
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

    #[derive(Serialize)]
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
            base: MetadataDraft::try_new(DummyKey("k"), "Name", "Desc")
                .expect("nonblank name")
                .bind_schema(empty_schema())
                .expect("valid bounded metadata"),
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
        let base = MetadataDraft::try_new(DummyKey("k"), "n", "d")
            .expect("nonblank name")
            .with_tags(["http", "io"])
            .bind_schema(empty_schema())
            .expect("valid bounded metadata");
        assert_eq!(base.tags(), &["http".to_owned(), "io".to_owned()]);
    }

    #[test]
    fn deprecation_forces_maturity() {
        let base = MetadataDraft::try_new(DummyKey("k"), "n", "d")
            .expect("nonblank name")
            .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
            .bind_schema(empty_schema())
            .expect("valid bounded metadata");
        assert_eq!(base.maturity(), MaturityLevel::Deprecated);
    }
}
