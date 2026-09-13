//! Structured deprecation notice attached to metadata entries.

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::bounded::{self, SHARED_BYTES};
use crate::{CatalogReference, MetadataError, RemovalSchedule};

/// Deprecation payload describing when a catalog entity became deprecated,
/// when it will be removed, and what replaces it.
///
/// Attached to [`BaseMetadata::deprecation`](crate::BaseMetadata::deprecation);
/// its presence requires [`MaturityLevel::Deprecated`](crate::MaturityLevel::Deprecated)
/// through every catalog construction and deserialization path.
///
/// Removal and replacement are typed guidance. Admission checks `since` against
/// the source interface or bundle version. Elapsed schedules remain valid.
/// A replacement retains its typed entity family and may remain unresolved.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeprecationNotice {
    /// Version in which the deprecation was introduced.
    since: Version,
    /// Announced date, interface version, or checked milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removal: Option<RemovalSchedule>,
    /// Typed replacement guidance, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacement: Option<CatalogReference>,
    /// Human-readable reason shown in the catalog UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl DeprecationNotice {
    /// Build a deprecation notice with only the required `since` version set.
    #[must_use]
    pub fn new(since: Version) -> Self {
        Self {
            since,
            removal: None,
            replacement: None,
            reason: None,
        }
    }

    /// Declare when the entity is scheduled for removal.
    #[must_use]
    pub fn with_removal(mut self, removal: RemovalSchedule) -> Self {
        self.removal = Some(removal);
        self
    }

    /// Declare the replacement entity key.
    #[must_use]
    pub fn with_replacement(mut self, replacement: CatalogReference) -> Self {
        self.replacement = Some(replacement);
        self
    }

    /// Set the human-readable reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Source interface or bundle version introducing deprecation.
    #[must_use]
    pub fn since(&self) -> &Version {
        &self.since
    }

    /// Announced removal guidance, without enforcement semantics.
    #[must_use]
    pub fn removal(&self) -> Option<&RemovalSchedule> {
        self.removal.as_ref()
    }

    /// Typed replacement guidance, which may remain unresolved.
    #[must_use]
    pub fn replacement(&self) -> Option<&CatalogReference> {
        self.replacement.as_ref()
    }

    /// Human-readable reason, treated as text by renderers.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    #[tracing::instrument(name = "metadata.validate_deprecation", skip_all, err)]
    pub(crate) fn validate(&self, current: &Version) -> Result<(), MetadataError> {
        if self.since.cmp_precedence(current).is_gt() {
            return Err(MetadataError::FutureDeprecation);
        }
        self.validate_notice()
    }

    fn validate_notice(&self) -> Result<(), MetadataError> {
        if let Some(replacement) = &self.replacement {
            replacement.validate()?;
        }
        if let Some(reason) = &self.reason {
            bounded::check_bytes(
                reason,
                SHARED_BYTES,
                crate::MetadataField::DeprecationReason,
            )?;
        }
        if let Some(RemovalSchedule::AtVersion(removal)) = &self.removal
            && !removal.cmp_precedence(&self.since).is_gt()
        {
            return Err(MetadataError::InvalidRemovalOrder);
        }
        bounded::check_serialized(self, SHARED_BYTES, MetadataError::SharedBudgetExceeded)
    }
}

impl<'de> Deserialize<'de> for DeprecationNotice {
    #[tracing::instrument(name = "metadata.deserialize_deprecation", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            #[serde(deserialize_with = "bounded::version")]
            since: Version,
            #[serde(default)]
            removal: Option<RemovalSchedule>,
            #[serde(default)]
            replacement: Option<CatalogReference>,
            #[serde(
                default,
                deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
            )]
            reason: Option<String>,
        }
        let fields: Fields = crate::deserialize_metadata_object(deserializer)?;
        let notice = Self {
            since: fields.since,
            removal: fields.removal,
            replacement: fields.replacement,
            reason: fields.reason,
        };
        notice.validate_notice().map_err(D::Error::custom)?;
        Ok(notice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_notice_roundtrip() {
        let notice = DeprecationNotice::new(Version::new(1, 2, 0));
        let s = serde_json::to_string(&notice).unwrap();
        // Optional guidance is omitted when absent.
        assert_eq!(s, r#"{"since":"1.2.0"}"#);
        let back: DeprecationNotice = serde_json::from_str(&s).unwrap();
        assert_eq!(back, notice);
    }

    #[test]
    fn full_notice_builder() {
        let notice = DeprecationNotice::new(Version::new(2, 0, 0))
            .with_removal(RemovalSchedule::OnDate("2026-07-01".parse().unwrap()))
            .with_replacement(CatalogReference::action("http.request.v2".parse().unwrap()))
            .with_reason("superseded by unified HTTP node");
        assert_eq!(
            notice.removal(),
            Some(&RemovalSchedule::OnDate("2026-07-01".parse().unwrap()))
        );
        assert_eq!(
            notice.replacement().map(CatalogReference::key),
            Some("http.request.v2")
        );
    }
}
