//! In-memory exact plan/flavor catalog and revision-retention reference model.
//!
//! Catalog state and execution-reference rows live under the execution
//! adapter's existing [`SharedState`] lock. That is intentional: the later
//! runtime-control start/terminal transactions must be able to compose
//! reference changes with execution aggregate changes without a second lock or
//! a split durability boundary.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, WorkerFlavorRevisionId,
};
use nebula_storage_port::{
    BeginDrainOutcome, ExecutionReferenceTransition, PlanFlavorCatalog, PlanFlavorCatalogAdmin,
    PlanFlavorCatalogWriter, PlanFlavorRevisionIds, PlanFlavorRevisionRecord,
    PlanFlavorRevisionTarget, RevisionCatalogError, RevisionInsertOutcome, RevisionReferenceCounts,
    StorageError, WorkerFlavorRevisionRecord,
};

use super::execution::{SharedState, State};
use crate::revision_catalog::{
    ArtifactLifecycle, delete_label, deleted_for, drain_label, draining_for, flavor_records_match,
    insert_label, load_label, plan_records_match, unavailable_for, validate_bounded_recorded_form,
    validate_pair_recorded_form,
};

#[derive(Debug, Clone)]
struct WorkerFlavorRow {
    lifecycle: ArtifactLifecycle,
    record: Option<WorkerFlavorRevisionRecord>,
}

#[derive(Debug, Clone)]
struct ExecutablePlanRow {
    lifecycle: ArtifactLifecycle,
    record: Option<PlanFlavorRevisionRecord>,
}

/// Private identity of the execution aggregate that owns one revision
/// reference. It deliberately has no public constructor or re-export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RevisionReferenceOwner(ExecutionId);

impl RevisionReferenceOwner {
    pub(super) const fn for_execution(execution_id: ExecutionId) -> Self {
        Self(execution_id)
    }
}

/// Private identity of an explicitly owned rollback-retention window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RollbackWindowId([u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevisionReferenceState {
    Live,
    Rollback {
        window_id: RollbackWindowId,
        retain_until: DateTime<Utc>,
    },
    Released {
        origin: ReferenceReleaseOrigin,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceReleaseOrigin {
    Live,
    Rollback {
        window_id: RollbackWindowId,
        retain_until: DateTime<Utc>,
    },
}

/// One authoritative execution reference. Counts are always derived from
/// these rows; no mutable counter exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RevisionReference {
    owner: RevisionReferenceOwner,
    bundle_id: ExecutionContractBundleId,
    ids: PlanFlavorRevisionIds,
}

impl RevisionReference {
    pub(super) const fn new(
        owner: RevisionReferenceOwner,
        bundle_id: ExecutionContractBundleId,
        ids: PlanFlavorRevisionIds,
    ) -> Self {
        Self {
            owner,
            bundle_id,
            ids,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RevisionReferenceRow {
    reference: RevisionReference,
    state: RevisionReferenceState,
}

/// Catalog/reference state embedded in the execution store's shared state.
#[derive(Debug, Default)]
pub(super) struct RevisionCatalogState {
    worker_flavors: HashMap<WorkerFlavorRevisionId, WorkerFlavorRow>,
    executable_plans: HashMap<ExecutablePlanRevisionId, ExecutablePlanRow>,
    references: HashMap<RevisionReferenceOwner, RevisionReferenceRow>,
}

pub(super) fn execution_matches_live_flavor(
    catalog: &RevisionCatalogState,
    execution_id: ExecutionId,
    flavor: WorkerFlavorRevisionId,
) -> bool {
    catalog
        .references
        .get(&RevisionReferenceOwner::for_execution(execution_id))
        .is_some_and(|row| {
            row.reference.ids.worker_flavor() == flavor
                && matches!(row.state, RevisionReferenceState::Live)
        })
}

trait RevisionClock: fmt::Debug + Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
struct SystemRevisionClock;

impl RevisionClock for SystemRevisionClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// In-memory implementation of the technical exact plan/flavor catalog.
///
/// Construct it through [`super::InMemoryExecutionStore::plan_flavor_catalog`]
/// so catalog records and future execution-owned reference mutations share the
/// execution store's single atomicity boundary.
#[derive(Clone)]
pub struct InMemoryPlanFlavorCatalog {
    inner: SharedState,
    clock: Arc<dyn RevisionClock>,
}

impl fmt::Debug for InMemoryPlanFlavorCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InMemoryPlanFlavorCatalog")
            .finish_non_exhaustive()
    }
}

impl InMemoryPlanFlavorCatalog {
    /// Build a catalog over an execution store's existing shared lock.
    ///
    /// Sharing this core is required so later execution-owner transactions can
    /// compose execution state and revision-reference changes atomically.
    #[must_use]
    pub fn new(execution_store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: execution_store.shared(),
            clock: Arc::new(SystemRevisionClock),
        }
    }

