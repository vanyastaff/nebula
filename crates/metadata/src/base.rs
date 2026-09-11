//! Draft, admitted, and recorded forms of shared catalog-leaf metadata.

use nebula_schema::ValidSchema;
use semver::Version;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeStruct,
};
use std::str::FromStr;

use crate::defaults::{default_version, is_default_maturity, is_default_version};
use crate::definition::resolve_maturity;
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
    icon: Icon,
    documentation_url: Option<String>,
    tags: Box<[String]>,
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
            icon: Icon::default(),
            documentation_url: None,
            tags: Box::default(),
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

    /// Set the documentation URL.
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Replace all tags with values from an iterator.
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Append one tag while preserving existing tags.
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        let mut tags = Vec::from(std::mem::take(&mut self.tags));
        tags.push(tag.into());
        self.tags = tags.into_boxed_slice();
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
    #[must_use]
    #[tracing::instrument(name = "metadata.bind_schema", skip_all)]
    pub fn bind_schema(self, schema: ValidSchema) -> BaseMetadata<K> {
        BaseMetadata {
            draft: self,
            schema,
        }
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
/// Leaf crates compose this as a private `#[serde(flatten)]` field on their
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
///     .bind_schema(ValidSchema::empty());
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

fn serialize_metadata<S, K>(
    draft: &MetadataDraft<K>,
    schema: &ValidSchema,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    K: Serialize,
{
    let maturity = draft.lifecycle.maturity();
    let deprecation = draft.lifecycle.deprecation();
    let mut field_count = 4;
    field_count += usize::from(!is_default_version(&draft.version));
    field_count += usize::from(!draft.icon.is_none());
    field_count += usize::from(draft.documentation_url.is_some());
    field_count += usize::from(!draft.tags.is_empty());
    field_count += usize::from(!is_default_maturity(&maturity));
    field_count += usize::from(deprecation.is_some());

    let mut fields = serializer.serialize_struct("BaseMetadata", field_count)?;
    fields.serialize_field("key", &draft.key)?;
    fields.serialize_field("name", draft.name.as_str())?;
    fields.serialize_field("description", &draft.description)?;
    fields.serialize_field("schema", schema)?;
    if !is_default_version(&draft.version) {
        fields.serialize_field("version", &draft.version)?;
    }
    if !draft.icon.is_none() {
        fields.serialize_field("icon", &draft.icon)?;
    }
    if let Some(documentation_url) = draft.documentation_url.as_deref() {
        fields.serialize_field("documentation_url", documentation_url)?;
    }
    if !draft.tags.is_empty() {
        fields.serialize_field("tags", &draft.tags)?;
    }
    if !is_default_maturity(&maturity) {
        fields.serialize_field("maturity", &maturity)?;
    }
    if let Some(notice) = deprecation {
        fields.serialize_field("deprecation", notice)?;
    }
    fields.end()
}

impl<K: Serialize> Serialize for BaseMetadata<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_metadata(&self.draft, &self.schema, serializer)
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
        serialize_metadata(&self.draft, &self.schema, serializer)
    }
}

impl<'de, K: FromStr> Deserialize<'de> for RecordedBaseMetadata<K> {
    #[tracing::instrument(name = "metadata.deserialize_recorded", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            key: String,
            name: String,
            description: String,
            schema: ValidSchema,
            #[serde(default = "default_version")]
            version: Version,
            #[serde(default)]
            icon: Icon,
            #[serde(default)]
            documentation_url: Option<String>,
            #[serde(default)]
            tags: Box<[String]>,
            #[serde(default)]
            maturity: MaturityLevel,
            #[serde(default)]
            deprecation: Option<DeprecationNotice>,
        }

        let fields = Fields::deserialize(deserializer)?;
        let key = fields
            .key
            .parse()
            .map_err(|_| D::Error::custom(MetadataError::InvalidKey))?;
        let mut draft = MetadataDraft::try_new(key, fields.name, fields.description)
            .map_err(D::Error::custom)?
            .with_version(fields.version)
            .with_icon(fields.icon)
            .with_tags(fields.tags)
            .with_wire_lifecycle(fields.maturity, fields.deprecation)
            .map_err(D::Error::custom)?;
        if let Some(url) = fields.documentation_url {
            draft = draft.with_documentation_url(url);
        }
        Ok(Self {
            draft,
            schema: fields.schema,
        })
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
        self.draft.documentation_url.as_deref()
    }

    /// Tags for filtering and discovery.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.draft.tags
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
                .bind_schema(empty_schema()),
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
            .bind_schema(empty_schema());
        assert_eq!(base.tags(), &["http".to_owned(), "io".to_owned()]);
    }

    #[test]
    fn deprecation_forces_maturity() {
        let base = MetadataDraft::try_new(DummyKey("k"), "n", "d")
            .expect("nonblank name")
            .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
            .bind_schema(empty_schema());
        assert_eq!(base.maturity(), MaturityLevel::Deprecated);
    }
}
