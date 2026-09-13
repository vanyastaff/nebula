//! Canonical discovery fields and the exact shared JSON representation.

use nebula_schema::ValidSchema;
use semver::Version;
use serde::Serialize;

use crate::bounded::{self, CATEGORY_COUNT, LINK_COUNT, PendingCollection};
use crate::defaults::{is_default_maturity, is_default_version};
use crate::{
    CatalogCategoryKey, CatalogLink, CatalogLinkRelation, DeprecationNotice, Icon,
    METADATA_WIRE_VERSION, MaturityLevel, MetadataError,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Discovery {
    pub(crate) categories: PendingCollection<CatalogCategoryKey>,
    pub(crate) tags: PendingCollection<String>,
    pub(crate) links: PendingCollection<CatalogLink>,
    overview_error: Option<MetadataError>,
}

impl Discovery {
    pub(crate) fn replace_links(&mut self, links: impl IntoIterator<Item = CatalogLink>) {
        self.links = PendingCollection::collect(links);
        self.overview_error = None;
    }

    pub(crate) fn set_overview(&mut self, target: &str) {
        self.links
            .retain(|link| link.relation() != CatalogLinkRelation::Overview);
        match target.parse() {
            Ok(target) => {
                self.links
                    .push(CatalogLink::new(CatalogLinkRelation::Overview, target));
                self.overview_error = None;
            },
            Err(error) => self.overview_error = Some(MetadataError::CatalogValue(error)),
        }
    }

    #[tracing::instrument(name = "metadata.canonicalize_discovery", skip_all, err)]
    pub(crate) fn canonicalize(&mut self) -> Result<(), MetadataError> {
        if let Some(error) = self.overview_error {
            return Err(error);
        }
        let mut categories = self
            .categories
            .take_checked(crate::MetadataField::Categories)?;
        categories.sort_unstable();
        categories.dedup();
        if categories.len() > CATEGORY_COUNT {
            return Err(MetadataError::TooManyEntries(
                crate::MetadataField::Categories,
            ));
        }
        self.categories.set_canonical(categories);
        let tags = bounded::canonical_tags(self.tags.take_checked(crate::MetadataField::Tags)?)?;
        self.tags.set_canonical(tags);
        let mut links = self.links.take_checked(crate::MetadataField::Links)?;
        links.sort_unstable();
        links.dedup();
        if links.len() > LINK_COUNT {
            return Err(MetadataError::TooManyEntries(crate::MetadataField::Links));
        }
        if links
            .iter()
            .filter(|link| link.relation() == CatalogLinkRelation::Overview)
            .count()
            > 1
        {
            return Err(MetadataError::ConflictingOverview);
        }
        self.links.set_canonical(links);
        Ok(())
    }

    pub(crate) fn documentation_url(&self) -> Option<&str> {
        self.links
            .as_slice()
            .iter()
            .find(|link| link.relation() == CatalogLinkRelation::Overview)
            .map(|link| link.target().as_str())
    }
}

// The same serializer is used for budget admission and emitted shared records.
// Passing no schema counts precisely the authored record, including JSON escaping.
#[derive(Serialize)]
pub(crate) struct SharedFields<'a, K> {
    pub(crate) metadata_wire_version: u32,
    pub(crate) key: &'a K,
    pub(crate) name: &'a str,
    pub(crate) description: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) schema: Option<&'a ValidSchema>,
    #[serde(skip_serializing_if = "is_default_version")]
    pub(crate) version: &'a Version,
    #[serde(skip_serializing_if = "Icon::is_none")]
    pub(crate) icon: &'a Icon,
    #[serde(skip_serializing_if = "<[CatalogCategoryKey]>::is_empty")]
    pub(crate) categories: &'a [CatalogCategoryKey],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    pub(crate) tags: &'a [String],
    #[serde(skip_serializing_if = "<[CatalogLink]>::is_empty")]
    pub(crate) links: &'a [CatalogLink],
    #[serde(skip_serializing_if = "is_default_maturity")]
    pub(crate) maturity: MaturityLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deprecation: Option<&'a DeprecationNotice>,
}

impl<K: Serialize> SharedFields<'_, K> {
    #[tracing::instrument(name = "metadata.validate_shared_fields", skip_all, err)]
    pub(crate) fn validate(&self) -> Result<(), MetadataError> {
        if self.metadata_wire_version != METADATA_WIRE_VERSION {
            return Err(MetadataError::UnsupportedWireVersion);
        }
        bounded::check_bytes(self.name, bounded::SHARED_BYTES, crate::MetadataField::Name)?;
        bounded::check_bytes(
            self.description,
            bounded::DESCRIPTION_BYTES,
            crate::MetadataField::Description,
        )?;
        if let Some(icon) = self.icon.as_inline().or_else(|| self.icon.as_url()) {
            bounded::check_bytes(icon, bounded::SHARED_BYTES, crate::MetadataField::Icon)?;
        }
        for suffix in [self.version.pre.as_str(), self.version.build.as_str()] {
            bounded::check_bytes(suffix, bounded::SHARED_BYTES, crate::MetadataField::Version)?;
        }
        if let Some(notice) = self.deprecation {
            notice.validate(self.version)?;
        }
        bounded::check_serialized(
            self,
            bounded::SHARED_BYTES,
            MetadataError::SharedBudgetExceeded,
        )?;
        // A prior counting pass cannot bound a later, custom serializer invocation.
        let key =
            bounded::capture_serialized(self.key, bounded::SHARED_BYTES, MetadataError::InvalidKey)
                .map_err(|_| MetadataError::InvalidKey)?;
        serde_json::from_slice::<String>(&key)
            .map(|_| ())
            .map_err(|_| MetadataError::InvalidKey)
    }
}