    #[cfg(test)]
    fn with_clock(inner: SharedState, clock: Arc<dyn RevisionClock>) -> Self {
        Self { inner, clock }
    }
}

fn reference_counts(
    catalog: &RevisionCatalogState,
    target: PlanFlavorRevisionTarget,
    now: DateTime<Utc>,
) -> RevisionReferenceCounts {
    let mut live_executions = 0_u64;
    let mut rollback_windows = 0_u64;

    for row in catalog
        .references
        .values()
        .filter(|row| target_matches_ids(target, row.reference.ids))
    {
        match row.state {
            RevisionReferenceState::Live => {
                live_executions = live_executions.saturating_add(1);
            },
            RevisionReferenceState::Rollback { retain_until, .. } if now < retain_until => {
                rollback_windows = rollback_windows.saturating_add(1);
            },
            RevisionReferenceState::Rollback { .. } | RevisionReferenceState::Released { .. } => {},
        }
    }

    RevisionReferenceCounts::new(live_executions, rollback_windows)
}

fn target_matches_ids(target: PlanFlavorRevisionTarget, ids: PlanFlavorRevisionIds) -> bool {
    match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => plan_id == ids.plan(),
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => {
            worker_flavor_id == ids.worker_flavor()
        },
    }
}

fn insert_pair(
    catalog: &mut RevisionCatalogState,
    record: &PlanFlavorRevisionRecord,
) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
    validate_pair_recorded_form(record)?;

    let ids = record.ids();
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(ids.plan());
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor());

    let existing_plan_matches = match catalog.executable_plans.get(&ids.plan()) {
        Some(ExecutablePlanRow {
            lifecycle: ArtifactLifecycle::Deleted,
            ..
        }) => return Err(deleted_for(plan_target)),
        Some(ExecutablePlanRow {
            record: Some(stored),
            ..
        }) if plan_records_match(stored, record) => true,
        Some(ExecutablePlanRow {
            record: Some(_), ..
        }) => {
            return Err(RevisionCatalogError::ContentConflict {
                target: plan_target,
            });
        },
        Some(ExecutablePlanRow { record: None, .. }) => {
            return Err(RevisionCatalogError::CorruptRecord {
                target: plan_target,
            });
        },
        None => false,
    };

    let existing_flavor_matches = match catalog.worker_flavors.get(&ids.worker_flavor()) {
        Some(WorkerFlavorRow {
            lifecycle: ArtifactLifecycle::Deleted,
            ..
        }) => return Err(deleted_for(flavor_target)),
        Some(WorkerFlavorRow {
            record: Some(stored),
            ..
        }) if flavor_records_match(stored, record.worker_flavor()) => true,
        Some(WorkerFlavorRow {
            record: Some(_), ..
        }) => {
            return Err(RevisionCatalogError::ContentConflict {
                target: flavor_target,
            });
        },
        Some(WorkerFlavorRow { record: None, .. }) => {
            return Err(RevisionCatalogError::CorruptRecord {
                target: flavor_target,
            });
        },
        None => false,
    };

    if existing_plan_matches && !existing_flavor_matches {
        return Err(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        });
    }

    // Draining is a property of the stored artifact, not of whether this
    // caller happens to have inserted this plan before. Gating the check on
    // the new-plan path made an idempotent retry return `AlreadyPresent` for a
    // revision that is being retired — so whether an installer learned the
    // truth depended on its own history rather than on the catalog. The plan's
    // own lifecycle was never consulted at all, leaving `Draining` on an
    // executable plan unreportable through this entry point.
    if matches!(
        catalog
            .executable_plans
            .get(&ids.plan())
            .map(|row| row.lifecycle),
        Some(ArtifactLifecycle::Draining)
    ) {
        return Err(draining_for(plan_target));
    }

    if matches!(
        catalog
            .worker_flavors
            .get(&ids.worker_flavor())
            .map(|row| row.lifecycle),
        Some(ArtifactLifecycle::Draining)
    ) {
        return Err(draining_for(flavor_target));
    }

    if existing_plan_matches && existing_flavor_matches {
        return Ok(RevisionInsertOutcome::AlreadyPresent);
    }

    if !existing_flavor_matches {
        catalog.worker_flavors.insert(
            ids.worker_flavor(),
            WorkerFlavorRow {
                lifecycle: ArtifactLifecycle::Active,
                record: Some(record.worker_flavor().clone()),
            },
        );
    }
    if !existing_plan_matches {
        catalog.executable_plans.insert(
            ids.plan(),
            ExecutablePlanRow {
                lifecycle: ArtifactLifecycle::Active,
                record: Some(record.clone()),
            },
        );
    }

    Ok(RevisionInsertOutcome::Inserted)
}

