//! Checked documentation links, resolved only with an explicit host origin.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::{Position, Url};

use crate::catalog_error::{
    CatalogValueError, deserialize_catalog_object, deserialize_catalog_string,
};

const MAX_LINK_TARGET_BYTES: usize = 2048;

/// The closed, deterministic ordering of catalog documentation relations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogLinkRelation {
    /// Primary documentation for the entity.
    Overview,
    /// Installation and configuration documentation.
    Setup,
    /// Detailed interface documentation.
    Reference,
    /// Guidance for moving between definitions or revisions.
    Migration,
    /// Diagnosis and recovery documentation.
    Troubleshooting,
}

impl FromStr for CatalogLinkRelation {
    type Err = CatalogValueError;

    #[tracing::instrument(name = "metadata.parse_link_relation", skip_all, err)]
    fn from_str(relation: &str) -> Result<Self, Self::Err> {
        match relation {
            "overview" => Ok(Self::Overview),
            "setup" => Ok(Self::Setup),
            "reference" => Ok(Self::Reference),
            "migration" => Ok(Self::Migration),
            "troubleshooting" => Ok(Self::Troubleshooting),
            _ => Err(CatalogValueError::InvalidLinkRelation),
        }
    }
}

impl<'de> Deserialize<'de> for CatalogLinkRelation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidLinkRelation,
            Self::from_str,
        )
    }
}

/// A checked relation/target pair. Publishing a link grants no fetch authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CatalogLink {
    relation: CatalogLinkRelation,
    target: CatalogLinkTarget,
}

impl CatalogLink {
    /// Pair a closed relation with an already checked target.
    #[must_use]
    pub const fn new(relation: CatalogLinkRelation, target: CatalogLinkTarget) -> Self {
        Self { relation, target }
    }

    /// The documentation relation.
    #[must_use]
    pub const fn relation(&self) -> CatalogLinkRelation {
        self.relation
    }

    /// The checked target, which may remain root-relative.
    #[must_use]
    pub const fn target(&self) -> &CatalogLinkTarget {
        &self.target
    }
}

impl<'de> Deserialize<'de> for CatalogLink {
    #[tracing::instrument(name = "metadata.deserialize_link", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            relation: CatalogLinkRelation,
            target: CatalogLinkTarget,
        }

        let fields: Fields =
            deserialize_catalog_object(deserializer, CatalogValueError::InvalidLink)?;
        Ok(Self::new(fields.relation, fields.target))
    }
}

/// A canonical HTTPS URL or a root-relative documentation path.
///
/// Both input and canonical representation are limited to 2048 UTF-8 bytes.
/// Credentials, scheme-relative paths, backslashes, control characters, and
/// surrounding whitespace are rejected. No parsing or resolution performs I/O.
/// Root-relative values stay unresolved unless a host supplies a
/// [`DocumentationOrigin`] to [`Self::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CatalogLinkTarget(String);

impl CatalogLinkTarget {
    /// Borrow the canonical URL or root-relative path, including query/fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this target requires an explicit documentation origin to resolve.
    #[must_use]
    pub fn is_root_relative(&self) -> bool {
        self.0.starts_with('/')
    }

    /// Resolve to an absolute checked HTTPS target using a host documentation origin.
    ///
    /// Absolute HTTPS targets keep their authored external origin. A relative
    /// target must resolve to the supplied origin, including scheme and port.
    /// Hosts without a documentation origin leave relative links unresolved;
    /// there is no implicit browser-origin fallback.
    /// The successful result always has [`Self::is_root_relative`] equal to false.
    ///
    /// # Errors
    /// Returns [`CatalogValueError::InvalidLinkTarget`] if URL resolution fails,
    /// [`CatalogValueError::LinkOriginMismatch`] if a relative link escapes, or
    /// [`CatalogValueError::LinkTargetTooLong`] if the absolute target exceeds
    /// 2048 bytes after combining the origin and path.
    ///
    /// ```
    /// use nebula_metadata::{CatalogLinkTarget, DocumentationOrigin};
    /// let origin: DocumentationOrigin = "https://docs.example.com".parse()?;
    /// let target: CatalogLinkTarget = "/guides/setup".parse()?;
    /// assert_eq!(target.resolve(&origin)?.as_str(), "https://docs.example.com/guides/setup");
    /// # Ok::<(), nebula_metadata::CatalogValueError>(())
    /// ```
    #[tracing::instrument(name = "metadata.resolve_link_target", skip_all, err)]
    pub fn resolve(&self, origin: &DocumentationOrigin) -> Result<Self, CatalogValueError> {
        if !self.is_root_relative() {
            return Ok(self.clone());
        }

        let resolved = origin
            .0
            .join(self.as_str())
            .map_err(|_| CatalogValueError::InvalidLinkTarget)?;
        if resolved.origin() != origin.0.origin() {
            return Err(CatalogValueError::LinkOriginMismatch);
        }
        Self::try_from(resolved.as_str())
    }
}

impl TryFrom<&str> for CatalogLinkTarget {
    type Error = CatalogValueError;

