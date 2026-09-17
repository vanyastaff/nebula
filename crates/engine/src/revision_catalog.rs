//! Runtime-control bridge for exact executable-plan and worker-flavor records.
//!
//! This module translates between the storage port's opaque record bytes and
//! the plugin-owned checked record types. A successful load proves structural
//! integrity and exact compatibility with one supplied
//! [`FrozenPluginRegistry`]. It is recorded data, not tenant authority,
//! admission, a resource binding, or permission to execute.

use std::{fmt, sync::Arc};

use nebula_plugin::{
    ExecutablePlanIntegrityError, ExecutablePlanRevision, FrozenPluginRegistry,
    PlanRegistryCompatibilityError, RecordedExecutablePlanRevisionV1,
    RecordedWorkerFlavorRevisionV1, WorkerFlavorIntegrityError, WorkerFlavorRevision,
};
use nebula_storage_port::{
    ExecutablePlanRecordFormat, PlanFlavorCatalog, PlanFlavorCatalogWriter, PlanFlavorRevisionIds,
    PlanFlavorRevisionRecord, PlanFlavorRevisionTarget, RevisionCatalogError,
    RevisionInsertOutcome, RevisionRecordBytes, WorkerFlavorRecordFormat,
    WorkerFlavorRevisionRecord,
};

/// Integrity-checked exact plan/flavor records loaded for runtime control.
///
/// This value carries no tenant proof or durable mutation capability. Runtime
/// admission must still validate authenticated bindings and retain the exact
/// revisions inside its owning storage transaction.
#[derive(Clone)]
pub struct LoadedPlanFlavorRevision {
    plan: ExecutablePlanRevision,
    registry: Arc<FrozenPluginRegistry>,
}

impl LoadedPlanFlavorRevision {
    /// Exact immutable executable plan.
    #[must_use]
    pub const fn plan(&self) -> &ExecutablePlanRevision {
        &self.plan
    }

    /// Exact immutable worker-flavor descriptor owned by the checked registry.
    #[must_use]
    pub fn worker_flavor(&self) -> &WorkerFlavorRevision {
        self.registry.revision()
    }

    /// Frozen registry snapshot against which this plan was checked.
    ///
    /// Keeping the snapshot inside the witness prevents compatibility proof
    /// from outliving or being silently detached from the exact registry that
    /// established it.
    #[must_use]
    pub fn registry(&self) -> &FrozenPluginRegistry {
        &self.registry
    }
}

impl fmt::Debug for LoadedPlanFlavorRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedPlanFlavorRevision")
            .field("plan_revision_id", &self.plan.id())
            .field("worker_flavor_revision_id", &self.registry.revision().id())
            .finish()
    }
}

/// Read-only runtime-control bridge over an exact plan/flavor catalog.
pub struct PlanFlavorRevisionLoader {
    catalog: Arc<dyn PlanFlavorCatalog>,
}

impl PlanFlavorRevisionLoader {
    /// Build a loader with no catalog administration capability.
    #[must_use]
    pub fn new(catalog: Arc<dyn PlanFlavorCatalog>) -> Self {
        Self { catalog }
    }