pub(super) fn load_pair(
    catalog: &RevisionCatalogState,
    ids: PlanFlavorRevisionIds,
) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(ids.plan());
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor());

    let plan_row = catalog
        .executable_plans
        .get(&ids.plan())
        .ok_or_else(|| unavailable_for(plan_target))?;
    if plan_row.lifecycle == ArtifactLifecycle::Deleted {
        return Err(deleted_for(plan_target));
    }
    let record = plan_row
        .record
        .as_ref()
        .ok_or(RevisionCatalogError::CorruptRecord {
            target: plan_target,
        })?;
    validate_bounded_recorded_form(record.plan_record_bytes(), plan_target)?;
    if record.ids().worker_flavor() != ids.worker_flavor() {
        return Err(RevisionCatalogError::PlanFlavorMismatch {
            requested: ids,
            stored_worker_flavor_id: record.ids().worker_flavor(),
        });
    }

    let flavor_row = catalog
        .worker_flavors
        .get(&ids.worker_flavor())
        .ok_or_else(|| unavailable_for(flavor_target))?;
    if flavor_row.lifecycle == ArtifactLifecycle::Deleted {
        return Err(deleted_for(flavor_target));
    }
    let flavor_record = flavor_row
        .record
        .as_ref()
        .ok_or(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        })?;
    validate_bounded_recorded_form(flavor_record.record_bytes(), flavor_target)?;
    if !flavor_records_match(flavor_record, record.worker_flavor()) {
        return Err(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        });
    }

    Ok(record.clone())
}

fn begin_drain_locked(
    catalog: &mut RevisionCatalogState,
    target: PlanFlavorRevisionTarget,
    now: DateTime<Utc>,
) -> Result<BeginDrainOutcome, RevisionCatalogError> {
    let lifecycle = match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => catalog
            .executable_plans
            .get(&plan_id)
            .map(|row| row.lifecycle),
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => catalog
            .worker_flavors
            .get(&worker_flavor_id)
            .map(|row| row.lifecycle),
    }
    .ok_or_else(|| unavailable_for(target))?;

    match lifecycle {
        ArtifactLifecycle::Active => {
            match target {
                PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => {
                    let row = catalog
                        .executable_plans
                        .get_mut(&plan_id)
                        .ok_or_else(|| unavailable_for(target))?;
                    row.lifecycle = ArtifactLifecycle::Draining;
                },
                PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => {
                    let row = catalog
                        .worker_flavors
                        .get_mut(&worker_flavor_id)
                        .ok_or_else(|| unavailable_for(target))?;
                    row.lifecycle = ArtifactLifecycle::Draining;
                },
            }
            Ok(BeginDrainOutcome::Started(reference_counts(
                catalog, target, now,
            )))
        },
        ArtifactLifecycle::Draining => Ok(BeginDrainOutcome::AlreadyDraining(reference_counts(
            catalog, target, now,
        ))),
        ArtifactLifecycle::Deleted => Err(deleted_for(target)),
    }
}

