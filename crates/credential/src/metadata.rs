use nebula_core::CredentialKey;
use nebula_metadata::{BaseMetadata, Metadata, MetadataDraft, RecordedBaseMetadata};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::AuthPattern;

/// Leaf authoring state returned by [`crate::Credential::metadata`].
///
/// A draft deliberately carries no schema. Registry admission derives the
/// only canonical schema from `C::Properties` and consumes the draft.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "credential metadata drafts are admitted by a credential registry"]
pub struct CredentialMetadataDraft {
    base: MetadataDraft<CredentialKey>,
}

impl CredentialMetadataDraft {
    /// Construct a credential metadata draft from validated static literals.
    pub fn new(
        key: CredentialKey,
        name: nebula_metadata::MetadataName,
        description: impl Into<String>,
    ) -> Self {
        Self {
            base: MetadataDraft::new(key, name, description),
        }
    }

    /// Construct a draft from a dynamic display name.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_metadata::MetadataError::BlankName`] when `name` is
    /// empty or whitespace-only.
    pub fn try_new(
        key: CredentialKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, nebula_metadata::MetadataError> {
        Ok(Self {
            base: MetadataDraft::try_new(key, name, description)?,
        })
    }

    /// Set the complete interface version.
    #[must_use = "draft methods must be chained"]
    pub fn with_version(mut self, version: Version) -> Self {
        self.base = self.base.with_version(version);
        self
    }

    /// Set the catalog icon.
    #[must_use = "draft methods must be chained"]
    pub fn with_icon(mut self, icon: nebula_metadata::Icon) -> Self {
        self.base = self.base.with_icon(icon);
        self
    }

    /// Set an inline-identifier icon.
    #[must_use = "draft methods must be chained"]
    pub fn with_inline_icon(mut self, name: impl Into<String>) -> Self {
        self.base = self.base.with_inline_icon(name);
        self
    }

    /// Set a URL-backed icon.
    #[must_use = "draft methods must be chained"]
    pub fn with_url_icon(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_url_icon(url);
        self
    }

    /// Attach a documentation URL.
    #[must_use = "draft methods must be chained"]
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_documentation_url(url);
        self
    }

    /// Replace the catalog categories.
    pub fn with_categories(
        mut self,
        categories: impl IntoIterator<Item = nebula_metadata::CatalogCategoryKey>,
    ) -> Self {
        self.base = self.base.with_categories(categories);
        self
    }

    /// Append a typed catalog link.
    pub fn add_link(mut self, link: nebula_metadata::CatalogLink) -> Self {
        self.base = self.base.add_link(link);
        self
    }

    /// Replace all catalog tags.
    #[must_use = "draft methods must be chained"]
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base = self.base.with_tags(tags);
        self
    }

    /// Append one catalog tag.
    #[must_use = "draft methods must be chained"]
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.base = self.base.add_tag(tag);
        self
    }

    /// Mark the credential experimental.
    #[must_use = "draft methods must be chained"]
    pub fn mark_experimental(mut self) -> Self {
        self.base = self.base.mark_experimental();
        self
    }

    /// Mark the credential beta.
    #[must_use = "draft methods must be chained"]
    pub fn mark_beta(mut self) -> Self {
        self.base = self.base.mark_beta();
        self
    }

    /// Mark the credential stable.
    #[must_use = "draft methods must be chained"]
    pub fn mark_stable(mut self) -> Self {
        self.base = self.base.mark_stable();
        self
    }

    /// Attach a deprecation notice and mark the credential deprecated.
    #[must_use = "draft methods must be chained"]
    pub fn with_deprecation(mut self, notice: nebula_metadata::DeprecationNotice) -> Self {
        self.base = self.base.with_deprecation(notice);
        self
    }

    #[tracing::instrument(name = "credential.metadata.admit", skip_all, fields(credential_key = C::KEY), err)]
    pub(crate) fn admit_for<C>(self) -> Result<CredentialMetadata, CredentialMetadataAdmissionError>
    where
        C: crate::Credential,
    {
        let schema = nebula_schema::schema_of::<C::Properties>().map_err(|report| {
            tracing::error!(
                credential.key = C::KEY,
                issue_count = report.errors().count(),
                error_code = "CREDENTIAL:PROPERTIES_SCHEMA_INVALID",
                "credential properties schema admission failed"
            );
            CredentialMetadataAdmissionError::PropertiesSchema
        })?;
        self.admit_with_schema(schema, <C::Scheme as nebula_core::AuthScheme>::pattern())
    }

    fn admit_with_schema(
        self,
        schema: nebula_schema::ValidSchema,
        pattern: AuthPattern,
    ) -> Result<CredentialMetadata, CredentialMetadataAdmissionError> {
        let base = self.base.bind_schema(schema).map_err(|_| {
            tracing::error!(
                error_code = "CREDENTIAL:CATALOG_METADATA_INVALID",
                "credential catalog metadata admission failed"
            );
            CredentialMetadataAdmissionError::CatalogMetadata
        })?;
        let metadata = CredentialMetadata { base, pattern };
        nebula_metadata::check_json_record(&metadata).map_err(|_| {
            tracing::error!(
                error_code = "CREDENTIAL:CATALOG_RECORD_INVALID",
                "credential catalog record admission failed"
            );
            CredentialMetadataAdmissionError::CatalogMetadata
        })?;
        Ok(metadata)
    }
}

