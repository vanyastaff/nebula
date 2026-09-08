use super::*;
use nebula_storage_port::{
    dto::{
        ContractBundleRecord, MaterializedStart, StartReservation, StoredContractBundle,
        TriggerStartKey,
    },
    store::{StartAcceptanceStore, StartMaterialization, StartMaterializationError},
};

#[derive(Debug, Clone, Copy)]
enum BundleFault {
    Unavailable,
    Missing,
    WrongScope,
    WrongExecution,
    Malformed,
    Integrity,
    UnsupportedVersion,
    TenantIdentity,
    EnvelopeIdentity,
    WorkflowRevision,
    PluginSet,
    Credentials,
}

#[derive(Debug)]
struct FaultBundleReader {
    inner: Arc<dyn StartAcceptanceStore>,
    fault: BundleFault,
}

#[async_trait::async_trait]
impl StartAcceptanceStore for FaultBundleReader {
    async fn lookup_trigger_start(
        &self,
        scope: &Scope,
        key: &TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError> {
        self.inner.lookup_trigger_start(scope, key).await
    }
    async fn materialize_start(
        &self,
        start: &MaterializedStart<'_>,
    ) -> Result<StartMaterialization, StartMaterializationError> {
        self.inner.materialize_start(start).await
    }
    async fn lookup_start(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<StartReservation>, StorageError> {
        self.inner.lookup_start(scope, key).await
    }
    async fn read_contract_bundle(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<StoredContractBundle>, StorageError> {
        if matches!(self.fault, BundleFault::Unavailable) {
            return Err(StorageError::Timeout {
                operation: "contract read".into(),
                duration: Duration::from_secs(1),
            });
        }
        if matches!(self.fault, BundleFault::Missing) {
            return Ok(None);
        }
        let stored = self
            .inner
            .read_contract_bundle(scope, execution_id)
            .await?
            .unwrap();
        let mut stored_scope = stored.scope().clone();
        let mut stored_execution_id = stored.execution_id().to_owned();
        let identity = stored.record().identity();
        let mut wire: serde_json::Value = serde_json::from_slice(stored.record().bytes()).unwrap();
        match self.fault {
            BundleFault::Missing | BundleFault::Unavailable => unreachable!(),
            BundleFault::WrongScope => {
                stored_scope.org_id = nebula_core::OrgId::new().to_string();
            },
            BundleFault::WrongExecution => {
                stored_execution_id = ExecutionId::new().to_string();
            },
            BundleFault::Malformed => wire = serde_json::json!({"secret":"secret-canary"}),
            BundleFault::Integrity => {
                wire["plugin_set_id"] =
                    serde_json::json!(nebula_core::PluginSetId::from_bytes([0xFD; 32]));
            },
            BundleFault::UnsupportedVersion => wire["schema_version"] = serde_json::json!(2),
            BundleFault::EnvelopeIdentity => {
                wire["bundle_id"] =
                    serde_json::json!(nebula_core::ExecutionContractBundleId::new());
            },
            BundleFault::WorkflowRevision
            | BundleFault::PluginSet
            | BundleFault::Credentials
            | BundleFault::TenantIdentity => {
                let bundle: nebula_execution::ExecutionContractBundle =
                    serde_json::from_slice(stored.record().bytes()).unwrap();
                let workflow = if matches!(self.fault, BundleFault::WorkflowRevision) {
                    nebula_core::WorkflowVersionId::new()
                } else {
                    bundle.revisions().workflow()
                };
                let plugin_set = if matches!(self.fault, BundleFault::PluginSet) {
                    nebula_core::PluginSetId::from_bytes([0xFD; 32])
                } else {
                    bundle.plugin_set_id()
                };
                let credentials = if matches!(self.fault, BundleFault::Credentials) {
                    vec![nebula_core::CredentialId::new()]
                } else {
                    Vec::new()
                };
                let org_id = if matches!(self.fault, BundleFault::TenantIdentity) {
                    nebula_core::OrgId::new()
                } else {
                    bundle.org_id()
                };
                let replacement = nebula_execution::ExecutionContractBundle::new_graph_v1(
                    bundle.bundle_id(),
                    org_id,
                    bundle.workspace_id(),
                    bundle.executable_plan_revision_id(),
                    plugin_set,
                    nebula_execution::ExecutionRevisions::new(
                        workflow,
                        bundle.revisions().worker_flavor(),
                    ),
                    credentials,
                );
                wire = serde_json::to_value(replacement).unwrap();
            },
        }
        let record =
            ContractBundleRecord::v1_json(identity, serde_json::to_vec(&wire).unwrap()).unwrap();
        Ok(Some(StoredContractBundle::new(
            stored_scope,
            stored_execution_id,
            record,
        )))
    }
}

#[tokio::test]
async fn unavailable_bundle_read_defers_start_without_mutating_execution() {
    use crate::ControlDispatch;
    let runtime_registry = Arc::new(ActionRegistry::new());
    let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
    let stores = TestStores::new();
    let (execution_id, _) = install_snapshot_execution(&stores, &frozen).await;
    let before = stores.get_state(execution_id).await.unwrap();
    let (engine, _) = make_engine(runtime_registry);
    let bundles = Arc::new(FaultBundleReader {
        inner: Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            &stores.execution,
        )),
        fault: BundleFault::Unavailable,
    });
    let engine = Arc::new(
        engine
            .with_execution_stores(stores.execution_stores())
            .with_plan_flavor_runtime(
                Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                    stores.execution.plan_flavor_catalog(),
                ))),
                frozen,
                bundles,
            ),
    );
    let scope = crate::store_seam::single_tenant_scope();
    let error = engine
        .resume_execution(&scope, execution_id)
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::ContractBundleRead { .. }));
    assert_eq!(
        nebula_error::Classify::category(&error),
        nebula_error::ErrorCategory::Unavailable
    );
    let dispatch = crate::EngineControlDispatch::new(
        engine,
        stores.execution.clone(),
        Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(
            &stores.execution,
        )),
        "contract-test-control".to_owned(),
        Duration::from_secs(30),
    );
    assert!(matches!(
        dispatch.dispatch_start(&scope, execution_id).await,
        Err(crate::ControlDispatchError::Deferred(_))
    ));
    assert_eq!(instantiations.load(Ordering::SeqCst), 0);
    assert_eq!(stores.get_state(execution_id).await.unwrap(), before);
}