fn delete_drained_locked(
    catalog: &mut RevisionCatalogState,
    target: PlanFlavorRevisionTarget,
    now: DateTime<Utc>,
) -> Result<(), RevisionCatalogError> {
    let lifecycle = match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => catalog
            .executable_plans
            .get(&plan_id)
            .map(|row| row.lifecycle),
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => catalog
            .worker_flavors
            .get(&worker_flavor_id)
            .map(|row| row.lifecycle),
    }
    .ok_or_else(|| unavailable_for(target))?;

    match lifecycle {
        ArtifactLifecycle::Active => {
            return Err(RevisionCatalogError::DrainRequired { target });
        },
        ArtifactLifecycle::Deleted => {
            let payload_is_cleared = match target {
                PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => catalog
                    .executable_plans
                    .get(&plan_id)
                    .is_some_and(|row| row.record.is_none()),
                PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => catalog
                    .worker_flavors
                    .get(&worker_flavor_id)
                    .is_some_and(|row| row.record.is_none()),
            };
            return if payload_is_cleared {
                Ok(())
            } else {
                Err(RevisionCatalogError::CorruptRecord { target })
            };
        },
        ArtifactLifecycle::Draining => {},
    }

    let references = reference_counts(catalog, target, now);
    if !references.is_empty() {
        return Err(RevisionCatalogError::Referenced { target, references });
    }

    match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => {
            let row = catalog
                .executable_plans
                .get_mut(&plan_id)
                .ok_or_else(|| unavailable_for(target))?;
            row.lifecycle = ArtifactLifecycle::Deleted;
            row.record = None;
        },
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => {
            let mut dependent_plans = 0_u64;
            for (plan_id, row) in &catalog.executable_plans {
                if row.lifecycle == ArtifactLifecycle::Deleted {
                    continue;
                }
                let Some(record) = row.record.as_ref() else {
                    return Err(RevisionCatalogError::CorruptRecord {
                        target: PlanFlavorRevisionTarget::ExecutablePlan(*plan_id),
                    });
                };
                if record.ids().worker_flavor() == worker_flavor_id {
                    dependent_plans = dependent_plans.saturating_add(1);
                }
            }
            if dependent_plans != 0 {
                return Err(RevisionCatalogError::DependentPlans {
                    worker_flavor_id,
                    dependent_plans,
                });
            }
            let row = catalog
                .worker_flavors
                .get_mut(&worker_flavor_id)
                .ok_or_else(|| unavailable_for(target))?;
            row.lifecycle = ArtifactLifecycle::Deleted;
            row.record = None;
        },
    }
    Ok(())
}

#[async_trait::async_trait]
impl PlanFlavorCatalog for InMemoryPlanFlavorCatalog {
    async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
    ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
        let result = {
            let state = self.inner.lock();
            load_pair(&state.revision_catalog, ids)
        };
        tracing::debug!(
            target: "nebula_storage::inmem",
            plan_revision_id = %ids.plan(),
            worker_flavor_revision_id = %ids.worker_flavor(),
            outcome = load_label(&result),
            "exact plan/flavor catalog load"
        );
        result
    }
}

