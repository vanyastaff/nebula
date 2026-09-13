//! Payload-free failures for checked catalog values.

use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, Visitor, value::MapAccessDeserializer},
};

/// A catalog value failed a structural or domain invariant.
///
/// Variants identify the field and constraint without retaining submitted text,
/// URL components, or parser errors. Error sources are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum CatalogValueError {
    /// A category key is empty.
    #[classify(category = "validation", code = "CATALOG:EMPTY_CATEGORY_KEY")]
    #[error("catalog category key must not be empty")]
    EmptyCategoryKey,
    /// A category key exceeds its byte limit.
    #[classify(category = "validation", code = "CATALOG:CATEGORY_KEY_TOO_LONG")]
    #[error("catalog category key exceeds 96 bytes")]
    CategoryKeyTooLong,
    /// A category key is not a lowercase structured identifier.
    #[classify(category = "validation", code = "CATALOG:INVALID_CATEGORY_KEY")]
    #[error("invalid catalog category key")]
    InvalidCategoryKey,
    /// A link relation is not one of the closed catalog relations.
    #[classify(category = "validation", code = "CATALOG:INVALID_LINK_RELATION")]
    #[error("invalid catalog link relation")]
    InvalidLinkRelation,
    /// A link object has an invalid shape or field.
    #[classify(category = "validation", code = "CATALOG:INVALID_LINK")]
    #[error("invalid catalog link")]
    InvalidLink,
    /// A target is neither an admitted HTTPS URL nor an unambiguous root path.
    #[classify(category = "validation", code = "CATALOG:INVALID_LINK_TARGET")]
    #[error("invalid catalog link target")]
    InvalidLinkTarget,
    /// An input or canonical link target exceeds its byte limit.
    #[classify(category = "validation", code = "CATALOG:LINK_TARGET_TOO_LONG")]
    #[error("catalog link target exceeds 2048 bytes")]
    LinkTargetTooLong,
    /// A documentation origin is not an HTTPS origin without other components.
    #[classify(category = "validation", code = "CATALOG:INVALID_DOCUMENTATION_ORIGIN")]
    #[error("invalid documentation origin")]
    InvalidDocumentationOrigin,
    /// A resolved root-relative target would leave its configured origin.
    #[classify(category = "validation", code = "CATALOG:LINK_ORIGIN_MISMATCH")]
    #[error("catalog link resolution changed the documentation origin")]
    LinkOriginMismatch,
    /// A reference has an unknown family or an invalid object shape.
    #[classify(category = "validation", code = "CATALOG:INVALID_REFERENCE")]
    #[error("invalid catalog reference")]
    InvalidReference,
    /// An action reference key failed the core key parser.
    #[classify(category = "validation", code = "CATALOG:INVALID_ACTION_REFERENCE_KEY")]
    #[error("invalid catalog action reference key")]
    InvalidActionReferenceKey,
    /// A credential reference key failed the core key parser.
    #[classify(
        category = "validation",
        code = "CATALOG:INVALID_CREDENTIAL_REFERENCE_KEY"
    )]
    #[error("invalid catalog credential reference key")]
    InvalidCredentialReferenceKey,
    /// A resource reference key failed the core key parser.
    #[classify(
        category = "validation",
        code = "CATALOG:INVALID_RESOURCE_REFERENCE_KEY"
    )]
    #[error("invalid catalog resource reference key")]
    InvalidResourceReferenceKey,
    /// A plugin reference key failed the core key parser.
    #[classify(category = "validation", code = "CATALOG:INVALID_PLUGIN_REFERENCE_KEY")]
    #[error("invalid catalog plugin reference key")]
    InvalidPluginReferenceKey,
    /// A target version requirement is invalid, exceeds catalog limits, or
    /// cannot preserve its native comparator fields through the wire grammar.
    #[classify(
        category = "validation",
        code = "CATALOG:INVALID_REFERENCE_VERSION_REQUIREMENT"
    )]
    #[error("invalid catalog reference version requirement")]
    InvalidReferenceVersionRequirement,
    /// A removal schedule has an unknown kind or an invalid object shape.
    #[classify(category = "validation", code = "CATALOG:INVALID_REMOVAL_SCHEDULE")]
    #[error("invalid catalog removal schedule")]
    InvalidRemovalSchedule,
    /// A removal date is not a canonical calendar date in years 1 through 9999.
    #[classify(category = "validation", code = "CATALOG:INVALID_REMOVAL_DATE")]
    #[error("removal date must be a valid YYYY-MM-DD date in years 0001 through 9999")]
    InvalidRemovalDate,
    /// A removal version is not a valid SemVer version.
    #[classify(category = "validation", code = "CATALOG:INVALID_REMOVAL_VERSION")]
    #[error("invalid catalog removal version")]
    InvalidRemovalVersion,
    /// A removal milestone contains only whitespace.
    #[classify(category = "validation", code = "CATALOG:BLANK_REMOVAL_MILESTONE")]
    #[error("removal milestone must contain non-whitespace text")]
    BlankRemovalMilestone,
    /// A trimmed removal milestone exceeds its byte limit.
    #[classify(category = "validation", code = "CATALOG:REMOVAL_MILESTONE_TOO_LONG")]
    #[error("removal milestone exceeds 256 bytes")]
    RemovalMilestoneTooLong,
    /// A removal milestone was encoded as something other than text.
    #[classify(category = "validation", code = "CATALOG:INVALID_REMOVAL_MILESTONE")]
    #[error("invalid removal milestone")]
    InvalidRemovalMilestone,
}

// Validate borrowed text before copying it into a leaf value. The visitor also
// works with owned JSON and readers; malformed types never echo their payload.
pub(crate) fn deserialize_catalog_string<'de, D, T>(
    deserializer: D,
    invalid: CatalogValueError,
    parse: fn(&str) -> Result<T, CatalogValueError>,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    struct CatalogStringVisitor<T>(fn(&str) -> Result<T, CatalogValueError>);

    impl<T> Visitor<'_> for CatalogStringVisitor<T> {
        type Value = Result<T, CatalogValueError>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a catalog string")
        }

        fn visit_str<E>(self, text: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok((self.0)(text))
        }
    }

    deserializer
        .deserialize_str(CatalogStringVisitor(parse))
        .map_err(|_| D::Error::custom(invalid))?
        .map_err(D::Error::custom)
}

// Serde's derived structs also accept positional sequences. Catalog wire
// records require named objects, and all structural failures must be sanitized.
pub(crate) fn deserialize_catalog_object<'de, D, T>(
    deserializer: D,
    invalid: CatalogValueError,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct CatalogObjectVisitor<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for CatalogObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a catalog object")
        }

        fn visit_map<A: MapAccess<'de>>(self, fields: A) -> Result<Self::Value, A::Error> {
            T::deserialize(MapAccessDeserializer::new(fields))
        }
    }

    deserializer
        .deserialize_map(CatalogObjectVisitor(PhantomData))
        .map_err(|_| D::Error::custom(invalid))
}
