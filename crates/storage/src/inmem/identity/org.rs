//! Orgs: slug is unique among active rows.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::OrgRow;
use nebula_storage_port::store::OrgStore;

use super::directory::SharedDirectory;
use super::{duplicate, now_rfc3339, version_conflict};

/// In-memory `orgs` store — standalone, or a projection of an
/// [`InMemoryIdentityDirectory`](super::InMemoryIdentityDirectory).
#[derive(Debug, Default, Clone)]
pub struct InMemoryOrgStore {
    inner: SharedDirectory,
}

impl InMemoryOrgStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn over(inner: SharedDirectory) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl OrgStore for InMemoryOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let orgs = &mut state.orgs;
        if orgs.contains_key(&row.id) {
            return Err(duplicate("org", "id"));
        }
        if orgs
            .values()
            .any(|org| org.deleted_at.is_none() && org.slug == row.slug)
        {
            return Err(duplicate("org", "slug"));
        }
        orgs.insert(row.id.clone(), row);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .orgs
            .get(id)
            .filter(|org| org.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .orgs
            .values()
            .find(|org| org.deleted_at.is_none() && org.slug == slug)
            .cloned())
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let orgs = &mut state.orgs;
        let Some(current) = orgs.get(&row.id).filter(|org| org.deleted_at.is_none()) else {
            return Err(StorageError::not_found("org", row.id));
        };
        if current.version != expected_version {
            let actual = current.version;
            return Err(version_conflict("org", row.id, expected_version, actual));
        }
        if orgs
            .values()
            .any(|org| org.id != row.id && org.deleted_at.is_none() && org.slug == row.slug)
        {
            return Err(duplicate("org", "slug"));
        }
        orgs.insert(row.id.clone(), row);
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let Some(row) = state
            .orgs
            .get_mut(id)
            .filter(|org| org.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("org", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}