#[async_trait::async_trait]
impl PlanFlavorCatalogWriter for InMemoryPlanFlavorCatalog {
    async fn insert(
        &self,
        record: &PlanFlavorRevisionRecord,
    ) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
        let ids = record.ids();
        let result = {
            let mut state = self.inner.lock();
            insert_pair(&mut state.revision_catalog, record)
        };
        tracing::debug!(
            target: "nebula_storage::inmem",
            plan_revision_id = %ids.plan(),
            worker_flavor_revision_id = %ids.worker_flavor(),
            outcome = insert_label(&result),
            "plan/flavor catalog insert"
        );
        result
    }
}

#[async_trait::async_trait]
impl PlanFlavorCatalogAdmin for InMemoryPlanFlavorCatalog {
    async fn begin_drain(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<BeginDrainOutcome, RevisionCatalogError> {
        let result = {
            let mut state = self.inner.lock();
            let now = self.clock.now();
            begin_drain_locked(&mut state.revision_catalog, target, now)
        };
        tracing::debug!(
            target: "nebula_storage::inmem",
            target = ?target,
            outcome = drain_label(&result),
            "plan/flavor catalog begin drain"
        );
        result
    }

    async fn delete_drained(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<(), RevisionCatalogError> {
        let result = {
            let mut state = self.inner.lock();
            let now = self.clock.now();
            delete_drained_locked(&mut state.revision_catalog, target, now)
        };
        tracing::debug!(
            target: "nebula_storage::inmem",
            target = ?target,
            outcome = delete_label(&result),
            "plan/flavor catalog guarded delete"
        );
        result
    }

    async fn release_expired_rollbacks(&self, limit: u64) -> Result<u64, RevisionCatalogError> {
        let now = self.clock.now();
        let mut state = self.inner.lock();
        let expired: Vec<RevisionReferenceOwner> = state
            .revision_catalog
            .references
            .iter()
            .filter(|(_, row)| {
                matches!(
                    row.state,
                    RevisionReferenceState::Rollback { retain_until, .. } if retain_until <= now
                )
            })
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|(owner, _)| *owner)
            .collect();

        let mut released = 0_u64;
        for owner in expired {
            let (reference, window_id, retain_until) =
                match state.revision_catalog.references.get(&owner) {
                    Some(row) => match row.state {
                        RevisionReferenceState::Rollback {
                            window_id,
                            retain_until,
                        } => (row.reference, window_id, retain_until),
                        _ => continue,
                    },
                    None => continue,
                };
            match transition_reference_locked(
                &mut state,
                OwningReferenceTransition::ReleaseRollback {
                    reference,
                    window_id,
                    retain_until,
                },
            ) {
                Ok(ReferenceDecision::Applied | ReferenceDecision::AlreadyApplied) => {
                    released = released.saturating_add(1);
                },
                Err(_) => return Err(RevisionCatalogError::Unavailable),
            }
        }
        Ok(released)
    }
}

/// Result of creating the private execution-owned reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainDecision {
    Retained,
    AlreadyRetained,
}

/// Result of a private owning reference transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReferenceDecision {
    Applied,
    AlreadyApplied,
}

/// Private execution-owner transition. This never crosses the storage-port or
/// SDK boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwningReferenceTransition {
    ReleaseLive {
        reference: RevisionReference,
    },
    RetainForRollback {
        reference: RevisionReference,
        window_id: RollbackWindowId,
        retain_until: DateTime<Utc>,
    },
    ReleaseRollback {
        reference: RevisionReference,
        window_id: RollbackWindowId,
        retain_until: DateTime<Utc>,
    },
}

/// Closed failures for backend-private reference fragments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum InternalRevisionError {
    #[error("executable plan revision is unavailable")]
    PlanUnavailable,
    #[error("worker flavor revision is unavailable")]
    WorkerFlavorUnavailable,
    #[error("exact pair exists but is not admissible")]
    PairNotAdmitted,
    #[error("revision is draining")]
    Draining,
    #[error("revision has been deleted")]
    Deleted,
    #[error("reference owner is already bound to different immutable pins")]
    ReferenceMismatch,
    #[error("reference owner has already closed its reference")]
    ReferenceClosed,
    #[error("reference owner does not exist")]
    ReferenceUnavailable,
}

