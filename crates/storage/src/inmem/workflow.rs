//! In-memory `WorkflowStore` + `WorkflowVersionStore`.
//!
//! Spec-16 splits the workflow aggregate (the workflow row: id / slug /
//! soft-delete / CAS version) from its versions (each carrying the opaque
//! definition payload). Both are `parking_lot::Mutex`-guarded maps keyed
//! with the tenant scope folded in, so a cross-tenant `get` returns
//! `Ok(None)` exactly as the SQL backends' `WHERE workspace_id = ? AND
//! org_id = ?` predicate would — an id that is not in the caller's scope
//! is indistinguishable from one that does not exist (no existence
//! oracle).

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

/// Workflow-row key: `(workspace_id, org_id, workflow_id)`.
type WfKey = (String, String, String);

/// Workflow-version key: `(workspace_id, org_id, workflow_id, number)`.
type WfVerKey = (String, String, String, u32);

fn wf_key(scope: &Scope, id: &str) -> WfKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        id.to_string(),
    )
}

fn wf_ver_key(scope: &Scope, workflow_id: &str, number: u32) -> WfVerKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        workflow_id.to_string(),
        number,
    )
}

/// Shared workflow-version map handle. The version store owns this map;
/// [`InMemoryWorkflowStore`] holds a clone of the same `Arc` so
/// [`WorkflowStore::save_with_published_version`] can mutate the row map
/// **and** the version map inside one critical section (both-or-neither),
/// mirroring how [`super::InMemoryControlQueue`] / journal reader build
/// over the shared [`super::InMemoryExecutionStore`] core.
type SharedVersions = Arc<Mutex<HashMap<WfVerKey, WorkflowVersionRecord>>>;

/// In-memory workflow-row store.
///
/// Constructed **only** from its paired [`InMemoryWorkflowVersionStore`]
/// via [`Self::new_with_versions`], so the version map is *always* shared
/// between the two. There is deliberately no parameterless constructor:
/// [`WorkflowStore::save_with_published_version`] writes the published
/// version into [`Self::versions`], and a private unshared map would make
/// that write invisible to every `WorkflowVersionStore` reader (a
/// just-created workflow would 404). This mirrors
/// [`super::InMemoryControlQueue`] / [`super::InMemoryJournalReader`],
/// which are likewise built *from* the shared [`super::InMemoryExecutionStore`]
/// core and have no standalone constructor — sharing is structural, not a
/// caller discipline.
#[derive(Debug, Clone)]
pub struct InMemoryWorkflowStore {
    inner: Arc<Mutex<HashMap<WfKey, WorkflowRecord>>>,
    /// The *same* map the paired [`InMemoryWorkflowVersionStore`]
    /// reads/writes (an `Arc` clone of its `inner`), so the atomic save
    /// and the version-read path observe one state.
    versions: SharedVersions,
}