    /// Load and revalidate only the requested exact plan/flavor pair.
    ///
    /// The method never selects a current/latest/closest revision and never
    /// invokes the compiler.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the catalog cannot load the exact pair,
    /// either record is malformed or has a forged identity, the records do
    /// not match the requested pins, or the supplied frozen registry is not
    /// exactly compatible.
    #[tracing::instrument(
        skip(self, registry),
        fields(
            executable_plan_revision_id = %ids.plan(),
            worker_flavor_revision_id = %ids.worker_flavor(),
            outcome = tracing::field::Empty,
            error_code = tracing::field::Empty,
        )
    )]
    pub async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
        registry: Arc<FrozenPluginRegistry>,
    ) -> Result<LoadedPlanFlavorRevision, PlanFlavorRevisionBridgeError> {
        let result = self.load_exact_inner(ids, registry).await;
        record_outcome(&result);
        result
    }

    async fn load_exact_inner(
        &self,
        ids: PlanFlavorRevisionIds,
        registry: Arc<FrozenPluginRegistry>,
    ) -> Result<LoadedPlanFlavorRevision, PlanFlavorRevisionBridgeError> {
        let stored = self.catalog.load_exact(ids).await?;

        if stored.ids() != ids {
            return Err(PlanFlavorRevisionBridgeError::StoredPairMismatch {
                requested: ids,
                stored: stored.ids(),
            });
        }
        if stored.worker_flavor().format() != WorkerFlavorRecordFormat::V1Json {
            return Err(RevisionCatalogError::UnsupportedRecordFormat {
                target: PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor()),
            }
            .into());
        }
        if stored.plan_format() != ExecutablePlanRecordFormat::GraphV1Json {
            return Err(RevisionCatalogError::UnsupportedRecordFormat {
                target: PlanFlavorRevisionTarget::ExecutablePlan(ids.plan()),
            }
            .into());
        }

        let recorded_flavor = stored
            .worker_flavor()
            .record_bytes()
            .deserialize_json::<RecordedWorkerFlavorRevisionV1>(
                PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor()),
            )
            .map_err(|source| {
                record_decode_error(source, PlanFlavorRevisionBridgeError::FlavorRecordDecode)
            })?;
        let worker_flavor = WorkerFlavorRevision::try_from(recorded_flavor)?;
        if worker_flavor.id() != ids.worker_flavor() {
            return Err(PlanFlavorRevisionBridgeError::FlavorIdentityMismatch {
                requested: ids.worker_flavor(),
                decoded: worker_flavor.id(),
            });
        }
        if &worker_flavor != registry.revision() {
            return Err(PlanFlavorRevisionBridgeError::RegistryFlavorMismatch {
                requested: ids.worker_flavor(),
                registry: registry.revision().id(),
            });
        }

        let recorded_plan = stored
            .plan_record_bytes()
            .deserialize_json::<RecordedExecutablePlanRevisionV1>(
                PlanFlavorRevisionTarget::ExecutablePlan(ids.plan()),
            )
            .map_err(|source| {
                record_decode_error(source, PlanFlavorRevisionBridgeError::PlanRecordDecode)
            })?;
        let plan = ExecutablePlanRevision::try_from(recorded_plan)?;
        if plan.id() != ids.plan() {
            return Err(PlanFlavorRevisionBridgeError::PlanIdentityMismatch {
                requested: ids.plan(),
                decoded: plan.id(),
            });
        }
        if plan.worker_flavor_revision_id() != ids.worker_flavor() {
            return Err(PlanFlavorRevisionBridgeError::PlanFlavorMismatch {
                plan: plan.id(),
                requested: ids.worker_flavor(),
                decoded: plan.worker_flavor_revision_id(),
            });
        }
        plan.validate_against(&registry)?;

        Ok(LoadedPlanFlavorRevision { plan, registry })
    }
}

impl fmt::Debug for PlanFlavorRevisionLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanFlavorRevisionLoader")
            .finish_non_exhaustive()
    }
}

/// Contract-activation bridge that installs one checked plan/flavor pair.
///
/// It owns only catalog installation. It cannot create an execution or retain
/// a live/rollback reference.
pub struct PlanFlavorRevisionInstaller {
    catalog: Arc<dyn PlanFlavorCatalogWriter>,
}

impl PlanFlavorRevisionInstaller {
    /// Build an installer with insert-only catalog capability.
    ///
    /// The capability cannot drain, delete, or mutate revision references.
    #[must_use]
    pub fn new(catalog: Arc<dyn PlanFlavorCatalogWriter>) -> Self {
        Self { catalog }
    }

