//! Workspaces: scoped by parent org; slug is unique among active rows per
//! org, and an org has at most one active default workspace.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::WorkspaceRow;
use nebula_storage_port::store::WorkspaceStore;

use super::directory::SharedDirectory;
use super::{duplicate, now_rfc3339, version_conflict};

/// In-memory `workspaces` store — standalone, or a projection of an
/// [`InMemoryIdentityDirectory`](super::InMemoryIdentityDirectory).
#[derive(Debug, Default, Clone)]
pub struct InMemoryWorkspaceStore {
    inner: SharedDirectory,
}

impl InMemoryWorkspaceStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn over(inner: SharedDirectory) -> Self {
        Self { inner }
    }
}

/// Reject `row` when another active workspace of its org already has its
/// slug, or is already the default while `row` claims to be.
fn check_active_uniqueness<'a>(
    mut others: impl Iterator<Item = &'a WorkspaceRow>,
    row: &WorkspaceRow,
) -> Result<(), StorageError> {
    let wants_default = row.is_default && row.deleted_at.is_none();
    others.try_for_each(|other| {
        if other.id == row.id || other.org_id != row.org_id || other.deleted_at.is_some() {
            return Ok(());
        }
        if other.slug == row.slug {
            return Err(duplicate("workspace", "slug"));
        }
        if wants_default && other.is_default {
            return Err(StorageError::Duplicate {
                entity: "workspace",
                detail: "the organization already has an active default workspace".into(),
            });
        }
        Ok(())
    })
}

#[async_trait::async_trait]
impl WorkspaceStore for InMemoryWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let key = (row.org_id.clone(), row.id.clone());
        let mut state = self.inner.lock();
        let workspaces = &mut state.workspaces;
        if workspaces.contains_key(&key) {
            return Err(duplicate("workspace", "id"));
        }
        check_active_uniqueness(workspaces.values(), &row)?;
        workspaces.insert(key, row);
        Ok(())
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .workspaces
            .get(&(org_id.to_owned(), id.to_owned()))
            .filter(|row| row.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_slug(
        &self,
        org_id: &str,
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .workspaces
            .values()
            .find(|row| row.deleted_at.is_none() && row.org_id == org_id && row.slug == slug)
            .cloned())
    }

    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError> {
        let mut rows: Vec<WorkspaceRow> = self
            .inner
            .lock()
            .workspaces
            .values()
            .filter(|row| row.deleted_at.is_none() && row.org_id == org_id)
            .cloned()
            .collect();
        // Same order as the SQL backends' `ORDER BY id`.
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let key = (row.org_id.clone(), row.id.clone());
        let mut state = self.inner.lock();
        let workspaces = &mut state.workspaces;
        let Some(current) = workspaces.get(&key).filter(|w| w.deleted_at.is_none()) else {
            return Err(StorageError::not_found("workspace", row.id));
        };
        if current.version != expected_version {
            let actual = current.version;
            return Err(version_conflict(
                "workspace",
                row.id,
                expected_version,
                actual,
            ));
        }
        check_active_uniqueness(workspaces.values(), &row)?;
        workspaces.insert(key, row);
        Ok(())
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let Some(row) = state
            .workspaces
            .get_mut(&(org_id.to_owned(), id.to_owned()))
            .filter(|row| row.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("workspace", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}