/// Payload-free failure to admit a credential definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialMetadataAdmissionError {
    /// Shared catalog metadata failed its construction contract.
    #[error("credential catalog metadata admission failed")]
    CatalogMetadata,
    /// `C::Properties` did not produce a valid canonical schema.
    #[error("credential properties schema admission failed")]
    PropertiesSchema,
}

/// Admitted credential metadata bound to the canonical `C::Properties` schema.
///
/// Fields are immutable and available only through getters. Wire data cannot
/// deserialize into this type; deserialize [`RecordedCredentialMetadata`] and
/// readmit it against a freshly registered definition instead.
///
/// ```compile_fail
/// use nebula_credential::CredentialMetadata;
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<CredentialMetadata>();
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialMetadata {
    base: BaseMetadata<CredentialKey>,
    pattern: AuthPattern,
}

impl Metadata for CredentialMetadata {
    type Key = CredentialKey;

    fn base(&self) -> &BaseMetadata<CredentialKey> {
        &self.base
    }
}

impl CredentialMetadata {
    /// Typed credential key.
    #[must_use]
    pub fn key(&self) -> &CredentialKey {
        self.base.key()
    }

    /// Human-readable credential name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.base.name()
    }

    /// Credential description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.base.description()
    }

    /// Canonical schema derived during registration from `C::Properties`.
    #[must_use]
    pub fn schema(&self) -> &nebula_schema::ValidSchema {
        self.base.schema()
    }

    /// Interface version.
    #[must_use]
    pub fn version(&self) -> &Version {
        self.base.version()
    }

    /// Catalog icon.
    #[must_use]
    pub fn icon(&self) -> &nebula_metadata::Icon {
        self.base.icon()
    }

    /// Documentation URL, when declared.
    #[must_use]
    pub fn documentation_url(&self) -> Option<&str> {
        self.base.documentation_url()
    }

    /// Catalog tags.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        self.base.tags()
    }

    /// Canonical catalog categories.
    #[must_use]
    pub fn categories(&self) -> &[nebula_metadata::CatalogCategoryKey] {
        self.base.categories()
    }

    /// Typed catalog links.
    #[must_use]
    pub fn links(&self) -> &[nebula_metadata::CatalogLink] {
        self.base.links()
    }

    /// Declared maturity level.
    #[must_use]
    pub fn maturity(&self) -> nebula_metadata::MaturityLevel {
        self.base.maturity()
    }

    /// Deprecation notice, if any.
    #[must_use]
    pub fn deprecation(&self) -> Option<&nebula_metadata::DeprecationNotice> {
        self.base.deprecation()
    }

    /// Authentication pattern used by discovery and compatibility checks.
    #[must_use]
    pub const fn pattern(&self) -> AuthPattern {
        self.pattern
    }

    /// Validate this definition against an earlier admitted definition.
    ///
    /// # Errors
    ///
    /// Returns a typed compatibility error when identity, version, schema, or
    /// auth-pattern evolution violates the catalog rules.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;
        if self.pattern != previous.pattern
            && self.base.version().major == previous.base.version().major
        {
            return Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump);
        }
        Ok(())
    }
}

/// Deserialized credential metadata evidence awaiting fresh-definition admission.
///
/// Direct serde decoding checks structure, not raw parser allocation. Byte and
/// reader ingress must use the bounded constructors or an externally bounded
/// transport.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordedCredentialMetadata {
    base: RecordedBaseMetadata<CredentialKey>,
    pattern: AuthPattern,
}