impl InMemoryWorkflowStore {
    /// Create a workflow-row store that shares its version map with
    /// `versions`, so [`WorkflowStore::save_with_published_version`]
    /// commits the row and the version atomically and the paired
    /// [`InMemoryWorkflowVersionStore`] observes the same data (the
    /// composition-root wiring — mirrors
    /// [`super::InMemoryControlQueue::new`] over a shared execution core).
    #[must_use]
    pub fn new_with_versions(versions: &InMemoryWorkflowVersionStore) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            versions: Arc::clone(&versions.inner),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStore for InMemoryWorkflowStore {
    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        let key = wf_key(scope, &record.id);
        let mut map = self.inner.lock();
        if map.contains_key(&key) {
            return Err(StorageError::Duplicate {
                entity: "workflow",
                detail: format!("workflow {} already exists", record.id),
            });
        }
        map.insert(key, record);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        let map = self.inner.lock();
        // A soft-deleted row is a miss for the read path (callers that
        // need tombstones use `list` semantics on a future variant).
        Ok(map.get(&wf_key(scope, id)).filter(|r| !r.deleted).cloned())
    }

    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        let map = self.inner.lock();
        Ok(map
            .iter()
            .find(|((ws, org, _), r)| {
                ws == &scope.workspace_id && org == &scope.org_id && r.slug == slug && !r.deleted
            })
            .map(|(_, r)| r.clone()))
    }

    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let key = wf_key(scope, &record.id);
        let mut map = self.inner.lock();
        // Read the current version out of the borrow so the error paths
        // can move `record.id` without an extra clone.
        let current_version = match map.get(&key) {
            Some(current) => current.version,
            None => return Err(StorageError::not_found("workflow", record.id)),
        };
        if current_version != expected_version {
            return Err(StorageError::Conflict {
                entity: "workflow",
                id: record.id,
                expected: expected_version,
                actual: current_version,
            });
        }
        map.insert(key, record);
        Ok(())
    }

    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: Option<u64>,
    ) -> Result<(), StorageError> {
        let row_key = wf_key(scope, &row.id);
        let ver_key = wf_ver_key(scope, &version.workflow_id, version.number);
        // Lock the row map and the version map together so the pair is
        // applied (or rejected) as one unit — no orphan-row window. Lock
        // order is fixed (rows then versions) and no other path takes
        // both, so this cannot deadlock.
        let mut rows = self.inner.lock();
        let mut vers = self.versions.lock();

        match expected_version {
            None => {
                // Create: the row must not exist and the version slot must
                // be free. Validate both before mutating either.
                if rows.contains_key(&row_key) {
                    return Err(StorageError::Duplicate {
                        entity: "workflow",
                        detail: format!("workflow {} already exists", row.id),
                    });
                }
                if vers.contains_key(&ver_key) {
                    return Err(StorageError::Duplicate {
                        entity: "workflow_version",
                        detail: format!(
                            "workflow {} version {} already exists",
                            version.workflow_id, version.number
                        ),
                    });
                }
                rows.insert(row_key, row);
                vers.insert(ver_key, version);
                Ok(())
            },
            Some(expected) => {
                // CAS update: the row must exist at `expected` and the new
                // version slot must be free. Validate both before mutating.
                let current_version = match rows.get(&row_key) {
                    Some(current) => current.version,
                    None => return Err(StorageError::not_found("workflow", row.id)),
                };
                if current_version != expected {
                    return Err(StorageError::Conflict {
                        entity: "workflow",
                        id: row.id,
                        expected,
                        actual: current_version,
                    });
                }
                if vers.contains_key(&ver_key) {
                    return Err(StorageError::Duplicate {
                        entity: "workflow_version",
                        detail: format!(
                            "workflow {} version {} already exists",
                            version.workflow_id, version.number
                        ),
                    });
                }
                rows.insert(row_key, row);
                vers.insert(ver_key, version);
                Ok(())
            },
        }
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        let Some(row) = map.get_mut(&wf_key(scope, id)) else {
            return Err(StorageError::not_found("workflow", id));
        };
        row.deleted = true;
        Ok(())
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        let map = self.inner.lock();
        let mut rows: Vec<WorkflowRecord> = map
            .iter()
            .filter(|((ws, org, _), r)| {
                ws == &scope.workspace_id && org == &scope.org_id && !r.deleted
            })
            .map(|(_, r)| r.clone())
            .collect();
        // Stable order by id so list output is deterministic across runs.
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }
}

/// In-memory workflow-version store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryWorkflowVersionStore {
    inner: Arc<Mutex<HashMap<WfVerKey, WorkflowVersionRecord>>>,
}

impl InMemoryWorkflowVersionStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl WorkflowVersionStore for InMemoryWorkflowVersionStore {
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        let key = wf_ver_key(scope, &record.workflow_id, record.number);
        let mut map = self.inner.lock();
        if map.contains_key(&key) {
            return Err(StorageError::Duplicate {
                entity: "workflow_version",
                detail: format!(
                    "workflow {} version {} already exists",
                    record.workflow_id, record.number
                ),
            });
        }
        map.insert(key, record);
        Ok(())
    }

    async fn get(
        &self,
        scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        let map = self.inner.lock();
        Ok(map.get(&wf_ver_key(scope, workflow_id, number)).cloned())
    }

    async fn get_published(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        let map = self.inner.lock();
        // Highest-numbered published version wins. `HashMap` iteration
        // order is unspecified, so `find` would return an arbitrary
        // published row when more than one is marked published (e.g. a
        // stale publish that was never cleared) — `max_by_key` makes the
        // result deterministic and matches the SQL backends'
        // `ORDER BY number DESC LIMIT 1`.
        Ok(map
            .iter()
            .filter(|((ws, org, wf, _), r)| {
                ws == &scope.workspace_id
                    && org == &scope.org_id
                    && wf == workflow_id
                    && r.published
            })
            .max_by_key(|((.., number), _)| *number)
            .map(|(_, r)| r.clone()))
    }

    async fn list(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        let map = self.inner.lock();
        let mut rows: Vec<WorkflowVersionRecord> = map
            .iter()
            .filter(|((ws, org, wf, _), _)| {
                ws == &scope.workspace_id && org == &scope.org_id && wf == workflow_id
            })
            .map(|(_, r)| r.clone())
            .collect();
        // Newest first (highest version number first).
        rows.sort_by_key(|r| std::cmp::Reverse(r.number));
        Ok(rows)
    }
}
