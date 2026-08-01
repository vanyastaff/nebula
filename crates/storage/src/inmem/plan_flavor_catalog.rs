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
    BeginDrainOutcome, PlanFlavorCatalog, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter,
    PlanFlavorRevisionIds, PlanFlavorRevisionRecord, PlanFlavorRevisionTarget,
    RevisionCatalogError, RevisionInsertOutcome, RevisionReferenceCounts,
    WorkerFlavorRevisionRecord,
};

use super::execution::SharedState;
#[cfg(test)]
use super::execution::State;
use crate::revision_catalog::{
    ArtifactLifecycle, delete_label, deleted_for, drain_label, draining_for, flavor_records_match,
    insert_label, load_label, plan_records_match, unavailable_for, validate_pair_recorded_form,
    validate_recorded_form,
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
    #[cfg(test)]
    const fn for_execution(execution_id: ExecutionId) -> Self {
        Self(execution_id)
    }
}

/// Private identity of an explicitly owned rollback-retention window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RollbackWindowId([u8; 16]);

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "reference mutation remains syntactically closed until an execution-owner transaction composes it"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "release provenance remains dormant with the execution-owner reference transaction"
    )
)]
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
    #[cfg(test)]
    const fn new(
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

fn load_pair(
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
    validate_recorded_form(record.plan_bytes(), plan_target)?;
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
    validate_recorded_form(flavor_record.bytes(), flavor_target)?;
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
}

/// Result of creating the private execution-owned reference.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainDecision {
    Retained,
    AlreadyRetained,
}

/// Result of a private owning reference transition.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReferenceDecision {
    Applied,
    AlreadyApplied,
}

