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

        let recorded_flavor = serde_json::from_slice::<RecordedWorkerFlavorRevisionV1>(
            stored.worker_flavor().bytes(),
        )
        .map_err(|_| PlanFlavorRevisionBridgeError::FlavorRecordDecode)?;
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

        let recorded_plan =
            serde_json::from_slice::<RecordedExecutablePlanRevisionV1>(stored.plan_bytes())
                .map_err(|_| PlanFlavorRevisionBridgeError::PlanRecordDecode)?;
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
        RevisionCatalogError::UnsupportedRecordFormat { .. } => {
            "REVISION_CATALOG:UNSUPPORTED_FORMAT"
        },
        RevisionCatalogError::CorruptRecord { .. } => "REVISION_CATALOG:CORRUPT_RECORD",
        RevisionCatalogError::Unavailable => "REVISION_CATALOG:UNAVAILABLE",
        RevisionCatalogError::OutcomeUnknown => "REVISION_CATALOG:OUTCOME_UNKNOWN",
        _ => "REVISION_CATALOG:UNKNOWN_STORAGE_ERROR",
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
mod tests {
    use std::{future::Future, pin::Pin};

    use nebula_action::{
        ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
    };
    use nebula_core::{
        ActionKey, ArtifactSetDigest, Dependencies, ExecutablePlanRevisionId,
        WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, node_key,
    };
    use nebula_plugin::{Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
    use nebula_schema::ValidSchema;
    use nebula_storage::InMemoryExecutionStore;
    use nebula_storage_port::{
        BeginDrainOutcome, PlanFlavorCatalogAdmin, PlanFlavorRevisionTarget,
        RevisionReferenceCounts,
    };
    use nebula_workflow::{NodeDefinition, WorkflowBuilder};
    use parking_lot::Mutex;

    use super::*;

    struct TestActionFactory {
        metadata: ActionMetadata,
        dependencies: Dependencies,
    }

    impl ActionFactory for TestActionFactory {
        fn metadata(&self) -> &ActionMetadata {
            &self.metadata
        }

        fn dependencies(&self) -> &Dependencies {
            &self.dependencies
        }

        fn instantiate<'a>(
            &'a self,
            _node: &'a NodeDefinition,
            _context: &'a dyn ActionContext,
        ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
            Box::pin(async {
                Err(ActionError::fatal(
                    "the exact-revision test factory is never instantiated",
                ))
            })
        }
    }

    struct TestPlugin {
        manifest: PluginManifest,
        actions: Vec<Arc<dyn ActionFactory>>,
    }

