//! In-memory `WorkflowStore` + `WorkflowVersionStore`.
//!
//! The workflow row (id / slug / soft delete / CAS version) and its versions
//! (each carrying the opaque definition payload) live in one
//! `parking_lot::Mutex`-guarded state shared by both stores, so a version
//! needs its workflow and a save writes the row and the version atomically —
//! the relational contract of `workflows` / `workflow_versions`. Keys fold
//! the tenant scope in, so a cross-tenant `get` returns `Ok(None)` (no
//! existence oracle). A soft-deleted workflow is invisible to every read and
//! write.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

/// Workflow-row key: `(org_id, workspace_id, workflow_id)`.
type WorkflowKey = (String, String, String);

/// Workflow-version key: `(org_id, workspace_id, workflow_id, number)`.
type VersionKey = (String, String, String, u32);

fn workflow_key(scope: &Scope, id: &str) -> WorkflowKey {
    (
        scope.org_id.clone(),
        scope.workspace_id.clone(),
        id.to_owned(),
    )
}

fn version_key(scope: &Scope, workflow_id: &str, number: u32) -> VersionKey {
    (
        scope.org_id.clone(),
        scope.workspace_id.clone(),
        workflow_id.to_owned(),
        number,
    )
}

#[derive(Debug)]
struct StoredWorkflow {
    record: WorkflowRecord,
    deleted: bool,
}

#[derive(Debug, Default)]
struct WorkflowState {
    rows: HashMap<WorkflowKey, StoredWorkflow>,
    versions: HashMap<VersionKey, WorkflowVersionRecord>,
}

impl WorkflowState {
    fn live(&self, scope: &Scope, id: &str) -> Option<&WorkflowRecord> {
        self.rows
            .get(&workflow_key(scope, id))
            .filter(|stored| !stored.deleted)
            .map(|stored| &stored.record)
    }

    fn live_in_scope<'a>(&'a self, scope: &'a Scope) -> impl Iterator<Item = &'a WorkflowRecord> {
        self.rows
            .iter()
            .filter(move |((org, workspace, _), stored)| {
                org == &scope.org_id && workspace == &scope.workspace_id && !stored.deleted
            })
            .map(|(_, stored)| &stored.record)
    }

    fn insert_workflow(&mut self, scope: &Scope, row: WorkflowRecord) -> Result<(), StorageError> {
        let key = workflow_key(scope, &row.id);
        if self.rows.contains_key(&key) {
            return Err(duplicate("workflow", "id"));
        }
        if self.live_in_scope(scope).any(|live| live.slug == row.slug) {
            return Err(duplicate("workflow", "slug"));
        }
        self.rows.insert(
            key,
            StoredWorkflow {
                record: row,
                deleted: false,
            },
        );
        Ok(())
    }

    /// Validate a CAS rewrite of a live row without applying it.
    fn check_update(
        &self,
        scope: &Scope,
        row: &WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let Some(current) = self.live(scope, &row.id) else {
            return Err(StorageError::not_found("workflow", row.id.clone()));
        };
        if current.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "workflow",
                id: row.id.clone(),
                expected: expected_version,
                actual: current.version,
            });
        }
        if self
            .live_in_scope(scope)
            .any(|live| live.id != row.id && live.slug == row.slug)
        {
            return Err(duplicate("workflow", "slug"));
        }
        Ok(())
    }

    /// Validate a version append without applying it: the workflow row must
    /// exist (`fk_workflow_versions__workflows`), the number and the
    /// activation identity must be free.
    fn check_version(
        &self,
        scope: &Scope,
        version: &WorkflowVersionRecord,
    ) -> Result<VersionKey, StorageError> {
        if !self
            .rows
            .contains_key(&workflow_key(scope, &version.workflow_id))
        {
            return Err(StorageError::not_found(
                "workflow",
                version.workflow_id.clone(),
            ));
        }
        let key = version_key(scope, &version.workflow_id, version.number);
        if self.versions.contains_key(&key) {
            return Err(duplicate("workflow_version", "number"));
        }
        if let Some(activation) = version.activation
            && self.versions.values().any(|existing| {
                existing.activation.is_some_and(|identity| {
                    identity.workflow_version_id() == activation.workflow_version_id()
                })
            })
        {
            return Err(duplicate("workflow_version", "activation"));
        }
        Ok(key)
    }
}

type SharedWorkflows = Arc<Mutex<WorkflowState>>;

/// `Duplicate` naming only the entity and the colliding field — never its
/// value.
fn duplicate(entity: &'static str, field: &str) -> StorageError {
    StorageError::Duplicate {
        entity,
        detail: format!("an active {entity} already has this {field}"),
    }
}

fn require_unactivated(version: &WorkflowVersionRecord) -> Result<(), StorageError> {
    if version.activation.is_some() {
        return Err(StorageError::InvalidInput(
            "activated versions require publication admission".into(),
        ));
    }
    Ok(())
}

/// In-memory workflow-row store.
///
/// Constructed **only** from its paired [`InMemoryWorkflowVersionStore`] via
/// [`Self::new_with_versions`], so both observe one state: a save writes the
/// row and its version in one critical section, and the version store reads
/// what the save wrote. Sharing is structural, not a caller discipline.
#[derive(Debug, Clone)]
pub struct InMemoryWorkflowStore {
    catalog: super::execution::SharedState,
    inner: SharedWorkflows,
}