pub(super) fn require_active_pair(
    catalog: &RevisionCatalogState,
    ids: PlanFlavorRevisionIds,
) -> Result<(), InternalRevisionError> {
    let plan = catalog
        .executable_plans
        .get(&ids.plan())
        .ok_or(InternalRevisionError::PlanUnavailable)?;
    match plan.lifecycle {
        ArtifactLifecycle::Active => {},
        ArtifactLifecycle::Draining => return Err(InternalRevisionError::Draining),
        ArtifactLifecycle::Deleted => return Err(InternalRevisionError::Deleted),
    }
    let Some(plan_record) = plan.record.as_ref() else {
        return Err(InternalRevisionError::PlanUnavailable);
    };
    if plan_record.ids().worker_flavor() != ids.worker_flavor() {
        return Err(InternalRevisionError::PairNotAdmitted);
    }

    let flavor = catalog
        .worker_flavors
        .get(&ids.worker_flavor())
        .ok_or(InternalRevisionError::WorkerFlavorUnavailable)?;
    match flavor.lifecycle {
        ArtifactLifecycle::Active => Ok(()),
        ArtifactLifecycle::Draining => Err(InternalRevisionError::Draining),
        ArtifactLifecycle::Deleted => Err(InternalRevisionError::Deleted),
    }
}

/// Create a live reference while the caller holds the owning aggregate's
/// shared backend transaction/lock.
///
/// Existing byte-for-byte pins for the same owner are idempotent even if a
/// drain started after the original commit. A different binding never changes
/// the authoritative row.
pub(super) fn retain_exact_locked(
    state: &mut State,
    reference: RevisionReference,
) -> Result<RetainDecision, InternalRevisionError> {
    if let Some(existing) = state.revision_catalog.references.get(&reference.owner) {
        if existing.reference != reference {
            return Err(InternalRevisionError::ReferenceMismatch);
        }
        return match existing.state {
            RevisionReferenceState::Live => Ok(RetainDecision::AlreadyRetained),
            RevisionReferenceState::Rollback { .. } | RevisionReferenceState::Released { .. } => {
                Err(InternalRevisionError::ReferenceClosed)
            },
        };
    }

    require_active_pair(&state.revision_catalog, reference.ids)?;
    state.revision_catalog.references.insert(
        reference.owner,
        RevisionReferenceRow {
            reference,
            state: RevisionReferenceState::Live,
        },
    );
    Ok(RetainDecision::Retained)
}