/// Private execution-owner transition. This never crosses the storage-port or
/// SDK boundary.
#[cfg(test)]
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
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum InternalRevisionError {
    #[error("executable plan revision is unavailable")]
    PlanUnavailable,
    #[error("worker flavor revision is unavailable")]
    WorkerFlavorUnavailable,
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

#[cfg(test)]
fn require_active_pair(
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
        return Err(InternalRevisionError::WorkerFlavorUnavailable);
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
#[cfg(test)]
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
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use chrono::TimeDelta;
    use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
    use nebula_storage_port::RevisionRecordBytes;
    use tokio::sync::Barrier;

    use super::*;
    use crate::inmem::InMemoryExecutionStore;

    #[derive(Debug)]
    struct FixedRevisionClock {
        epoch_millis: AtomicI64,
    }

    impl FixedRevisionClock {
        fn new(now: DateTime<Utc>) -> Self {
            Self {
                epoch_millis: AtomicI64::new(now.timestamp_millis()),
            }
        }

        fn set(&self, now: DateTime<Utc>) {
            self.epoch_millis
                .store(now.timestamp_millis(), Ordering::SeqCst);
        }
    }

    impl RevisionClock for FixedRevisionClock {
        fn now(&self) -> DateTime<Utc> {
            DateTime::from_timestamp_millis(self.epoch_millis.load(Ordering::SeqCst))
                .unwrap_or_default()
        }
    }

    #[derive(Clone)]
    struct RevisionCatalogTestDriver {
        inner: SharedState,
    }

    impl RevisionCatalogTestDriver {
        fn retain(
            &self,
            reference: RevisionReference,
        ) -> Result<RetainDecision, InternalRevisionError> {
            retain_exact_locked(&mut self.inner.lock(), reference)
        }

        fn transition(
            &self,
            transition: OwningReferenceTransition,
        ) -> Result<ReferenceDecision, InternalRevisionError> {
            transition_reference_locked(&mut self.inner.lock(), transition)
        }

        fn reference_rows(&self) -> usize {
            self.inner.lock().revision_catalog.references.len()
        }

        fn remove_worker_flavor_record(&self, worker_flavor_id: WorkerFlavorRevisionId) {
            self.inner
                .lock()
                .revision_catalog
                .worker_flavors
                .remove(&worker_flavor_id);
        }
    }

    struct Fixture {
        catalog: InMemoryPlanFlavorCatalog,
        driver: RevisionCatalogTestDriver,
        clock: Arc<FixedRevisionClock>,
        record: PlanFlavorRevisionRecord,
        ids: PlanFlavorRevisionIds,
    }

    impl Fixture {
        fn new() -> Self {
            let execution_store = InMemoryExecutionStore::new();
            let inner = execution_store.shared();
            let now = DateTime::from_timestamp(1_900_000_000, 0).unwrap_or_default();
            let clock = Arc::new(FixedRevisionClock::new(now));
            let catalog = InMemoryPlanFlavorCatalog::with_clock(Arc::clone(&inner), clock.clone());
            let plan_id = ExecutablePlanRevisionId::from_bytes([0x31; 32]);
            let worker_flavor_id = WorkerFlavorRevisionId::from_bytes([0x41; 32]);
            let worker_flavor = WorkerFlavorRevisionRecord::v1_json(
                worker_flavor_id,
                RevisionRecordBytes::try_from_vec(br#"{"flavor":"v1"}"#.to_vec())
                    .expect("fixture flavor bytes are non-empty"),
            );
            let record = PlanFlavorRevisionRecord::graph_v1_json(
                plan_id,
                RevisionRecordBytes::try_from_vec(br#"{"plan":"v1"}"#.to_vec())
                    .expect("fixture plan bytes are non-empty"),
                worker_flavor,
            );
            let ids = record.ids();
            Self {
                catalog,
                driver: RevisionCatalogTestDriver { inner },
                clock,
                record,
                ids,
            }
        }

        fn reference(&self) -> RevisionReference {
            RevisionReference::new(
                RevisionReferenceOwner::for_execution(ExecutionId::new()),
                ExecutionContractBundleId::new(),
                self.ids,
            )
        }

        async fn insert(&self) {
            assert_eq!(
                self.catalog.insert(&self.record).await,
                Ok(RevisionInsertOutcome::Inserted)
            );
        }
    }

    /// Red→green: re-installing the same revision through a differently
    /// configured encoder is idempotent, not a permanent conflict.
    ///
    /// Record bodies are plain `serde_json` output, so field order depends on
    /// struct declaration order and on whether `serde_json` was built with
    /// `preserve_order`. Comparing raw bytes turned that incidental difference
    /// into `ContentConflict` for a revision both binaries agree on by content
    /// address — an immutable plan that could never be installed again.
    #[tokio::test]
    async fn reencoded_identical_record_is_already_present_not_a_conflict() {
        let fixture = Fixture::new();
        fixture.insert().await;

        // Same document, keys emitted in the opposite order and re-indented.
        let reencoded = PlanFlavorRevisionRecord::graph_v1_json(
            fixture.ids.plan(),
            RevisionRecordBytes::try_from_vec(br#"{  "plan"  :  "v1"  }"#.to_vec())
                .expect("re-encoded bytes are non-empty"),
            fixture.record.worker_flavor().clone(),
        );

        assert_eq!(
            fixture.catalog.insert(&reencoded).await,
            Ok(RevisionInsertOutcome::AlreadyPresent),
            "an encoding difference must not make an immutable revision uninstallable"
        );
        assert_eq!(
            fixture.catalog.load_exact(fixture.ids).await,
            Ok(fixture.record.clone()),
            "the originally stored bytes stay authoritative"
        );
    }

    /// Red→green: a draining revision reports `Draining` to every installer,
    /// not only to one that has never inserted this plan.
    ///
    /// The draining check used to be gated on the new-plan path, so a retrying
    /// installer that had already stored the identical plan got
    /// `AlreadyPresent` — reading as "installed and healthy" for a revision
    /// being retired — while a caller arriving with a new plan against the same
    /// flavor correctly got `Draining`. Whether the truth surfaced depended on
    /// the caller's own history rather than on the catalog.
    #[tokio::test]
    async fn idempotent_reinsert_reports_draining_rather_than_already_present() {
        let fixture = Fixture::new();
        fixture.insert().await;
        assert_eq!(
            fixture.catalog.insert(&fixture.record).await,
            Ok(RevisionInsertOutcome::AlreadyPresent),
            "an ordinary retry against a live revision is still idempotent"
        );

        assert!(matches!(
            fixture
                .catalog
                .begin_drain(PlanFlavorRevisionTarget::WorkerFlavor(
                    fixture.ids.worker_flavor()
                ))
                .await,
            Ok(BeginDrainOutcome::Started(_))
        ));

        assert_eq!(
            fixture.catalog.insert(&fixture.record).await,
            Err(RevisionCatalogError::Draining {
                target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
            }),
            "once the flavor is draining, the same retry must report Draining"
        );
    }

    /// A draining executable plan is reportable through `insert` at all.
    ///
    /// The plan's own lifecycle was never consulted — only the flavor's — so
    /// `Draining` on the plan could not surface from this entry point.
    #[tokio::test]
    async fn insert_reports_a_draining_executable_plan() {
        let fixture = Fixture::new();
        fixture.insert().await;
        assert!(matches!(
            fixture
                .catalog
                .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()))
                .await,
            Ok(BeginDrainOutcome::Started(_))
        ));

        assert_eq!(
            fixture.catalog.insert(&fixture.record).await,
            Err(RevisionCatalogError::Draining {
                target: PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()),
            }),
            "a draining plan must be reported, not silently accepted"
        );
    }

    #[tokio::test]
    async fn insert_and_exact_load_are_idempotent_without_latest_fallback() {
        let fixture = Fixture::new();
        fixture.insert().await;
        assert_eq!(
            fixture.catalog.insert(&fixture.record).await,
            Ok(RevisionInsertOutcome::AlreadyPresent)
        );
        assert_eq!(
            fixture.catalog.load_exact(fixture.ids).await,
            Ok(fixture.record.clone())
        );

        let wrong_ids = PlanFlavorRevisionIds::new(
            fixture.ids.plan(),
            WorkerFlavorRevisionId::from_bytes([0x42; 32]),
        );
        assert!(matches!(
            fixture.catalog.load_exact(wrong_ids).await,
            Err(RevisionCatalogError::PlanFlavorMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn immutable_plan_content_conflict_preserves_the_original_record() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let conflicting_record = PlanFlavorRevisionRecord::graph_v1_json(
            fixture.ids.plan(),
            RevisionRecordBytes::try_from_vec(br#"{"plan":"different"}"#.to_vec())
                .expect("fixture conflict bytes are non-empty"),
            fixture.record.worker_flavor().clone(),
        );

        assert_eq!(
            fixture.catalog.insert(&conflicting_record).await,
            Err(RevisionCatalogError::ContentConflict {
                target: PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()),
            })
        );
        assert_eq!(
            fixture.catalog.load_exact(fixture.ids).await,
            Ok(fixture.record.clone())
        );
    }

    #[tokio::test]
    async fn immutable_flavor_conflict_does_not_partially_insert_the_new_plan() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let new_plan_id = ExecutablePlanRevisionId::from_bytes([0x32; 32]);
        let conflicting_flavor = WorkerFlavorRevisionRecord::v1_json(
            fixture.ids.worker_flavor(),
            RevisionRecordBytes::try_from_vec(br#"{"flavor":"different"}"#.to_vec())
                .expect("fixture conflict bytes are non-empty"),
        );
        let conflicting_record = PlanFlavorRevisionRecord::graph_v1_json(
            new_plan_id,
            RevisionRecordBytes::try_from_vec(br#"{"plan":"new"}"#.to_vec())
                .expect("fixture plan bytes are non-empty"),
            conflicting_flavor,
        );

        assert_eq!(
            fixture.catalog.insert(&conflicting_record).await,
            Err(RevisionCatalogError::ContentConflict {
                target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
            })
        );
        assert_eq!(
            fixture.catalog.load_exact(conflicting_record.ids()).await,
            Err(RevisionCatalogError::PlanUnavailable {
                plan_id: new_plan_id,
            })
        );
    }

    #[tokio::test]
    async fn draining_blocks_new_references_but_retained_execution_still_loads() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        assert_eq!(
            fixture.driver.retain(retained),
            Ok(RetainDecision::Retained)
        );
        assert!(matches!(
            fixture
                .catalog
                .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
                    fixture.ids.plan()
                ))
                .await,
            Ok(BeginDrainOutcome::Started(counts))
                if counts.live_executions() == 1 && counts.rollback_windows() == 0
        ));
        assert!(matches!(
            fixture
                .catalog
                .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
                    fixture.ids.plan()
                ))
                .await,
            Ok(BeginDrainOutcome::AlreadyDraining(counts))
                if counts.live_executions() == 1 && counts.rollback_windows() == 0
        ));
        assert_eq!(
            fixture.catalog.load_exact(fixture.ids).await,
            Ok(fixture.record.clone())
        );

        let new_reference = fixture.reference();
        assert_eq!(
            fixture.driver.retain(new_reference),
            Err(InternalRevisionError::Draining)
        );
        assert_eq!(
            fixture.driver.retain(retained),
            Ok(RetainDecision::AlreadyRetained)
        );
        assert_eq!(fixture.driver.reference_rows(), 1);
    }

    #[tokio::test]
    async fn same_owner_with_different_pins_is_rejected_without_reference_delta() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        assert_eq!(
            fixture.driver.retain(retained),
            Ok(RetainDecision::Retained)
        );
        let mismatched = RevisionReference::new(
            retained.owner,
            ExecutionContractBundleId::new(),
            retained.ids,
        );
        assert_eq!(
            fixture.driver.retain(mismatched),
            Err(InternalRevisionError::ReferenceMismatch)
        );
        assert_eq!(fixture.driver.reference_rows(), 1);
    }

    #[tokio::test]
    async fn stale_transition_with_different_pins_cannot_close_the_owner_reference() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        fixture
            .driver
            .retain(retained)
            .expect("active fixture pair accepts a live reference");
        let stale_reference = RevisionReference::new(
            retained.owner,
            ExecutionContractBundleId::new(),
            retained.ids,
        );

        assert_eq!(
            fixture
                .driver
                .transition(OwningReferenceTransition::ReleaseLive {
                    reference: stale_reference,
                }),
            Err(InternalRevisionError::ReferenceMismatch)
        );
        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        assert!(matches!(
            fixture.catalog.begin_drain(target).await,
            Ok(BeginDrainOutcome::Started(references))
                if references.live_executions() == 1
        ));
    }

    #[tokio::test]
    async fn insert_does_not_heal_a_plan_whose_pinned_flavor_row_is_missing() {
        let fixture = Fixture::new();
        fixture.insert().await;
        fixture
            .driver
            .remove_worker_flavor_record(fixture.ids.worker_flavor());

        assert_eq!(
            fixture.catalog.insert(&fixture.record).await,
            Err(RevisionCatalogError::CorruptRecord {
                target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
            })
        );
        assert!(matches!(
            fixture.catalog.load_exact(fixture.ids).await,
            Err(RevisionCatalogError::WorkerFlavorUnavailable { .. })
        ));
    }

    #[tokio::test]
    async fn rollback_deadline_equality_is_expired_and_retry_cannot_extend_it() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        fixture
            .driver
            .retain(retained)
            .expect("active fixture pair accepts a live reference");
        let now = fixture.clock.now();
        let rollback_window_id = RollbackWindowId([0x55; 16]);
        let transition = OwningReferenceTransition::RetainForRollback {
            reference: retained,
            window_id: rollback_window_id,
            retain_until: now,
        };
        assert_eq!(
            fixture.driver.transition(transition),
            Ok(ReferenceDecision::Applied)
        );
        assert_eq!(
            fixture.driver.transition(transition),
            Ok(ReferenceDecision::AlreadyApplied)
        );
        assert_eq!(
            fixture
                .driver
                .transition(OwningReferenceTransition::RetainForRollback {
                    reference: retained,
                    window_id: rollback_window_id,
                    retain_until: now + TimeDelta::minutes(5),
                }),
            Err(InternalRevisionError::ReferenceMismatch)
        );

        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        assert!(matches!(
            fixture.catalog.begin_drain(target).await,
            Ok(BeginDrainOutcome::Started(counts)) if counts.is_empty()
        ));
        assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
    }

    #[tokio::test]
    async fn rollback_release_requires_the_exact_window_and_rejects_stale_live_release() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        fixture
            .driver
            .retain(retained)
            .expect("active fixture pair accepts a live reference");
        let rollback_window_id = RollbackWindowId([0x57; 16]);
        let retain_until = fixture.clock.now() + TimeDelta::minutes(5);
        fixture
            .driver
            .transition(OwningReferenceTransition::RetainForRollback {
                reference: retained,
                window_id: rollback_window_id,
                retain_until,
            })
            .expect("live owner may enter its rollback window");

        assert_eq!(
            fixture
                .driver
                .transition(OwningReferenceTransition::ReleaseLive {
                    reference: retained,
                }),
            Err(InternalRevisionError::ReferenceMismatch)
        );
        assert_eq!(
            fixture
                .driver
                .transition(OwningReferenceTransition::ReleaseRollback {
                    reference: retained,
                    window_id: rollback_window_id,
                    retain_until: retain_until + TimeDelta::seconds(1),
                }),
            Err(InternalRevisionError::ReferenceMismatch)
        );

        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        assert!(matches!(
            fixture.catalog.begin_drain(target).await,
            Ok(BeginDrainOutcome::Started(references))
                if references.rollback_windows() == 1
        ));
        let release = OwningReferenceTransition::ReleaseRollback {
            reference: retained,
            window_id: rollback_window_id,
            retain_until,
        };
        assert_eq!(
            fixture.driver.transition(release),
            Ok(ReferenceDecision::Applied)
        );
        assert_eq!(
            fixture.driver.transition(release),
            Ok(ReferenceDecision::AlreadyApplied)
        );
        assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
    }

    #[tokio::test]
    async fn live_and_unexpired_rollback_rows_block_delete_until_owner_releases() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        fixture
            .driver
            .retain(retained)
            .expect("active fixture pair accepts a live reference");
        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        fixture
            .catalog
            .begin_drain(target)
            .await
            .expect("inserted plan starts draining");
        assert!(matches!(
            fixture.catalog.delete_drained(target).await,
            Err(RevisionCatalogError::Referenced { references, .. })
                if references.live_executions() == 1
        ));

        let retain_until = fixture.clock.now() + TimeDelta::minutes(5);
        fixture
            .driver
            .transition(OwningReferenceTransition::RetainForRollback {
                reference: retained,
                window_id: RollbackWindowId([0x56; 16]),
                retain_until,
            })
            .expect("live owner may enter its rollback window");
        assert!(matches!(
            fixture.catalog.delete_drained(target).await,
            Err(RevisionCatalogError::Referenced { references, .. })
                if references.rollback_windows() == 1
        ));

        fixture.clock.set(retain_until);
        assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
    }

    #[tokio::test]
    async fn flavor_delete_waits_for_every_non_deleted_dependent_plan() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor());
        fixture
            .catalog
            .begin_drain(flavor_target)
            .await
            .expect("inserted flavor starts draining");
        assert!(matches!(
            fixture.catalog.delete_drained(flavor_target).await,
            Err(RevisionCatalogError::DependentPlans {
                dependent_plans: 1,
                ..
            })
        ));

        let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        fixture
            .catalog
            .begin_drain(plan_target)
            .await
            .expect("inserted plan starts draining");
        fixture
            .catalog
            .delete_drained(plan_target)
            .await
            .expect("unreferenced plan may be deleted");
        assert_eq!(fixture.catalog.delete_drained(flavor_target).await, Ok(()));
        assert!(matches!(
            fixture.catalog.insert(&fixture.record).await,
            Err(RevisionCatalogError::Deleted { .. })
        ));
    }

    #[tokio::test]
    async fn retain_racing_drain_has_exactly_one_linearized_winner() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let reference = fixture.reference();
        let barrier = Arc::new(Barrier::new(2));
        let retain_driver = fixture.driver.clone();
        let retain_barrier = Arc::clone(&barrier);
        let retain_task = tokio::spawn(async move {
            retain_barrier.wait().await;
            retain_driver.retain(reference)
        });
        let drain_catalog = fixture.catalog.clone();
        let drain_barrier = Arc::clone(&barrier);
        let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        let drain_task = tokio::spawn(async move {
            drain_barrier.wait().await;
            drain_catalog.begin_drain(plan_target).await
        });

        let retain_outcome = retain_task.await.expect("retain task must not panic");
        let drain_outcome = drain_task.await.expect("drain task must not panic");
        match (retain_outcome, drain_outcome) {
            (Ok(RetainDecision::Retained), Ok(BeginDrainOutcome::Started(reference_counts))) => {
                assert_eq!(reference_counts.live_executions(), 1);
            },
            (
                Err(InternalRevisionError::Draining),
                Ok(BeginDrainOutcome::Started(reference_counts)),
            ) => assert!(reference_counts.is_empty()),
            outcomes => panic!("unexpected retain/drain outcomes: {outcomes:?}"),
        }
    }

    #[tokio::test]
    async fn exact_load_racing_delete_returns_copied_record_or_deleted_tombstone() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        fixture
            .catalog
            .begin_drain(target)
            .await
            .expect("inserted plan starts draining");

        let barrier = Arc::new(Barrier::new(2));
        let load_catalog = fixture.catalog.clone();
        let load_barrier = Arc::clone(&barrier);
        let ids = fixture.ids;
        let load_task = tokio::spawn(async move {
            load_barrier.wait().await;
            load_catalog.load_exact(ids).await
        });
        let delete_catalog = fixture.catalog.clone();
        let delete_barrier = Arc::clone(&barrier);
        let delete_task = tokio::spawn(async move {
            delete_barrier.wait().await;
            delete_catalog.delete_drained(target).await
        });

        let load_outcome = load_task.await.expect("load task must not panic");
        let delete_outcome = delete_task.await.expect("delete task must not panic");
        assert_eq!(delete_outcome, Ok(()));
        match load_outcome {
            Ok(record) => assert_eq!(record, fixture.record),
            Err(RevisionCatalogError::Deleted {
                target: deleted_target,
            }) => assert_eq!(deleted_target, target),
            outcome => panic!("unexpected load/delete outcome: {outcome:?}"),
        }
    }

    #[tokio::test]
    async fn release_is_idempotent_and_unblocks_guarded_delete() {
        let fixture = Fixture::new();
        fixture.insert().await;
        let retained = fixture.reference();
        fixture
            .driver
            .retain(retained)
            .expect("active fixture pair accepts a live reference");
        let transition = OwningReferenceTransition::ReleaseLive {
            reference: retained,
        };
        assert_eq!(
            fixture.driver.transition(transition),
            Ok(ReferenceDecision::Applied)
        );
        assert_eq!(
            fixture.driver.transition(transition),
            Ok(ReferenceDecision::AlreadyApplied)
        );

        let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
        fixture
            .catalog
            .begin_drain(target)
            .await
            .expect("inserted plan starts draining");
        assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
        assert_eq!(
            fixture.catalog.delete_drained(target).await,
            Ok(()),
            "lost delete acknowledgement is reconciled by the tombstone"
        );
    }
}
