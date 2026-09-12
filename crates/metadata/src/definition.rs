//! Intrinsic catalog-definition checks shared by leaves and plugin manifests.

use crate::{DeprecationNotice, MaturityLevel};

/// Closed field locations for metadata diagnostics, never supplied path text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataField {
    /// Typed entity identity.
    Key,
    /// Display name.
    Name,
    /// Interface or bundle SemVer.
    Version,
    /// Shared fallback description.
    Description,
    /// Catalog icon.
    Icon,
    /// Structured categories.
    Categories,
    /// Search tags.
    Tags,
    /// Documentation links.
    Links,
    /// Human-readable deprecation reason.
    DeprecationReason,
    /// Manifest group hierarchy.
    ManifestGroup,
    /// Manifest color.
    ManifestColor,
    /// Manifest author.
    ManifestAuthor,
    /// Manifest license.
    ManifestLicense,
    /// Manifest homepage.
    ManifestHomepage,
    /// Manifest repository URL.
    ManifestRepository,
    /// Manifest dependency declarations.
    ManifestDependencies,
    /// Manifest minimum engine version.
    ManifestNebulaVersion,
}

impl std::fmt::Display for MetadataField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Key => "key",
            Self::Name => "name",
            Self::Version => "version",
            Self::Description => "description",
            Self::Icon => "icon",
            Self::Categories => "categories",
            Self::Tags => "tags",
            Self::Links => "links",
            Self::DeprecationReason => "deprecation.reason",
            Self::ManifestGroup => "manifest.group",
            Self::ManifestColor => "manifest.color",
            Self::ManifestAuthor => "manifest.author",
            Self::ManifestLicense => "manifest.license",
            Self::ManifestHomepage => "manifest.homepage",
            Self::ManifestRepository => "manifest.repository",
            Self::ManifestDependencies => "manifest.dependencies",
            Self::ManifestNebulaVersion => "manifest.nebula_version",
        })
    }
}

/// An intrinsic catalog-definition invariant failed during construction.
///
/// These errors contain no submitted keys, names, schemas, or notice payloads.
/// Revision compatibility is checked separately by [`crate::validate_base_compat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum MetadataError {
    /// The display name is empty or contains only whitespace.
    #[classify(category = "validation", code = "METADATA:BLANK_NAME")]
    #[error("metadata name must contain non-whitespace text")]
    BlankName,
    /// Deprecated maturity was requested without a notice.
    #[classify(category = "validation", code = "METADATA:MISSING_DEPRECATION")]
    #[error("deprecated metadata requires a deprecation notice")]
    MissingDeprecationNotice,
    /// A key was rejected by its parser or did not serialize as a JSON string.
    #[classify(category = "validation", code = "METADATA:INVALID_KEY")]
    #[error("invalid metadata key")]
    InvalidKey,
    /// A version or requirement cannot retain its intent through SemVer wire parsing.
    #[classify(category = "validation", code = "METADATA:INVALID_VERSION")]
    #[error("invalid metadata {0}")]
    InvalidVersion(MetadataField),
    /// A shared string exceeds its UTF-8 byte limit.
    #[classify(category = "validation", code = "METADATA:FIELD_TOO_LARGE")]
    #[error("metadata {0} exceeds its byte limit")]
    FieldTooLarge(MetadataField),
    /// A collection exceeds its canonical or raw entry limit.
    #[classify(category = "validation", code = "METADATA:TOO_MANY_ENTRIES")]
    #[error("metadata {0} exceeds its canonical entry limit")]
    TooManyEntries(MetadataField),
    /// Raw entries exceeded the fixed collection work budget before deduplication.
    #[classify(category = "validation", code = "METADATA:TOO_MANY_RAW_ENTRIES")]
    #[error("metadata {0} exceeds its raw entry limit")]
    TooManyRawEntries(MetadataField),
    /// Search tags must contain text after trimming.
    #[classify(category = "validation", code = "METADATA:BLANK_TAG")]
    #[error("metadata tag must contain non-whitespace text")]
    BlankTag,
    /// More than one distinct Overview target was authored.
    #[classify(category = "validation", code = "METADATA:CONFLICTING_OVERVIEW")]
    #[error("metadata has conflicting overview links")]
    ConflictingOverview,
    /// A checked supporting value failed its owning parser.
    #[classify(category = "validation", code = "METADATA:INVALID_CATALOG_VALUE")]
    #[error("invalid metadata catalog value: {0}")]
    CatalogValue(#[from] crate::CatalogValueError),
    /// The notice starts after the current interface or bundle version.
    #[classify(category = "validation", code = "METADATA:FUTURE_DEPRECATION")]
    #[error("metadata deprecation since must not exceed the current version")]
    FutureDeprecation,
    /// Announced version removal must follow the deprecation version.
    #[classify(category = "validation", code = "METADATA:INVALID_REMOVAL_ORDER")]
    #[error("metadata removal version must follow deprecation since")]
    InvalidRemovalOrder,
    /// Exact canonical serialized authored fields exceed the shared budget.
    #[classify(category = "validation", code = "METADATA:SHARED_BUDGET")]
    #[error("metadata shared authored fields exceed their serialized byte budget")]
    SharedBudgetExceeded,
    /// The whole manifest, including separate packaging data, is oversized.
    #[classify(category = "validation", code = "METADATA:MANIFEST_BUDGET")]
    #[error("metadata manifest exceeds its serialized byte budget")]
    ManifestBudgetExceeded,
    /// Bound schema evidence exceeds its separate serialized budget.
    #[classify(category = "validation", code = "METADATA:SCHEMA_BUDGET")]
    #[error("metadata schema exceeds its serialized byte budget")]
    SchemaBudgetExceeded,
    /// The complete canonical record exceeds the default transport ceiling.
    #[classify(category = "validation", code = "METADATA:RECORD_TOO_LARGE")]
    #[error("metadata record exceeds its serialized byte budget")]
    RecordTooLarge,
    /// Canonical JSON cannot be decoded as an object with the default parser limits.
    #[classify(category = "validation", code = "METADATA:RECORD_NOT_DECODABLE")]
    #[error("metadata record exceeds JSON shape or nesting limits")]
    RecordNotDecodable,
    /// A supplied serializer failed; its diagnostic is deliberately discarded.
    #[classify(category = "validation", code = "METADATA:SERIALIZATION")]
    #[error("metadata serialization failed")]
    SerializationFailed,
    /// Structural wire parsing failed; parser diagnostics are deliberately discarded.
    #[classify(category = "validation", code = "METADATA:INVALID_WIRE")]
    #[error("invalid metadata wire fields or version")]
    InvalidWire,
    /// The required shared record discriminator is unsupported.
    #[classify(category = "validation", code = "METADATA:WIRE_VERSION")]
    #[error("unsupported metadata wire version")]
    UnsupportedWireVersion,
}

