//! Exact executable-plan and worker-flavor catalog values.

use std::fmt;

use crate::ids::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};

/// Exact executable-plan and worker-flavor revision pair.
///
/// The pair carries typed identifiers so a plan identifier cannot be
/// accidentally substituted for a worker-flavor identifier at a storage
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanFlavorRevisionIds {
    plan: ExecutablePlanRevisionId,
    worker_flavor: WorkerFlavorRevisionId,
}

impl PlanFlavorRevisionIds {
    /// Construct one exact plan/flavor revision pair.
    pub const fn new(
        plan: ExecutablePlanRevisionId,
        worker_flavor: WorkerFlavorRevisionId,
    ) -> Self {
        Self {
            plan,
            worker_flavor,
        }
    }

    /// Return the exact executable-plan revision identifier.
    pub const fn plan(self) -> ExecutablePlanRevisionId {
        self.plan
    }

    /// Return the exact worker-flavor revision identifier.
    pub const fn worker_flavor(self) -> WorkerFlavorRevisionId {
        self.worker_flavor
    }
}

/// Non-empty opaque serialized revision record.
///
/// Storage does not interpret these bytes. The owning plugin layer decodes
/// and verifies the versioned record after an exact load.
#[must_use]
#[derive(Clone, PartialEq, Eq)]
pub struct RevisionRecordBytes(Box<[u8]>);

impl RevisionRecordBytes {
    /// Convert a non-empty byte vector into an opaque revision record.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionCatalogError::EmptyRecord`] when `bytes` is empty.
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, RevisionCatalogError> {
        if bytes.is_empty() {
            return Err(RevisionCatalogError::EmptyRecord);
        }

        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Borrow the serialized record bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume this value and return its serialized record bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.0.into_vec()
    }
}

impl fmt::Debug for RevisionRecordBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevisionRecordBytes")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Supported durable executable-plan record encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExecutablePlanRecordFormat {
    /// Canonical Graph-v1 JSON recorded form.
    GraphV1Json,
}

/// Supported durable worker-flavor record encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WorkerFlavorRecordFormat {
    /// Version-one JSON recorded form.
    V1Json,
}

/// Opaque durable worker-flavor revision record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFlavorRevisionRecord {
    id: WorkerFlavorRevisionId,
    format: WorkerFlavorRecordFormat,
    bytes: RevisionRecordBytes,
}

impl WorkerFlavorRevisionRecord {
    /// Construct a version-one JSON worker-flavor record.
    pub const fn v1_json(id: WorkerFlavorRevisionId, bytes: RevisionRecordBytes) -> Self {
        Self {
            id,
            format: WorkerFlavorRecordFormat::V1Json,
            bytes,
        }
    }

    /// Return the claimed worker-flavor revision identifier.
    pub const fn id(&self) -> WorkerFlavorRevisionId {
        self.id
    }

    /// Return the durable record encoding.
    pub const fn format(&self) -> WorkerFlavorRecordFormat {
        self.format
    }

    /// Borrow the opaque serialized record.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }
}

/// One atomically persisted exact executable-plan and worker-flavor pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFlavorRevisionRecord {
    ids: PlanFlavorRevisionIds,
    plan_format: ExecutablePlanRecordFormat,
    plan_bytes: RevisionRecordBytes,
    worker_flavor: WorkerFlavorRevisionRecord,
}

impl PlanFlavorRevisionRecord {
    /// Construct a Graph-v1 plan paired with its exact worker-flavor record.
    ///
    /// The worker-flavor identifier is derived from `worker_flavor`; callers
    /// cannot supply a second, conflicting flavor identifier.
    pub const fn graph_v1_json(
        plan_id: ExecutablePlanRevisionId,
        plan_bytes: RevisionRecordBytes,
        worker_flavor: WorkerFlavorRevisionRecord,
    ) -> Self {
        Self {
            ids: PlanFlavorRevisionIds::new(plan_id, worker_flavor.id()),
            plan_format: ExecutablePlanRecordFormat::GraphV1Json,
            plan_bytes,
            worker_flavor,
        }
    }

    /// Return the exact typed revision pair.
    pub const fn ids(&self) -> PlanFlavorRevisionIds {
        self.ids
    }

    /// Return the durable executable-plan record encoding.
    pub const fn plan_format(&self) -> ExecutablePlanRecordFormat {
        self.plan_format
    }

    /// Borrow the opaque serialized executable-plan record.
    pub fn plan_bytes(&self) -> &[u8] {
        self.plan_bytes.as_bytes()
    }

    /// Borrow the exact worker-flavor record paired with the plan.
    pub const fn worker_flavor(&self) -> &WorkerFlavorRevisionRecord {
        &self.worker_flavor
    }
}

/// Catalog lifecycle target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanFlavorRevisionTarget {
    /// One immutable executable-plan revision.
    ExecutablePlan(ExecutablePlanRevisionId),
    /// One immutable worker-flavor revision.
    WorkerFlavor(WorkerFlavorRevisionId),
}