    /// Revalidate, encode, and atomically install one immutable exact pair.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the plan is incompatible with the
    /// supplied registry, a checked record cannot be encoded, or the catalog
    /// rejects the immutable insertion.
    #[tracing::instrument(
        skip(self, registry, plan),
        fields(
            executable_plan_revision_id = %plan.id(),
            worker_flavor_revision_id = %plan.worker_flavor_revision_id(),
            outcome = tracing::field::Empty,
            error_code = tracing::field::Empty,
        )
    )]
    pub async fn install(
        &self,
        registry: &FrozenPluginRegistry,
        plan: &ExecutablePlanRevision,
    ) -> Result<RevisionInsertOutcome, PlanFlavorRevisionBridgeError> {
        let result = self.install_inner(registry, plan).await;
        record_outcome(&result);
        result
    }

    async fn install_inner(
        &self,
        registry: &FrozenPluginRegistry,
        plan: &ExecutablePlanRevision,
    ) -> Result<RevisionInsertOutcome, PlanFlavorRevisionBridgeError> {
        plan.validate_against(registry)?;

        let flavor_record = RecordedWorkerFlavorRevisionV1::from(registry.revision());
        let flavor_bytes = serde_json::to_vec(&flavor_record)
            .map_err(|_| PlanFlavorRevisionBridgeError::FlavorRecordEncode)?;
        let flavor = WorkerFlavorRevisionRecord::v1_json(
            registry.revision().id(),
            RevisionRecordBytes::try_from_vec(flavor_bytes)?,
        );

        let plan_record = RecordedExecutablePlanRevisionV1::from(plan);
        let plan_bytes = serde_json::to_vec(&plan_record)
            .map_err(|_| PlanFlavorRevisionBridgeError::PlanRecordEncode)?;
        let record = PlanFlavorRevisionRecord::graph_v1_json(
            plan.id(),
            RevisionRecordBytes::try_from_vec(plan_bytes)?,
            flavor,
        );

        self.catalog.insert(&record).await.map_err(Into::into)
    }
}

impl fmt::Debug for PlanFlavorRevisionInstaller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanFlavorRevisionInstaller")
            .finish_non_exhaustive()
    }
}

/// Failure to install or exact-load a checked plan/flavor pair.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlanFlavorRevisionBridgeError {
    /// The technical catalog rejected the operation.
    #[error("plan/flavor catalog operation failed")]
    Catalog {
        /// Closed catalog failure.
        #[source]
        source: RevisionCatalogError,
    },

    /// A checked plan record could not be serialized.
    #[error("checked executable-plan record could not be encoded")]
    PlanRecordEncode,

    /// A stored plan record could not be decoded.
    #[error("stored executable-plan record is corrupt")]
    PlanRecordDecode,

    /// A checked worker-flavor record could not be serialized.
    #[error("checked worker-flavor record could not be encoded")]
    FlavorRecordEncode,

    /// A stored worker-flavor record could not be decoded.
    #[error("stored worker-flavor record is corrupt")]
    FlavorRecordDecode,

    /// The plugin-owned executable-plan integrity check failed.
    #[error("stored executable-plan record failed integrity validation")]
    PlanIntegrity {
        /// Structural integrity failure.
        #[source]
        source: ExecutablePlanIntegrityError,
    },

    /// The plugin-owned worker-flavor integrity check failed.
    #[error("stored worker-flavor record failed integrity validation")]
    FlavorIntegrity {
        /// Structural integrity failure.
        #[source]
        source: WorkerFlavorIntegrityError,
    },

    /// The executable plan is not exact-compatible with the supplied registry.
    #[error("stored executable plan is incompatible with the frozen registry")]
    RegistryCompatibility {
        /// Exact compatibility failure.
        #[source]
        source: PlanRegistryCompatibilityError,
    },

    /// The catalog returned another pair than the one requested.
    #[error("catalog returned a different exact plan/flavor pair")]
    StoredPairMismatch {
        /// Pair requested by runtime control.
        requested: PlanFlavorRevisionIds,
        /// Pair returned by storage.
        stored: PlanFlavorRevisionIds,
    },

    /// Decoded plan identity differs from the requested exact ID.
    #[error("decoded executable-plan identity differs from the requested revision")]
    PlanIdentityMismatch {
        /// Requested exact plan.
        requested: nebula_core::ExecutablePlanRevisionId,
        /// Integrity-checked decoded plan.
        decoded: nebula_core::ExecutablePlanRevisionId,
    },

    /// Decoded flavor identity differs from the requested exact ID.
    #[error("decoded worker-flavor identity differs from the requested revision")]
    FlavorIdentityMismatch {
        /// Requested exact flavor.
        requested: nebula_core::WorkerFlavorRevisionId,
        /// Integrity-checked decoded flavor.
        decoded: nebula_core::WorkerFlavorRevisionId,
    },

    /// The decoded plan pins another worker flavor.
    #[error("decoded executable plan pins a different worker flavor")]
    PlanFlavorMismatch {
        /// Integrity-checked plan identity.
        plan: nebula_core::ExecutablePlanRevisionId,
        /// Requested exact flavor.
        requested: nebula_core::WorkerFlavorRevisionId,
        /// Flavor pinned by the decoded plan.
        decoded: nebula_core::WorkerFlavorRevisionId,
    },

    /// The supplied process registry is not the requested exact flavor.
    #[error("frozen registry is not the requested exact worker flavor")]
    RegistryFlavorMismatch {
        /// Requested and decoded exact flavor.
        requested: nebula_core::WorkerFlavorRevisionId,
        /// Flavor exposed by the supplied process registry.
        registry: nebula_core::WorkerFlavorRevisionId,
    },
}

