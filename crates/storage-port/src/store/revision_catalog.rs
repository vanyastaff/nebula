//! Object-safe exact plan/flavor catalog roles.

use std::{fmt, sync::Arc};

use async_trait::async_trait;

use crate::dto::{
    BeginDrainOutcome, PlanFlavorRevisionIds, PlanFlavorRevisionRecord, PlanFlavorRevisionTarget,
    RevisionCatalogError, RevisionInsertOutcome,
};

/// Read-only exact executable-plan and worker-flavor catalog.
///
/// Exact load works for both Active and Draining revisions. It never selects
/// a latest or closest revision, follows a redirect, or recompiles a plan.
#[async_trait]
pub trait PlanFlavorCatalog: Send + Sync + fmt::Debug {
    /// Load one exact plan/flavor pair.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RevisionCatalogError`] when either exact identity is
    /// unavailable, deleted, mismatched, corrupt, or the backend cannot
    /// determine a safe outcome.
    async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
    ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError>;
}

/// Insert-only role for publishing immutable plan/flavor revisions.
///
/// Contract installation receives only this capability. It cannot begin a
/// drain, delete a revision, or mutate execution-owned references.
#[async_trait]
pub trait PlanFlavorCatalogWriter: Send + Sync + fmt::Debug {
    /// Atomically insert one immutable exact plan/flavor pair.
    ///
    /// A byte-identical retry is idempotent. Reusing an identity with
    /// different content fails closed, and a deleted identity cannot be
    /// resurrected.
    async fn insert(
        &self,
        record: &PlanFlavorRevisionRecord,
    ) -> Result<RevisionInsertOutcome, RevisionCatalogError>;
}

/// Administrative lifecycle role for immutable plan/flavor revisions.
///
/// Runtime loaders and contract installers should never receive this
/// destructive capability. This role deliberately has no insertion or
/// reference mutation method: retaining and releasing execution-owned
/// references must occur inside their owning backend transactions.
#[async_trait]
pub trait PlanFlavorCatalogAdmin: Send + Sync + fmt::Debug {
    /// Atomically change one Active revision to Draining.
    ///
    /// Repeating the operation is idempotent and returns a fresh projection
    /// of authoritative live and rollback-window references.
    async fn begin_drain(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<BeginDrainOutcome, RevisionCatalogError>;

    /// Delete one already-draining revision when no blocker remains.
    ///
    /// Worker-flavor deletion rejects any non-deleted dependent plan.
    /// Successful deletion retains an identity tombstone and clears payload
    /// bytes. Repeating deletion against that tombstone succeeds, so a caller
    /// can reconcile a commit whose acknowledgement was lost.
    ///
    /// # Reference-based retention is not yet enforced
    ///
    /// The backend rechecks reference rows in the same linearized operation
    /// *where those rows can exist*. They cannot yet: reference mutation is
    /// reachable only from an execution-owner transaction that composes
    /// reference changes with execution aggregate changes, and that
    /// transaction is not implemented. In the shipped in-memory backend the
    /// constructors are `#[cfg(test)]`, so no production caller can create a
    /// reference and [`RevisionCatalogError::Referenced`] cannot be returned
    /// from a release build.
    ///
    /// Callers must therefore **not** treat this method as a guard against
    /// deleting a revision that live executions still depend on. Until the
    /// owner transaction lands, establishing that no execution needs a
    /// revision is the caller's responsibility; `begin_drain`'s reported
    /// counts are the intended signal and are likewise zero in production.
    async fn delete_drained(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<(), RevisionCatalogError>;
}

#[async_trait]
impl<T> PlanFlavorCatalog for Arc<T>
where
    T: PlanFlavorCatalog + ?Sized,
{
    async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
    ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
        (**self).load_exact(ids).await
    }
}

#[async_trait]
impl<T> PlanFlavorCatalogWriter for Arc<T>
where
    T: PlanFlavorCatalogWriter + ?Sized,
{
    async fn insert(
        &self,
        record: &PlanFlavorRevisionRecord,
    ) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
        (**self).insert(record).await
    }
}

#[async_trait]
impl<T> PlanFlavorCatalogAdmin for Arc<T>
where
    T: PlanFlavorCatalogAdmin + ?Sized,
{
    async fn begin_drain(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<BeginDrainOutcome, RevisionCatalogError> {
        (**self).begin_drain(target).await
    }

    async fn delete_drained(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<(), RevisionCatalogError> {
        (**self).delete_drained(target).await
    }
}