impl InMemoryWorkflowStore {
    /// Create a workflow-row store over the state of `versions`. `execution`
    /// supplies the shared revision catalog lock so activation admission and
    /// publication linearize against drain/delete operations.
    #[must_use]
    pub fn new_with_versions(
        versions: &InMemoryWorkflowVersionStore,
        execution: &super::InMemoryExecutionStore,
    ) -> Self {
        Self {
            catalog: Arc::clone(&execution.inner),
            inner: Arc::clone(&versions.inner),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStore for InMemoryWorkflowStore {
    #[tracing::instrument(skip_all, fields(workflow_id = %row.id, expected_version), err)]
    async fn publish_activated_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: u64,
    ) -> Result<(), WorkflowPublicationError> {
        let activation = crate::workflow_activation::validate_publication(
            scope,
            &row,
            &version,
            expected_version,
        )?;
        // Fixed lock order: catalog, workflows. Drain takes catalog only.
        let catalog = self.catalog.lock();
        let mut state = self.inner.lock();
        state.check_update(scope, &row, expected_version)?;
        super::plan_flavor_catalog::require_active_pair(
            &catalog.revision_catalog,
            activation.revisions(),
        )
        .map_err(|_| WorkflowPublicationError::RevisionNotAdmitted)?;
        let plan = super::plan_flavor_catalog::load_pair(
            &catalog.revision_catalog,
            activation.revisions(),
        )
        .map_err(|_| WorkflowPublicationError::RevisionNotAdmitted)?;
        crate::workflow_activation::validate_plan_identity(plan.plan_bytes(), &row.id, activation)?;
        // A reused activation identity is a disagreeing publication.
        if state
            .versions
            .iter()
            .any(|((org, workspace, workflow, _), existing)| {
                org == &scope.org_id
                    && workspace == &scope.workspace_id
                    && workflow == &row.id
                    && existing.activation.is_some_and(|identity| {
                        identity.workflow_version_id() == activation.workflow_version_id()
                    })
            })
        {
            return Err(WorkflowPublicationError::InvalidPublication);
        }
        let key = state.check_version(scope, &version)?;
        state.versions.insert(key, version);
        if let Some(stored) = state.rows.get_mut(&workflow_key(scope, &row.id)) {
            stored.record = row;
        }
        Ok(())
    }

    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        self.inner.lock().insert_workflow(scope, record)
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        Ok(self.inner.lock().live(scope, id).cloned())
    }

    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        Ok(self
            .inner
            .lock()
            .live_in_scope(scope)
            .find(|row| row.slug == slug)
            .cloned())
    }

    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        state.check_update(scope, &record, expected_version)?;
        if let Some(stored) = state.rows.get_mut(&workflow_key(scope, &record.id)) {
            stored.record = record;
        }
        Ok(())
    }

    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: Option<u64>,
    ) -> Result<(), StorageError> {
        require_unactivated(&version)?;
        // One critical section, validate both writes before applying either
        // — no orphan-row window.
        let mut state = self.inner.lock();
        match expected_version {
            None => {
                let row_key = workflow_key(scope, &row.id);
                state.insert_workflow(scope, row)?;
                match state.check_version(scope, &version) {
                    Ok(key) => {
                        state.versions.insert(key, version);
                        Ok(())
                    },
                    Err(error) => {
                        state.rows.remove(&row_key);
                        Err(error)
                    },
                }
            },
            Some(expected) => {
                state.check_update(scope, &row, expected)?;
                let key = state.check_version(scope, &version)?;
                state.versions.insert(key, version);
                if let Some(stored) = state.rows.get_mut(&workflow_key(scope, &row.id)) {
                    stored.record = row;
                }
                Ok(())
            },
        }
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        match state
            .rows
            .get_mut(&workflow_key(scope, id))
            .filter(|stored| !stored.deleted)
        {
            Some(stored) => {
                stored.deleted = true;
                Ok(())
            },
            None => Err(StorageError::not_found("workflow", id)),
        }
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        let state = self.inner.lock();
        let mut rows: Vec<WorkflowRecord> = state.live_in_scope(scope).cloned().collect();
        // Same order as the SQL backends' `ORDER BY id`.
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn count(&self, scope: &Scope) -> Result<u64, StorageError> {
        let n = self.inner.lock().live_in_scope(scope).count();
        u64::try_from(n).map_err(|_| StorageError::Internal("workflow count overflows u64".into()))
    }

    async fn is_reachable(&self) -> Result<(), StorageError> {
        // No transport to fail — the in-memory store is always reachable.
        Ok(())
    }
}

/// In-memory workflow-version store; owns the state its paired
/// [`InMemoryWorkflowStore`] shares.
#[derive(Debug, Default, Clone)]
pub struct InMemoryWorkflowVersionStore {
    inner: SharedWorkflows,
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
        require_unactivated(&record)?;
        let mut state = self.inner.lock();
        let key = state.check_version(scope, &record)?;
        state.versions.insert(key, record);
        Ok(())
    }

    async fn get(
        &self,
        scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        Ok(self
            .inner
            .lock()
            .versions
            .get(&version_key(scope, workflow_id, number))
            .cloned())
    }

    async fn get_published(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        // Highest-numbered published version wins, matching the SQL
        // backends' `ORDER BY number DESC LIMIT 1`.
        Ok(self
            .inner
            .lock()
            .versions
            .iter()
            .filter(|((org, workspace, workflow, _), version)| {
                org == &scope.org_id
                    && workspace == &scope.workspace_id
                    && workflow == workflow_id
                    && version.published
            })
            .max_by_key(|((.., number), _)| *number)
            .map(|(_, version)| version.clone()))
    }

    async fn list(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        let mut versions: Vec<WorkflowVersionRecord> = self
            .inner
            .lock()
            .versions
            .iter()
            .filter(|((org, workspace, workflow, _), _)| {
                org == &scope.org_id && workspace == &scope.workspace_id && workflow == workflow_id
            })
            .map(|(_, version)| version.clone())
            .collect();
        // Newest first (highest version number first).
        versions.sort_by_key(|version| std::cmp::Reverse(version.number));
        Ok(versions)
    }
}