impl From<RevisionCatalogError> for PlanFlavorRevisionBridgeError {
    fn from(source: RevisionCatalogError) -> Self {
        Self::Catalog { source }
    }
}

impl From<ExecutablePlanIntegrityError> for PlanFlavorRevisionBridgeError {
    fn from(source: ExecutablePlanIntegrityError) -> Self {
        Self::PlanIntegrity { source }
    }
}

impl From<WorkerFlavorIntegrityError> for PlanFlavorRevisionBridgeError {
    fn from(source: WorkerFlavorIntegrityError) -> Self {
        Self::FlavorIntegrity { source }
    }
}

impl From<PlanRegistryCompatibilityError> for PlanFlavorRevisionBridgeError {
    fn from(source: PlanRegistryCompatibilityError) -> Self {
        Self::RegistryCompatibility { source }
    }
}

impl PlanFlavorRevisionBridgeError {
    pub(crate) const fn is_transient_catalog_failure(&self) -> bool {
        matches!(
            self,
            Self::Catalog {
                source: RevisionCatalogError::Unavailable | RevisionCatalogError::OutcomeUnknown
            }
        )
    }

    const fn code(&self) -> &'static str {
        match self {
            Self::Catalog { source } => catalog_error_code(source),
            Self::PlanRecordEncode => "REVISION_CATALOG:PLAN_ENCODE",
            Self::PlanRecordDecode => "REVISION_CATALOG:PLAN_DECODE",
            Self::FlavorRecordEncode => "REVISION_CATALOG:FLAVOR_ENCODE",
            Self::FlavorRecordDecode => "REVISION_CATALOG:FLAVOR_DECODE",
            Self::PlanIntegrity { .. } => "REVISION_CATALOG:PLAN_INTEGRITY",
            Self::FlavorIntegrity { .. } => "REVISION_CATALOG:FLAVOR_INTEGRITY",
            Self::RegistryCompatibility { .. } => "REVISION_CATALOG:REGISTRY_INCOMPATIBLE",
            Self::StoredPairMismatch { .. } => "REVISION_CATALOG:STORED_PAIR_MISMATCH",
            Self::PlanIdentityMismatch { .. } => "REVISION_CATALOG:PLAN_ID_MISMATCH",
            Self::FlavorIdentityMismatch { .. } => "REVISION_CATALOG:FLAVOR_ID_MISMATCH",
            Self::PlanFlavorMismatch { .. } => "REVISION_CATALOG:PLAN_FLAVOR_MISMATCH",
            Self::RegistryFlavorMismatch { .. } => "REVISION_CATALOG:REGISTRY_FLAVOR_MISMATCH",
        }
    }
}