/// Authoritative blockers derived from revision-reference rows.
///
/// These values are projections, never independently persisted mutable
/// counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevisionReferenceCounts {
    live_executions: u64,
    rollback_windows: u64,
}

impl RevisionReferenceCounts {
    /// Construct reference counts derived from authoritative rows.
    pub const fn new(live_executions: u64, rollback_windows: u64) -> Self {
        Self {
            live_executions,
            rollback_windows,
        }
    }

    /// Return the number of live execution references.
    pub const fn live_executions(self) -> u64 {
        self.live_executions
    }

    /// Return the number of unexpired rollback-window references.
    pub const fn rollback_windows(self) -> u64 {
        self.rollback_windows
    }

    /// Return whether no authoritative reference blocks deletion.
    pub const fn is_empty(self) -> bool {
        self.live_executions == 0 && self.rollback_windows == 0
    }
}

/// Result of atomically inserting one exact plan/flavor pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevisionInsertOutcome {
    /// The pair was absent and is now stored as Active.
    Inserted,
    /// Byte-identical records with the same identifiers already existed.
    AlreadyPresent,
}

/// Result of beginning one revision's drain lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BeginDrainOutcome {
    /// Active changed to Draining with the observed reference projection.
    Started(RevisionReferenceCounts),
    /// The revision was already Draining.
    AlreadyDraining(RevisionReferenceCounts),
}

/// Closed, payload-redacted exact revision catalog failure.
///
/// Variants contain only typed revision identifiers and bounded counts.
/// Serialized records, driver messages, SQL, tenant data, and authority
/// proofs never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RevisionCatalogError {
    /// The exact executable-plan revision does not exist.
    #[error("executable-plan revision is unavailable")]
    PlanUnavailable {
        /// Requested executable-plan revision.
        plan_id: ExecutablePlanRevisionId,
    },

    /// The exact worker-flavor revision does not exist.
    #[error("worker-flavor revision is unavailable")]
    WorkerFlavorUnavailable {
        /// Requested worker-flavor revision.
        worker_flavor_id: WorkerFlavorRevisionId,
    },

    /// The stored plan is pinned to a different worker flavor.
    #[error("executable-plan and worker-flavor revisions do not match")]
    PlanFlavorMismatch {
        /// Exact pair requested by the caller.
        requested: PlanFlavorRevisionIds,
        /// Worker flavor pinned by the stored plan.
        stored_worker_flavor_id: WorkerFlavorRevisionId,
    },

    /// An immutable identifier already contains different record bytes.
    #[error("revision identity already contains different content")]
    ContentConflict {
        /// Immutable identity whose content differs.
        target: PlanFlavorRevisionTarget,
    },

    /// A new reference lost a race with revision drain.
    #[error("revision is draining and rejects new references")]
    Draining {
        /// Draining revision.
        target: PlanFlavorRevisionTarget,
    },

    /// The immutable identity has a deletion tombstone.
    #[error("revision has been deleted")]
    Deleted {
        /// Deleted revision.
        target: PlanFlavorRevisionTarget,
    },

    /// Deletion was requested before the revision began draining.
    #[error("revision must begin draining before deletion")]
    DrainRequired {
        /// Active revision.
        target: PlanFlavorRevisionTarget,
    },

    /// Live executions or rollback windows still retain the revision.
    ///
    /// Not yet reachable in a release build: reference rows can only be
    /// created by the execution-owner transaction, which is unimplemented, so
    /// the shipped backend's reference constructors are `#[cfg(test)]`. Do not
    /// rely on its absence as evidence that a revision is unreferenced — see
    /// [`PlanFlavorCatalogAdmin::delete_drained`](crate::PlanFlavorCatalogAdmin::delete_drained).
    #[error("revision remains referenced")]
    Referenced {
        /// Revision whose deletion was rejected.
        target: PlanFlavorRevisionTarget,
        /// Authoritative reference projection observed by the operation.
        references: RevisionReferenceCounts,
    },

    /// Stored plans still depend on the worker flavor.
    #[error("worker-flavor revision still has dependent executable plans")]
    DependentPlans {
        /// Worker flavor whose deletion was rejected.
        worker_flavor_id: WorkerFlavorRevisionId,
        /// Number of non-deleted stored plans pinned to the worker flavor.
        dependent_plans: u64,
    },

    /// An opaque durable record was empty.
    #[error("revision record is empty")]
    EmptyRecord,

    /// Persisted format metadata names an unsupported recorded form.
    #[error("revision record format is unsupported")]
    UnsupportedRecordFormat {
        /// Revision whose format metadata is unsupported.
        target: PlanFlavorRevisionTarget,
    },

    /// Persisted record bytes violate their recorded-form contract.
    #[error("revision record is corrupt")]
    CorruptRecord {
        /// Revision whose record is corrupt.
        target: PlanFlavorRevisionTarget,
    },

    /// The operation definitely did not commit.
    #[error("revision catalog is unavailable")]
    Unavailable,

    /// Commit was dispatched but authoritative acknowledgement was lost.
    #[error("revision catalog outcome is unknown; do not retry blindly")]
    OutcomeUnknown,
}