#[tokio::test]
async fn bundle_faults_reject_cold_warm_and_adopted_turns_before_instantiation() {
    for fault in [
        BundleFault::Missing,
        BundleFault::WrongScope,
        BundleFault::WrongExecution,
        BundleFault::Malformed,
        BundleFault::Integrity,
        BundleFault::UnsupportedVersion,
        BundleFault::TenantIdentity,
        BundleFault::EnvelopeIdentity,
        BundleFault::WorkflowRevision,
        BundleFault::PluginSet,
        BundleFault::Credentials,
    ] {
        for (warm, adopt) in [(false, false), (true, false), (true, true)] {
            let runtime_registry = Arc::new(ActionRegistry::new());
            let (frozen, instantiations) =
                snapshot_registry_counted(&runtime_registry, "must-not-run");
            let stores = TestStores::new();
            let (execution_id, _) = install_snapshot_execution(&stores, &frozen).await;
            if warm {
                let (_, encoded) = stores.get_state(execution_id).await.unwrap().unwrap();
                let mut state: ExecutionState =
                    serde_json::from_slice(&serde_json::to_vec(&encoded).unwrap()).unwrap();
                state.transition_status(ExecutionStatus::Running).unwrap();
                stores.replace_exact_state(state).await;
            }
            let before = stores.get_state(execution_id).await.unwrap();
            let bundles = Arc::new(FaultBundleReader {
                inner: Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                    &stores.execution,
                )),
                fault,
            });
            let (engine, _) = make_engine(runtime_registry);
            let engine = engine
                .with_execution_stores(stores.execution_stores())
                .with_plan_flavor_runtime(
                    Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                        stores.execution.plan_flavor_catalog(),
                    ))),
                    frozen,
                    bundles,
                );
            let scope = crate::store_seam::single_tenant_scope();
            let error = if adopt {
                let fence = stores
                    .execution
                    .acquire_lease(
                        &scope,
                        &execution_id.to_string(),
                        "handoff",
                        Duration::from_secs(30),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                engine
                    .resume_execution_leased(&scope, execution_id, fence)
                    .await
                    .unwrap_err()
            } else {
                engine
                    .resume_execution(&scope, execution_id)
                    .await
                    .unwrap_err()
            };
            assert!(
                match fault {
                    BundleFault::Missing => matches!(error, EngineError::MissingContractBundle),
                    BundleFault::Integrity | BundleFault::UnsupportedVersion =>
                        matches!(error, EngineError::ContractBundleIntegrity { .. }),
                    BundleFault::Credentials =>
                        matches!(error, EngineError::UnresolvedPlanBindings),
                    _ => matches!(error, EngineError::InvalidRecordedContract),
                },
                "unexpected error for {fault:?}: {error:?}"
            );
            assert!(!format!("{error:?} {error}").contains("secret-canary"));
            assert_eq!(instantiations.load(Ordering::SeqCst), 0, "{fault:?}");
            assert_eq!(
                stores.get_state(execution_id).await.unwrap(),
                before,
                "{fault:?}"
            );
            assert!(
                stores
                    .execution
                    .acquire_lease(
                        &scope,
                        &execution_id.to_string(),
                        "recovery",
                        Duration::from_secs(30)
                    )
                    .await
                    .unwrap()
                    .is_some(),
                "adopted preparation rejection must release its own lease"
            );
        }
    }
}

#[tokio::test]
async fn state_revision_pins_must_agree_with_the_immutable_bundle() {
    for change_flavor in [false, true] {
        let runtime_registry = Arc::new(ActionRegistry::new());
        let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
        let stores = TestStores::new();
        let (execution_id, _) = install_snapshot_execution(&stores, &frozen).await;
        let (_, encoded) = stores.get_state(execution_id).await.unwrap().unwrap();
        let mut state: ExecutionState =
            serde_json::from_slice(&serde_json::to_vec(&encoded).unwrap()).unwrap();
        if change_flavor {
            state.worker_flavor_revision_id =
                Some(nebula_core::WorkerFlavorRevisionId::from_bytes([0xFD; 32]));
        } else {
            state.executable_plan_revision_id = Some(
                nebula_core::ExecutablePlanRevisionId::from_bytes([0xFD; 32]),
            );
        }
        stores.replace_exact_state(state).await;
        let before = stores.get_state(execution_id).await.unwrap();
        let (engine, _) = make_engine(runtime_registry);
        let engine = engine
            .with_execution_stores(stores.execution_stores())
            .with_plan_flavor_runtime(
                Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                    stores.execution.plan_flavor_catalog(),
                ))),
                frozen,
                Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                    &stores.execution,
                )),
            );
        let error = engine
            .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::InvalidRecordedContract));
        assert_eq!(instantiations.load(Ordering::SeqCst), 0);
        assert_eq!(stores.get_state(execution_id).await.unwrap(), before);
    }
}
