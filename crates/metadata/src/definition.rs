//! Intrinsic catalog-definition checks shared by leaves and plugin manifests.

use crate::{DeprecationNotice, MaturityLevel};

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
    /// A wire key was rejected by its typed key parser.
    #[classify(category = "validation", code = "METADATA:INVALID_KEY")]
    #[error("invalid metadata key")]
    InvalidKey,
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