    #[tracing::instrument(name = "metadata.parse_link_target", skip_all, err)]
    fn try_from(target: &str) -> Result<Self, Self::Error> {
        if target.len() > MAX_LINK_TARGET_BYTES {
            return Err(CatalogValueError::LinkTargetTooLong);
        }
        if has_forbidden_url_spelling(target) {
            return Err(CatalogValueError::InvalidLinkTarget);
        }

        let canonical = if target.starts_with('/') {
            if target.starts_with("//") {
                return Err(CatalogValueError::InvalidLinkTarget);
            }
            // This reserved origin is a parser base only. It is never retained
            // as a link origin, exposed to a host, resolved through DNS, or fetched.
            let base = Url::parse("https://catalog.invalid/")
                .map_err(|_| CatalogValueError::InvalidLinkTarget)?;
            let parsed = base
                .join(target)
                .map_err(|_| CatalogValueError::InvalidLinkTarget)?;
            if parsed.origin() != base.origin() {
                return Err(CatalogValueError::LinkOriginMismatch);
            }
            let path = &parsed[Position::BeforePath..];
            // Dot-segment normalization can produce a leading double slash.
            // Such a serialization would become an authority on the next join.
            if !path.starts_with('/') || path.starts_with("//") {
                return Err(CatalogValueError::InvalidLinkTarget);
            }
            path.to_owned()
        } else {
            parse_https(target)?.into()
        };
        if canonical.len() > MAX_LINK_TARGET_BYTES {
            return Err(CatalogValueError::LinkTargetTooLong);
        }
        Ok(Self(canonical))
    }
}

impl TryFrom<String> for CatalogLinkTarget {
    type Error = CatalogValueError;

    fn try_from(target: String) -> Result<Self, Self::Error> {
        Self::try_from(target.as_str())
    }
}

impl FromStr for CatalogLinkTarget {
    type Err = CatalogValueError;

    fn from_str(target: &str) -> Result<Self, Self::Err> {
        Self::try_from(target)
    }
}

impl fmt::Display for CatalogLinkTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CatalogLinkTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidLinkTarget,
            Self::from_str,
        )
    }
}

/// An explicitly configured HTTPS documentation origin.
///
/// Credentials, non-root paths, queries, and fragments are rejected, even when
/// normalization would discard them. The canonical origin includes a root slash
/// and retains non-default ports. It is a resolution base, not fetch authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentationOrigin(Url);

impl DocumentationOrigin {
    /// Borrow the canonical HTTPS origin with its root slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<&str> for DocumentationOrigin {
    type Error = CatalogValueError;

    #[tracing::instrument(name = "metadata.parse_documentation_origin", skip_all, err)]
    fn try_from(origin: &str) -> Result<Self, Self::Error> {
        if origin.len() > MAX_LINK_TARGET_BYTES || has_forbidden_url_spelling(origin) {
            return Err(CatalogValueError::InvalidDocumentationOrigin);
        }
        let parsed =
            parse_https(origin).map_err(|_| CatalogValueError::InvalidDocumentationOrigin)?;
        let authority_and_path = &origin["https://".len()..];
        if parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || authority_and_path
                .split_once('/')
                .is_some_and(|(_, path)| !path.is_empty())
            || parsed.as_str().len() > MAX_LINK_TARGET_BYTES
        {
            return Err(CatalogValueError::InvalidDocumentationOrigin);
        }
        Ok(Self(parsed))
    }
}

impl TryFrom<String> for DocumentationOrigin {
    type Error = CatalogValueError;

    fn try_from(origin: String) -> Result<Self, Self::Error> {
        Self::try_from(origin.as_str())
    }
}

impl FromStr for DocumentationOrigin {
    type Err = CatalogValueError;

    fn from_str(origin: &str) -> Result<Self, Self::Err> {
        Self::try_from(origin)
    }
}

impl fmt::Display for DocumentationOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DocumentationOrigin {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DocumentationOrigin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidDocumentationOrigin,
            Self::from_str,
        )
    }
}

fn has_forbidden_url_spelling(input: &str) -> bool {
    input.is_empty()
        || input.trim() != input
        || input
            .chars()
            .any(|character| character == '\\' || character.is_control())
}

fn parse_https(input: &str) -> Result<Url, CatalogValueError> {
    // URL parsing deliberately repairs some inputs. Require an explicit HTTPS
    // authority and reject userinfo syntax, including an empty username which
    // the parser otherwise removes from the canonical representation.
    let prefix = input
        .get(.."https://".len())
        .ok_or(CatalogValueError::InvalidLinkTarget)?;
    if !prefix.eq_ignore_ascii_case("https://") {
        return Err(CatalogValueError::InvalidLinkTarget);
    }
    let authority = input[prefix.len()..]
        .split(['/', '?', '#'])
        .next()
        .ok_or(CatalogValueError::InvalidLinkTarget)?;
    if authority.is_empty() || authority.contains('@') {
        return Err(CatalogValueError::InvalidLinkTarget);
    }
    let parsed = Url::parse(input).map_err(|_| CatalogValueError::InvalidLinkTarget)?;
    if parsed.scheme() != "https"
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(CatalogValueError::InvalidLinkTarget);
    }
    Ok(parsed)
}