/// A catalog definition could not be built from intrinsic data or a type's schema.
///
/// Display, debug and tracing omit authored schema diagnostics. The original
/// report remains available through the typed variant and error source.
#[derive(Clone, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum MetadataBuildError {
    /// An intrinsic metadata invariant failed.
    #[classify(category = "validation", code = "METADATA:INVALID_DEFINITION")]
    #[error("invalid catalog definition")]
    Metadata(#[from] MetadataError),
    /// Schema construction failed before catalog admission.
    #[classify(category = "validation", code = "METADATA:INVALID_SCHEMA")]
    #[error("invalid catalog schema")]
    Schema(#[source] nebula_schema::ValidationReport),
    /// A cached schema disagrees with the schema derived from its declared Rust type.
    #[classify(category = "validation", code = "METADATA:SCHEMA_CONTRACT_MISMATCH")]
    #[error("catalog schema does not match its declared type contract")]
    SchemaContractMismatch,
}

/// Recorded metadata could not be re-admitted as a fresh static definition.
///
/// The error intentionally carries no recorded or authored field values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum MetadataReadmissionError {
    /// At least one recorded field differs from the fresh static definition.
    #[classify(
        category = "validation",
        code = "METADATA:RECORDED_DEFINITION_MISMATCH"
    )]
    #[error("recorded metadata does not match the fresh static definition")]
    DefinitionMismatch,
}

impl From<nebula_schema::ValidationReport> for MetadataBuildError {
    #[tracing::instrument(name = "metadata.reject_schema", skip_all)]
    fn from(report: nebula_schema::ValidationReport) -> Self {
        tracing::warn!(code = "METADATA:INVALID_SCHEMA", "catalog schema rejected");
        Self::Schema(report)
    }
}

impl std::fmt::Debug for MetadataBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Metadata(error) => f.debug_tuple("Metadata").field(error).finish(),
            Self::Schema(_) => f.debug_struct("Schema").finish_non_exhaustive(),
            Self::SchemaContractMismatch => f.write_str("SchemaContractMismatch"),
        }
    }
}

#[tracing::instrument(name = "metadata.validate_name", skip_all, err)]
pub(crate) fn validate_name(name: &str) -> Result<(), MetadataError> {
    if crate::name::is_blank(name) {
        return Err(MetadataError::BlankName);
    }
    Ok(())
}

#[tracing::instrument(
    name = "metadata.resolve_maturity",
    skip_all,
    fields(maturity = ?maturity, has_deprecation = notice.is_some()),
    err
)]
pub(crate) fn resolve_maturity(
    maturity: MaturityLevel,
    notice: Option<&DeprecationNotice>,
) -> Result<MaturityLevel, MetadataError> {
    if notice.is_some() {
        return Ok(MaturityLevel::Deprecated);
    }
    if maturity == MaturityLevel::Deprecated {
        return Err(MetadataError::MissingDeprecationNotice);
    }
    Ok(maturity)
}