const fn catalog_error_code(error: &RevisionCatalogError) -> &'static str {
    match error {
        RevisionCatalogError::PlanUnavailable { .. } => "REVISION_CATALOG:PLAN_UNAVAILABLE",
        RevisionCatalogError::WorkerFlavorUnavailable { .. } => {
            "REVISION_CATALOG:FLAVOR_UNAVAILABLE"
        },
        RevisionCatalogError::PlanFlavorMismatch { .. } => "REVISION_CATALOG:PAIR_MISMATCH",
        RevisionCatalogError::ContentConflict { .. } => "REVISION_CATALOG:CONTENT_CONFLICT",
        RevisionCatalogError::Draining { .. } => "REVISION_CATALOG:DRAINING",
        RevisionCatalogError::Deleted { .. } => "REVISION_CATALOG:DELETED",
        RevisionCatalogError::DrainRequired { .. } => "REVISION_CATALOG:DRAIN_REQUIRED",
        RevisionCatalogError::Referenced { .. } => "REVISION_CATALOG:REFERENCED",
        RevisionCatalogError::DependentPlans { .. } => "REVISION_CATALOG:DEPENDENT_PLANS",
        RevisionCatalogError::EmptyRecord => "REVISION_CATALOG:EMPTY_RECORD",
        RevisionCatalogError::RecordTooLarge { .. } => "REVISION_CATALOG:RECORD_TOO_LARGE",
        RevisionCatalogError::RecordNestingTooDeep { .. } => {
            "REVISION_CATALOG:RECORD_NESTING_TOO_DEEP"
        },
        RevisionCatalogError::RecordStringTooLarge { .. } => {
            "REVISION_CATALOG:RECORD_STRING_TOO_LARGE"
        },
        RevisionCatalogError::RecordStringBudgetExceeded { .. } => {
            "REVISION_CATALOG:RECORD_STRING_BUDGET_EXCEEDED"
        },
        RevisionCatalogError::RecordCollectionBudgetExceeded { .. } => {
            "REVISION_CATALOG:RECORD_COLLECTION_BUDGET_EXCEEDED"
        },
        RevisionCatalogError::UnsupportedRecordFormat { .. } => {
            "REVISION_CATALOG:UNSUPPORTED_FORMAT"
        },
        RevisionCatalogError::CorruptRecord { .. } => "REVISION_CATALOG:CORRUPT_RECORD",
        RevisionCatalogError::Unavailable => "REVISION_CATALOG:UNAVAILABLE",
        RevisionCatalogError::OutcomeUnknown => "REVISION_CATALOG:OUTCOME_UNKNOWN",
        _ => "REVISION_CATALOG:UNKNOWN_STORAGE_ERROR",
    }
}

fn record_decode_error(
    source: RevisionCatalogError,
    malformed_record: PlanFlavorRevisionBridgeError,
) -> PlanFlavorRevisionBridgeError {
    if matches!(source, RevisionCatalogError::CorruptRecord { .. }) {
        malformed_record
    } else {
        source.into()
    }
}

fn record_outcome<T>(result: &Result<T, PlanFlavorRevisionBridgeError>) {
    let span = tracing::Span::current();
    match result {
        Ok(_) => {
            span.record("outcome", "success");
        },
        Err(error) => {
            span.record("outcome", "error");
            span.record("error_code", error.code());
        },
    }
}

#[cfg(test)]
#[path = "revision_catalog_tests.rs"]
mod tests;
