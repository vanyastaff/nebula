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

/// In-memory workflow-row store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryWorkflowStore {
    inner: Arc<Mutex<HashMap<WfKey, WorkflowRecord>>>,
}

impl InMemoryWorkflowStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
