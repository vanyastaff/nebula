use nebula_core::CredentialKey;
use nebula_metadata::{BaseDefinition, Definition, RecordedBaseDefinition};
use serde::{Deserialize, Serialize};

use crate::AuthPattern;

/// What a credential's author says about it for the catalog: name,
/// description, version, icon, links, tags, maturity.
///
/// This is the shared [`nebula_metadata::Metadata`] under the name credential
/// authors use. It holds neither a key nor a schema nor an auth pattern: the
/// registry stamps [`Credential::KEY`](crate::Credential::KEY), derives the
/// schema from `Properties` and the pattern from `Scheme` when it admits the
/// credential as a [`CredentialDefinition`].
pub type CredentialMetadata = nebula_metadata::Metadata;

/// Payload-free failure to admit a credential definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialAdmissionError {
    /// Shared catalog metadata failed its construction contract.
    #[error("credential catalog metadata admission failed")]
    CatalogMetadata,
    /// `C::Properties` did not produce a valid canonical schema.
    #[error("credential properties schema admission failed")]
    PropertiesSchema,
}

/// Admitted credential definition bound to the canonical `C::Properties` schema.
///
/// Fields are immutable. The shared catalog fields are read through the
/// [`Definition`] trait, as on every other component definition. Wire data cannot
/// deserialize into this type; deserialize [`RecordedCredentialDefinition`] and
/// readmit it against a freshly registered definition instead.
///
/// ```compile_fail
/// use nebula_credential::CredentialDefinition;
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<CredentialDefinition>();
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialDefinition {
    base: BaseDefinition<CredentialKey>,
    pattern: AuthPattern,
}

impl Definition for CredentialDefinition {
    type Key = CredentialKey;

    fn base(&self) -> &BaseDefinition<CredentialKey> {
        &self.base
    }
}

impl CredentialDefinition {
    /// Admit `C`'s catalog definition: its author's metadata, under
    /// [`Credential::KEY`](crate::Credential::KEY) and the canonical
    /// `C::Properties` schema.
    #[tracing::instrument(name = "credential.metadata.admit", skip_all, fields(credential_key = C::KEY), err)]
    pub(crate) fn admit_for<C>(
        metadata: CredentialMetadata,
    ) -> Result<Self, CredentialAdmissionError>
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
            CredentialAdmissionError::PropertiesSchema
        })?;
        Self::admit_with_schema(
            crate::key_of::<C>(),
            metadata,
            schema,
            <C::Scheme as nebula_core::AuthScheme>::pattern(),
        )
    }

    fn admit_with_schema(
        key: CredentialKey,
        metadata: CredentialMetadata,
        schema: nebula_schema::ValidSchema,
        pattern: AuthPattern,
    ) -> Result<Self, CredentialAdmissionError> {
        let base = metadata.bind(key, schema).map_err(|_| {
            tracing::error!(
                error_code = "CREDENTIAL:CATALOG_METADATA_INVALID",
                "credential catalog metadata admission failed"
            );
            CredentialAdmissionError::CatalogMetadata
        })?;
        let definition = Self { base, pattern };
        nebula_metadata::check_json_record(&definition).map_err(|_| {
            tracing::error!(
                error_code = "CREDENTIAL:CATALOG_RECORD_INVALID",
                "credential catalog record admission failed"
            );
            CredentialAdmissionError::CatalogMetadata
        })?;
        Ok(definition)
    }

    /// The shared catalog definition: key, name, description, schema,
    /// version, icon, links, tags, maturity.
    #[must_use]
    pub const fn base(&self) -> &BaseDefinition<CredentialKey> {
        &self.base
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
    ) -> Result<(), CredentialCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;
        if self.pattern != previous.pattern
            && self.base.version().major == previous.base.version().major
        {
            return Err(CredentialCompatibilityError::PatternChangeWithoutMajorBump);
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
pub struct RecordedCredentialDefinition {
    base: RecordedBaseDefinition<CredentialKey>,
    pattern: AuthPattern,
}

impl<'de> Deserialize<'de> for RecordedCredentialDefinition {
    #[tracing::instrument(name = "credential.metadata.decode_recorded", skip_all)]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            base: RecordedBaseDefinition<CredentialKey>,
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

impl RecordedCredentialDefinition {
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
    /// Returns [`CredentialReadmissionError`] when shared metadata,
    /// schema, or auth pattern differs.
    #[tracing::instrument(name = "credential.metadata.readmit", skip_all, err)]
    pub fn readmit_against(
        &self,
        fresh_definition: &CredentialDefinition,
    ) -> Result<CredentialDefinition, CredentialReadmissionError> {
        self.base
            .readmit_against(&fresh_definition.base)
            .map_err(|_| CredentialReadmissionError::DefinitionMismatch)?;
        if self.pattern != fresh_definition.pattern {
            tracing::warn!(
                error_code = "CREDENTIAL:RECORDED_METADATA_MISMATCH",
                "recorded credential metadata rejected"
            );
            return Err(CredentialReadmissionError::DefinitionMismatch);
        }
        Ok(fresh_definition.clone())
    }
}

/// Payload-free recorded-metadata readmission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialReadmissionError {
    /// Recorded evidence differs from the fresh static definition.
    #[error("recorded credential metadata does not match the fresh definition")]
    DefinitionMismatch,
}

/// Compatibility validation errors for credential metadata evolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialCompatibilityError {
    /// A shared catalog rule fired.
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<CredentialKey>),
    /// Auth pattern changed without a major version bump.
    #[error("credential auth pattern changed without a major version bump")]
    PatternChangeWithoutMajorBump,
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