/// Transition a reference while the caller holds the owning aggregate's
/// shared backend transaction/lock.
pub(super) fn transition_reference_locked(
    state: &mut State,
    transition: OwningReferenceTransition,
) -> Result<ReferenceDecision, InternalRevisionError> {
    match transition {
        OwningReferenceTransition::ReleaseLive { reference } => {
            let row = state
                .revision_catalog
                .references
                .get_mut(&reference.owner)
                .ok_or(InternalRevisionError::ReferenceUnavailable)?;
            if row.reference != reference {
                return Err(InternalRevisionError::ReferenceMismatch);
            }
            match row.state {
                RevisionReferenceState::Live => {
                    row.state = RevisionReferenceState::Released {
                        origin: ReferenceReleaseOrigin::Live,
                    };
                    Ok(ReferenceDecision::Applied)
                },
                RevisionReferenceState::Released {
                    origin: ReferenceReleaseOrigin::Live,
                } => Ok(ReferenceDecision::AlreadyApplied),
                RevisionReferenceState::Rollback { .. }
                | RevisionReferenceState::Released {
                    origin: ReferenceReleaseOrigin::Rollback { .. },
                } => Err(InternalRevisionError::ReferenceMismatch),
            }
        },
        OwningReferenceTransition::RetainForRollback {
            reference,
            window_id,
            retain_until,
        } => {
            let row = state
                .revision_catalog
                .references
                .get_mut(&reference.owner)
                .ok_or(InternalRevisionError::ReferenceUnavailable)?;
            if row.reference != reference {
                return Err(InternalRevisionError::ReferenceMismatch);
            }
            match row.state {
                RevisionReferenceState::Live => {
                    row.state = RevisionReferenceState::Rollback {
                        window_id,
                        retain_until,
                    };
                    Ok(ReferenceDecision::Applied)
                },
                RevisionReferenceState::Rollback {
                    window_id: existing_window_id,
                    retain_until: existing_retain_until,
                } if existing_window_id == window_id && existing_retain_until == retain_until => {
                    Ok(ReferenceDecision::AlreadyApplied)
                },
                RevisionReferenceState::Rollback { .. }
                | RevisionReferenceState::Released { .. } => {
                    Err(InternalRevisionError::ReferenceMismatch)
                },
            }
        },
        OwningReferenceTransition::ReleaseRollback {
            reference,
            window_id,
            retain_until,
        } => {
            let row = state
                .revision_catalog
                .references
                .get_mut(&reference.owner)
                .ok_or(InternalRevisionError::ReferenceUnavailable)?;
            if row.reference != reference {
                return Err(InternalRevisionError::ReferenceMismatch);
            }
            match row.state {
                RevisionReferenceState::Rollback {
                    window_id: existing_window_id,
                    retain_until: existing_retain_until,
                } if existing_window_id == window_id && existing_retain_until == retain_until => {
                    row.state = RevisionReferenceState::Released {
                        origin: ReferenceReleaseOrigin::Rollback {
                            window_id,
                            retain_until,
                        },
                    };
                    Ok(ReferenceDecision::Applied)
                },
                RevisionReferenceState::Released {
                    origin:
                        ReferenceReleaseOrigin::Rollback {
                            window_id: existing_window_id,
                            retain_until: existing_retain_until,
                        },
                } if existing_window_id == window_id && existing_retain_until == retain_until => {
                    Ok(ReferenceDecision::AlreadyApplied)
                },
                RevisionReferenceState::Live
                | RevisionReferenceState::Rollback { .. }
                | RevisionReferenceState::Released { .. } => {
                    Err(InternalRevisionError::ReferenceMismatch)
                },
            }
        },
    }
}

/// Apply one execution-owned reference transition while the caller holds the
/// aggregate lock.
///
/// The reference row is keyed by execution id, so the caller only supplies the
/// terminal transition; the exact bundle/plan/flavor pins are read from the
/// authoritative row. Missing rows (legacy executions that predate
/// materialized starts) and already-applied transitions are idempotent.
pub(super) fn apply_reference_transition_locked(
    state: &mut State,
    execution_id: ExecutionId,
    transition: ExecutionReferenceTransition,
) -> Result<(), StorageError> {
    let owner = RevisionReferenceOwner::for_execution(execution_id);
    let Some(row) = state.revision_catalog.references.get(&owner) else {
        return Ok(());
    };
    let reference = row.reference;
    let owning = match transition {
        ExecutionReferenceTransition::ReleaseLive => {
            OwningReferenceTransition::ReleaseLive { reference }
        },
        ExecutionReferenceTransition::RetainRollback {
            window_id,
            retain_until,
        } => OwningReferenceTransition::RetainForRollback {
            reference,
            window_id: RollbackWindowId(window_id),
            retain_until,
        },
    };

    match transition_reference_locked(state, owning) {
        Ok(ReferenceDecision::Applied | ReferenceDecision::AlreadyApplied) => Ok(()),
        Err(InternalRevisionError::ReferenceUnavailable) => Ok(()),
        Err(other) => Err(StorageError::Internal(format!(
            "terminal dereference failed: {other}"
        ))),
    }
}

#[cfg(test)]
#[path = "plan_flavor_catalog_tests.rs"]
mod tests;