impl<'de> Deserialize<'de> for RecordedCredentialMetadata {
    #[tracing::instrument(name = "credential.metadata.decode_recorded", skip_all)]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            base: RecordedBaseMetadata<CredentialKey>,
            pattern: AuthPattern,
        }

        let fields: Fields = nebula_metadata::deserialize_metadata_object(deserializer)
            .map_err(|_| serde::de::Error::custom("invalid recorded credential metadata"))?;
        let recorded = Self {
            base: fields.base,
            pattern: fields.pattern,
        };
        nebula_metadata::check_json_record(&recorded)
            .map_err(<D::Error as serde::de::Error>::custom)?;
        Ok(recorded)
    }
}

impl RecordedCredentialMetadata {
    /// Decode recorded evidence within the selected whole-envelope byte limit.
    ///
    /// # Errors
    /// Returns a payload-free error for oversized input or invalid wire records.
    pub fn from_slice(
        bytes: &[u8],
        limits: nebula_metadata::MetadataDecodeLimits,
    ) -> Result<Self, nebula_metadata::MetadataDecodeError> {
        nebula_metadata::decode_json_slice(bytes, limits)
    }

    /// Read recorded evidence with bounded buffering before JSON parsing.
    ///
    /// The caller owns the reader's framing and I/O deadline.
    ///
    /// # Errors
    /// Returns a payload-free read, size, or record error.
    pub fn from_reader(
        reader: impl std::io::Read,
        limits: nebula_metadata::MetadataDecodeLimits,
    ) -> Result<Self, nebula_metadata::MetadataDecodeError> {
        nebula_metadata::decode_json_reader(reader, limits)
    }

    /// Readmit matching evidence against a freshly built static definition.
    ///
    /// The returned value is cloned exclusively from `fresh_definition`; no
    /// deserialized field becomes admitted metadata.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialMetadataReadmissionError`] when shared metadata,
    /// schema, or auth pattern differs.
    #[tracing::instrument(name = "credential.metadata.readmit", skip_all, err)]
    pub fn readmit_against(
        &self,
        fresh_definition: &CredentialMetadata,
    ) -> Result<CredentialMetadata, CredentialMetadataReadmissionError> {
        self.base
            .readmit_against(&fresh_definition.base)
            .map_err(|_| CredentialMetadataReadmissionError::DefinitionMismatch)?;
        if self.pattern != fresh_definition.pattern {
            tracing::warn!(
                error_code = "CREDENTIAL:RECORDED_METADATA_MISMATCH",
                "recorded credential metadata rejected"
            );
            return Err(CredentialMetadataReadmissionError::DefinitionMismatch);
        }
        Ok(fresh_definition.clone())
    }
}

/// Payload-free recorded-metadata readmission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialMetadataReadmissionError {
    /// Recorded evidence differs from the fresh static definition.
    #[error("recorded credential metadata does not match the fresh definition")]
    DefinitionMismatch,
}

/// Compatibility validation errors for credential metadata evolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A shared catalog rule fired.
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<CredentialKey>),
    /// Auth pattern changed without a major version bump.
    #[error("credential auth pattern changed without a major version bump")]
    PatternChangeWithoutMajorBump,
}

#[cfg(test)]
mod tests {
    use nebula_core::credential_key;
    use nebula_metadata::{BaseCompatError, DeprecationNotice, Icon, MaturityLevel};
    use semver::Version;

    use super::{CredentialMetadata, CredentialMetadataDraft, MetadataCompatibilityError};
    use crate::AuthPattern;