    impl fmt::Debug for TestPlugin {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("TestPlugin")
                .field("key", self.manifest.key())
                .finish()
        }
    }

    impl Plugin for TestPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
            self.actions.clone()
        }
    }

    fn frozen(artifact_byte: u8) -> Arc<FrozenPluginRegistry> {
        let key = ActionKey::new("demo.run").expect("fixture action key is valid");
        let action = Arc::new(TestActionFactory {
            metadata: ActionMetadata::new(key, "Run", "exact revision fixture")
                .with_kind(ActionKind::Stateless)
                .with_schema(ValidSchema::empty())
                .with_output_schema(ValidSchema::empty()),
            dependencies: Dependencies::new(),
        });
        let plugin = TestPlugin {
            manifest: PluginManifest::builder("demo", "Demo")
                .build()
                .expect("fixture manifest is valid"),
            actions: vec![action],
        };
        let resolved =
            Arc::new(ResolvedPlugin::from(plugin).expect("fixture plugin contracts resolve"));
        let mut registry = PluginRegistry::new();
        registry
            .register(resolved)
            .expect("fixture plugin registers once");
        Arc::new(
            registry
                .freeze(
                    ArtifactSetDigest::from_bytes([artifact_byte; 32]),
                    "1.0.0"
                        .parse()
                        .expect("fixture runtime contract version is valid"),
                )
                .expect("fixture registry freezes"),
        )
    }

    fn compile(registry: &FrozenPluginRegistry) -> ExecutablePlanRevision {
        let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "run")
            .expect("fixture node is valid");
        let workflow = WorkflowBuilder::new("Exact revision fixture")
            .id(WorkflowId::from_bytes([0x42; 16]))
            .add_node(node)
            .build()
            .expect("fixture workflow is valid");
        registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x43; 16]), &workflow)
            .expect("fixture plan compiles")
    }

    #[derive(Debug, Default)]
    struct StubCatalog {
        record: Mutex<Option<PlanFlavorRevisionRecord>>,
    }

    impl StubCatalog {
        fn replace(&self, record: PlanFlavorRevisionRecord) {
            *self.record.lock() = Some(record);
        }
    }

    #[async_trait::async_trait]
    impl PlanFlavorCatalog for StubCatalog {
        async fn load_exact(
            &self,
            ids: PlanFlavorRevisionIds,
        ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
            self.record
                .lock()
                .as_ref()
                .filter(|record| record.ids() == ids)
                .cloned()
                .ok_or_else(|| RevisionCatalogError::PlanUnavailable {
                    plan_id: ids.plan(),
                })
        }
    }

    #[async_trait::async_trait]
    impl PlanFlavorCatalogWriter for StubCatalog {
        async fn insert(
            &self,
            record: &PlanFlavorRevisionRecord,
        ) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
            let mut stored = self.record.lock();
            match stored.as_ref() {
                None => {
                    *stored = Some(record.clone());
                    Ok(RevisionInsertOutcome::Inserted)
                },
                Some(existing) if existing == record => Ok(RevisionInsertOutcome::AlreadyPresent),
                Some(_) => Err(RevisionCatalogError::ContentConflict {
                    target: PlanFlavorRevisionTarget::ExecutablePlan(record.ids().plan()),
                }),
            }
        }
    }

    #[async_trait::async_trait]
    impl PlanFlavorCatalogAdmin for StubCatalog {
        async fn begin_drain(
            &self,
            _target: PlanFlavorRevisionTarget,
        ) -> Result<BeginDrainOutcome, RevisionCatalogError> {
            Ok(BeginDrainOutcome::Started(RevisionReferenceCounts::new(
                0, 0,
            )))
        }

        async fn delete_drained(
            &self,
            _target: PlanFlavorRevisionTarget,
        ) -> Result<(), RevisionCatalogError> {
            Ok(())
        }
    }

    fn bridges(
        catalog: Arc<StubCatalog>,
    ) -> (PlanFlavorRevisionInstaller, PlanFlavorRevisionLoader) {
        (
            PlanFlavorRevisionInstaller::new(catalog.clone()),
            PlanFlavorRevisionLoader::new(catalog),
        )
    }

    #[tokio::test]
    async fn install_then_load_revalidates_the_exact_pair() {
        let registry = frozen(0x31);
        let plan = compile(&registry);
        let catalog = Arc::new(StubCatalog::default());
        let (installer, loader) = bridges(catalog);

        let first = installer
            .install(&registry, &plan)
            .await
            .expect("checked pair installs");
        let second = installer
            .install(&registry, &plan)
            .await
            .expect("byte-identical install is idempotent");
        assert_eq!(first, RevisionInsertOutcome::Inserted);
        assert_eq!(second, RevisionInsertOutcome::AlreadyPresent);

        let ids = PlanFlavorRevisionIds::new(plan.id(), registry.revision().id());
        let loaded = loader
            .load_exact(ids, Arc::clone(&registry))
            .await
            .expect("exact pair reloads and revalidates");
        assert_eq!(loaded.plan().id(), plan.id());
        assert_eq!(loaded.worker_flavor(), registry.revision());
        assert!(std::ptr::eq(loaded.registry(), registry.as_ref()));
    }

    #[tokio::test]
    async fn real_inmemory_catalog_loads_during_drain_then_honors_the_tombstone() {
        let registry = frozen(0x36);
        let plan = compile(&registry);
        let ids = PlanFlavorRevisionIds::new(plan.id(), registry.revision().id());
        let execution_store = InMemoryExecutionStore::new();
        let catalog = Arc::new(execution_store.plan_flavor_catalog());
        let installer = PlanFlavorRevisionInstaller::new(catalog.clone());
        let loader = PlanFlavorRevisionLoader::new(catalog.clone());

        assert_eq!(
            installer
                .install(&registry, &plan)
                .await
                .expect("checked pair installs in the real catalog"),
            RevisionInsertOutcome::Inserted
        );
        assert!(matches!(
            catalog
                .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(ids.plan()))
                .await,
            Ok(BeginDrainOutcome::Started(references)) if references.is_empty()
        ));

        let loaded = loader
            .load_exact(ids, Arc::clone(&registry))
            .await
            .expect("a draining exact revision remains loadable");
        assert_eq!(loaded.plan().id(), plan.id());
        assert!(std::ptr::eq(loaded.registry(), registry.as_ref()));

        catalog
            .delete_drained(PlanFlavorRevisionTarget::ExecutablePlan(ids.plan()))
            .await
            .expect("an unreferenced draining plan may be tombstoned");
        assert!(matches!(
            loader.load_exact(ids, registry).await,
            Err(PlanFlavorRevisionBridgeError::Catalog {
                source: RevisionCatalogError::Deleted {
                    target: PlanFlavorRevisionTarget::ExecutablePlan(deleted_plan),
                },
            }) if deleted_plan == ids.plan()
        ));
    }

    #[tokio::test]
    async fn load_rejects_another_process_flavor_without_fallback() {
        let original = frozen(0x32);
        let other = frozen(0x33);
        let plan = compile(&original);
        let catalog = Arc::new(StubCatalog::default());
        let (installer, loader) = bridges(catalog);
        installer
            .install(&original, &plan)
            .await
            .expect("original pair installs");

        let ids = PlanFlavorRevisionIds::new(plan.id(), original.revision().id());
        assert!(matches!(
            loader.load_exact(ids, other).await,
            Err(PlanFlavorRevisionBridgeError::RegistryFlavorMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn load_rejects_bytes_whose_checked_identity_differs_from_the_lookup_key() {
        let registry = frozen(0x34);
        let plan = compile(&registry);
        let plan_bytes = RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan))
                .expect("checked plan serializes"),
        )
        .expect("checked plan bytes are non-empty");
        let flavor_bytes = RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&RecordedWorkerFlavorRevisionV1::from(registry.revision()))
                .expect("checked flavor serializes"),
        )
        .expect("checked flavor bytes are non-empty");
        let forged_plan_id = ExecutablePlanRevisionId::from_bytes([0xee; 32]);
        let record = PlanFlavorRevisionRecord::graph_v1_json(
            forged_plan_id,
            plan_bytes,
            WorkerFlavorRevisionRecord::v1_json(registry.revision().id(), flavor_bytes),
        );
        let catalog = Arc::new(StubCatalog::default());
        catalog.replace(record);
        let loader = PlanFlavorRevisionLoader::new(catalog);
        let ids = PlanFlavorRevisionIds::new(forged_plan_id, registry.revision().id());

        assert!(matches!(
            loader.load_exact(ids, registry).await,
            Err(PlanFlavorRevisionBridgeError::PlanIdentityMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn load_rejects_corrupt_flavor_envelope_without_exposing_bytes() {
        let registry = frozen(0x35);
        let plan = compile(&registry);
        let plan_bytes = RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan))
                .expect("checked plan serializes"),
        )
        .expect("checked plan bytes are non-empty");
        let mut corrupt_flavor =
            serde_json::to_value(RecordedWorkerFlavorRevisionV1::from(registry.revision()))
                .expect("checked flavor serializes");
        corrupt_flavor
            .as_object_mut()
            .expect("recorded flavor is a JSON object")
            .insert(
                "credential_secret_must_not_appear".to_owned(),
                serde_json::Value::String("sensitive-value-must-not-appear".to_owned()),
            );
        let corrupt_canary =
            serde_json::to_vec(&corrupt_flavor).expect("corrupt flavor fixture serializes");
        let record = PlanFlavorRevisionRecord::graph_v1_json(
            plan.id(),
            plan_bytes,
            WorkerFlavorRevisionRecord::v1_json(
                registry.revision().id(),
                RevisionRecordBytes::try_from_vec(corrupt_canary)
                    .expect("corrupt fixture remains non-empty"),
            ),
        );
        let catalog = Arc::new(StubCatalog::default());
        catalog.replace(record);
        let loader = PlanFlavorRevisionLoader::new(catalog);
        let ids = PlanFlavorRevisionIds::new(plan.id(), registry.revision().id());

        let error = loader
            .load_exact(ids, registry)
            .await
            .expect_err("corrupt flavor must fail closed");
        assert!(matches!(
            error,
            PlanFlavorRevisionBridgeError::FlavorRecordDecode
        ));
        let diagnostic = format!("{error} {error:?}");
        assert!(!diagnostic.contains("credential_secret_must_not_appear"));
        assert!(!diagnostic.contains("sensitive-value-must-not-appear"));
    }

    #[test]
    fn loaders_do_not_expose_catalog_or_reference_authority_in_debug() {
        let catalog = Arc::new(StubCatalog::default());
        let (installer, loader) = bridges(catalog);
        assert_eq!(format!("{loader:?}"), "PlanFlavorRevisionLoader { .. }");
        assert_eq!(
            format!("{installer:?}"),
            "PlanFlavorRevisionInstaller { .. }"
        );
    }

    #[test]
    fn exact_pair_ids_remain_typed() {
        let plan = ExecutablePlanRevisionId::from_bytes([0x51; 32]);
        let flavor = WorkerFlavorRevisionId::from_bytes([0x52; 32]);
        let ids = PlanFlavorRevisionIds::new(plan, flavor);
        assert_eq!(ids.plan(), plan);
        assert_eq!(ids.worker_flavor(), flavor);
    }

    #[test]
    fn telemetry_distinguishes_safe_retry_from_unknown_commit_outcome() {
        let unavailable = PlanFlavorRevisionBridgeError::from(RevisionCatalogError::Unavailable);
        let outcome_unknown =
            PlanFlavorRevisionBridgeError::from(RevisionCatalogError::OutcomeUnknown);

        assert_eq!(unavailable.code(), "REVISION_CATALOG:UNAVAILABLE");
        assert_eq!(outcome_unknown.code(), "REVISION_CATALOG:OUTCOME_UNKNOWN");
        assert_ne!(unavailable.code(), outcome_unknown.code());
    }
}