    fn admitted(pattern: AuthPattern, major: u64, minor: u64) -> CredentialMetadata {
        CredentialMetadataDraft::new(
            credential_key!("cred"),
            crate::metadata_name!("Credential"),
            "description",
        )
        .with_version(Version::new(major, minor, 0))
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            pattern,
        )
        .expect("valid credential metadata")
    }

    #[test]
    fn draft_fields_survive_admission() {
        let version = Version::parse("2.1.3-beta.1").expect("version literal is valid");
        let metadata = CredentialMetadataDraft::new(
            credential_key!("cred"),
            crate::metadata_name!("Credential"),
            "description",
        )
        .with_version(version.clone())
        .with_icon(Icon::inline("key"))
        .with_documentation_url("https://example.test/credentials/cred")
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");

        assert_eq!(metadata.version(), &version);
        assert_eq!(metadata.icon(), &Icon::inline("key"));
        assert_eq!(
            metadata.documentation_url(),
            Some("https://example.test/credentials/cred")
        );
        assert_eq!(metadata.pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn draft_icon_methods_preserve_curated_variants() {
        let inline = CredentialMetadataDraft::new(
            credential_key!("inline"),
            crate::metadata_name!("Inline"),
            "description",
        )
        .with_inline_icon("key")
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        let url = CredentialMetadataDraft::new(
            credential_key!("url"),
            crate::metadata_name!("URL"),
            "description",
        )
        .with_url_icon("https://example.test/icon.svg")
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        let none = CredentialMetadataDraft::new(
            credential_key!("none"),
            crate::metadata_name!("None"),
            "description",
        )
        .with_icon(Icon::None)
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");

        assert_eq!(inline.icon(), &Icon::inline("key"));
        assert_eq!(url.icon(), &Icon::url("https://example.test/icon.svg"));
        assert_eq!(none.icon(), &Icon::None);
    }

    #[test]
    fn draft_lifecycle_and_tags_survive_admission() {
        let experimental = CredentialMetadataDraft::new(
            credential_key!("experimental"),
            crate::metadata_name!("Experimental"),
            "description",
        )
        .mark_experimental()
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        let beta = CredentialMetadataDraft::new(
            credential_key!("beta"),
            crate::metadata_name!("Beta"),
            "description",
        )
        .mark_experimental()
        .mark_beta()
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        let stable = CredentialMetadataDraft::new(
            credential_key!("stable"),
            crate::metadata_name!("Stable"),
            "description",
        )
        .mark_beta()
        .mark_stable()
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        let notice = DeprecationNotice::new(Version::new(2, 0, 0))
            .with_replacement(nebula_metadata::CatalogReference::credential(
                credential_key!("replacement"),
            ))
            .with_reason("superseded");
        let deprecated = CredentialMetadataDraft::new(
            credential_key!("deprecated"),
            crate::metadata_name!("Deprecated"),
            "description",
        )
        .with_tags(["auth"])
        .add_tag("legacy")
        .with_version(Version::new(2, 0, 0))
        .with_deprecation(notice.clone())
        .mark_stable()
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");

        assert_eq!(experimental.maturity(), MaturityLevel::Experimental);
        assert_eq!(beta.maturity(), MaturityLevel::Beta);
        assert_eq!(stable.maturity(), MaturityLevel::Stable);
        assert_eq!(deprecated.maturity(), MaturityLevel::Deprecated);
        assert_eq!(deprecated.deprecation(), Some(&notice));
        assert_eq!(deprecated.tags(), ["auth", "legacy"]);
    }

    #[test]
    fn admitted_wire_nests_versioned_base_and_records_as_evidence() {
        let metadata = admitted(AuthPattern::SecretToken, 2, 1);
        let encoded = serde_json::to_string(&metadata).expect("metadata serializes");
        let value: serde_json::Value =
            serde_json::from_str(&encoded).expect("serialized metadata is valid JSON");
        assert!(value.get("key").is_none());
        assert_eq!(value["base"]["metadata_wire_version"], 2);
        assert_eq!(
            value["base"].get("key").and_then(serde_json::Value::as_str),
            Some("cred")
        );

        let recorded: super::RecordedCredentialMetadata =
            serde_json::from_str(&encoded).expect("metadata records as evidence");
        assert_eq!(recorded.readmit_against(&metadata), Ok(metadata));
    }

    #[test]
    fn pattern_change_requires_major_bump() {
        let previous = admitted(AuthPattern::SecretToken, 1, 0);
        let next = admitted(AuthPattern::OAuth2, 1, 1);
        assert_eq!(
            next.validate_compatibility(&previous),
            Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump)
        );
    }

    #[test]
    fn pattern_change_with_major_is_accepted() {
        let previous = admitted(AuthPattern::SecretToken, 1, 0);
        let next = admitted(AuthPattern::OAuth2, 2, 0);
        assert!(next.validate_compatibility(&previous).is_ok());
    }

    #[test]
    fn key_change_is_rejected() {
        let previous = admitted(AuthPattern::SecretToken, 1, 0);
        let next = CredentialMetadataDraft::new(
            credential_key!("other"),
            crate::metadata_name!("Credential"),
            "description",
        )
        .admit_with_schema(
            nebula_schema::schema_of::<()>().expect("unit schema is valid"),
            AuthPattern::SecretToken,
        )
        .expect("valid credential metadata");
        assert_eq!(
            next.validate_compatibility(&previous),
            Err(MetadataCompatibilityError::Base(
                BaseCompatError::KeyChanged {
                    previous: credential_key!("cred"),
                    current: credential_key!("other"),
                }
            ))
        );
    }
}
