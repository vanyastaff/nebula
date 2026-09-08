use std::{future::Future, pin::Pin};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, OrgId, WorkspaceId, node_key};
use nebula_plugin::{Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_schema::ValidSchema;
use nebula_storage::{
    InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowStore,
    InMemoryWorkflowVersionStore, inmem::InMemoryStartAcceptanceStore,
};
use nebula_storage_port::{
    dto::WorkflowRecord,
    store::{ControlQueue, StartReservationMaintenance, WorkflowStore, WorkflowVersionStore},
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder, WorkflowDefinition};

use super::*;

#[derive(Debug, Clone, Copy)]
enum CommitFault {
    None,
    UnknownBeforeOnce,
    UnknownAfterOnce,
    AlwaysUnknown,
    RejectAfterUnknown,
}
#[derive(Debug, Clone, Copy)]
enum BundleFault {
    None,
    Missing,
    Corrupt,
    WrongScope,
}

#[derive(Debug)]
struct FaultStartStore {
    inner: Arc<InMemoryStartAcceptanceStore>,
    commit: CommitFault,
    bundle: BundleFault,
    attempts: parking_lot::Mutex<Vec<(Value, ControlMsg, ContractBundleRecord)>>,
}
impl FaultStartStore {
    fn new(
        inner: Arc<InMemoryStartAcceptanceStore>,
        commit: CommitFault,
        bundle: BundleFault,
    ) -> Self {
        Self {
            inner,
            commit,
            bundle,
            attempts: parking_lot::Mutex::new(Vec::new()),
        }
    }
}
#[async_trait::async_trait]
impl StartAcceptanceStore for FaultStartStore {
    async fn lookup_trigger_start(
        &self,
        scope: &Scope,
        key: &TriggerStartKey<'_>,
    ) -> Result<Option<String>, nebula_storage_port::StorageError> {
        self.inner.lookup_trigger_start(scope, key).await
    }
    async fn materialize_start(
        &self,
        start: &MaterializedStart<'_>,
    ) -> Result<StartMaterialization, StartMaterializationError> {
        let call = {
            let mut attempts = self.attempts.lock();
            attempts.push((
                start.execution().initial_state.clone(),
                start.command().clone(),
                start.bundle().clone(),
            ));
            attempts.len()
        };
        match self.commit {
            CommitFault::AlwaysUnknown => return Err(StartMaterializationError::OutcomeUnknown),
            CommitFault::UnknownBeforeOnce | CommitFault::RejectAfterUnknown if call == 1 => {
                return Err(StartMaterializationError::OutcomeUnknown);
            },
            CommitFault::RejectAfterUnknown => {
                return Ok(StartMaterialization::RevisionRejected(
                    StartRevisionRejection::PairNotAdmitted,
                ));
            },
            _ => {},
        }
        let result = self.inner.materialize_start(start).await?;
        if matches!(self.commit, CommitFault::UnknownAfterOnce) && call == 1 {
            return Err(StartMaterializationError::OutcomeUnknown);
        }
        Ok(result)
    }
    async fn lookup_start(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<nebula_storage_port::dto::StartReservation>, nebula_storage_port::StorageError>
    {
        self.inner.lookup_start(scope, key).await
    }
    async fn read_contract_bundle(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<
        Option<nebula_storage_port::dto::StoredContractBundle>,
        nebula_storage_port::StorageError,
    > {
        if matches!(self.bundle, BundleFault::Missing) {
            return Ok(None);
        }
        let stored = self.inner.read_contract_bundle(scope, execution_id).await?;
        Ok(stored.map(|stored| {
            let stored_scope = if matches!(self.bundle, BundleFault::WrongScope) {
                Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string())
            } else {
                stored.scope().clone()
            };
            let record = if matches!(self.bundle, BundleFault::Corrupt) {
                ContractBundleRecord::v1_json(
                    stored.record().identity(),
                    b"{\"private\":\"secret-canary\"}".to_vec(),
                )
                .unwrap()
            } else {
                stored.record().clone()
            };
            nebula_storage_port::dto::StoredContractBundle::new(
                stored_scope,
                stored.execution_id().to_owned(),
                record,
            )
        }))
    }
}
use crate::{PlanFlavorRevisionInstaller, WorkflowActivationService};

struct FixtureFactory {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}
impl ActionFactory for FixtureFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
    fn instantiate<'a>(
        &'a self,
        _: &'a NodeDefinition,
        _: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async { panic!("start admission must never instantiate an action") })
    }
}
#[derive(Debug)]
struct FixturePlugin {
    manifest: PluginManifest,
}
impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![Arc::new(FixtureFactory {
            metadata: ActionMetadata::new(
                ActionKey::new("start.run").unwrap(),
                "Run",
                "start fixture",
            )
            .with_kind(ActionKind::Stateless)
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
            .with_schema(ValidSchema::empty())
            .with_output_schema(ValidSchema::empty()),
            dependencies: Dependencies::new(),
        })]
    }
}
fn frozen() -> Arc<FrozenPluginRegistry> {
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin {
                manifest: PluginManifest::builder("start", "Start").build().unwrap(),
            })
            .unwrap(),
        ))
        .unwrap();
    Arc::new(
        plugins
            .freeze(
                ArtifactSetDigest::from_bytes([0x57; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    )
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-08-09T10:11:12.123456789Z".parse().unwrap()
    }
    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}
struct Fixture {
    executions: Arc<InMemoryExecutionStore>,
    starts: Arc<InMemoryStartAcceptanceStore>,
    workflows: Arc<InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
    registry: Arc<FrozenPluginRegistry>,
    definition: WorkflowDefinition,
    scope: Scope,
}
impl Fixture {
    async fn new() -> Self {
        let executions = Arc::new(InMemoryExecutionStore::new());
        let starts = Arc::new(InMemoryStartAcceptanceStore::new(&executions));
        let versions = Arc::new(InMemoryWorkflowVersionStore::new());
        let workflows = Arc::new(InMemoryWorkflowStore::new_with_versions(
            &versions,
            &executions,
        ));
        let definition = WorkflowBuilder::new("Start fixture")
            .add_node(NodeDefinition::new(node_key!("run"), "Run", "start", "run").unwrap())
            .build()
            .unwrap();
        let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
        workflows
            .create(
                &scope,
                WorkflowRecord {
                    id: definition.id.to_string(),
                    scope: scope.clone(),
                    version: 1,
                    slug: "start-fixture".into(),
                    deleted: false,
                },
            )
            .await
            .unwrap();
        let fixture = Self {
            executions,
            starts,
            workflows,
            versions,
            registry: frozen(),
            definition,
            scope,
        };
        fixture
            .activation()
            .activate(
                &fixture.scope,
                fixture.definition.id,
                1,
                fixture.definition.clone(),
            )
            .await
            .unwrap();
        fixture
    }
    fn activation(&self) -> WorkflowActivationService {
        WorkflowActivationService::new(
            self.workflows.clone(),
            self.versions.clone(),
            self.registry.clone(),
            PlanFlavorRevisionInstaller::new(Arc::new(self.executions.plan_flavor_catalog())),
            Arc::new(FixedClock),
        )
    }
    fn service(&self) -> WorkflowStartService {
        WorkflowStartService::new(
            WorkflowStores {
                workflow: self.workflows.clone(),
                versions: self.versions.clone(),
            },
            self.executions.clone(),
            self.starts.clone(),
            PlanFlavorRevisionLoader::new(Arc::new(self.executions.plan_flavor_catalog())),
            self.registry.clone(),
            Arc::new(FixedClock),
            ExecutionBudget::default(),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn runtime_start_atomically_materializes_real_bundle_state_and_command() {
    let fixture = Fixture::new().await;
    let receipt = fixture
        .service()
        .start(
            &fixture.scope,
            fixture.definition.id,
            Some(serde_json::json!({"private":"input-canary"})),
            Some("caller-key"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(receipt.disposition(), WorkflowStartDisposition::Accepted);
    assert_eq!(receipt.state().created_at, FixedClock.now());
    assert_eq!(receipt.state().updated_at, FixedClock.now());
    assert!(receipt.state().node_states.is_empty());
    assert_eq!(receipt.state().workflow_version_number, Some(2));
    assert_eq!(
        receipt.state().executable_plan_revision_id,
        Some(receipt.bundle().executable_plan_revision_id())
    );
    assert!(receipt.state().budget.is_some());
    let stored = fixture
        .starts
        .read_contract_bundle(&fixture.scope, &receipt.state().execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.record().identity().bundle_id(),
        receipt.bundle().bundle_id()
    );
    let queue = InMemoryControlQueue::new(&fixture.executions);
    let commands = queue.claim_pending(&[3; 16], 10).await.unwrap();
    assert_eq!(commands.len(), 1);
    assert!(!format!("{receipt:?}").contains("input-canary"));
}

#[tokio::test]
async fn runtime_start_unkeyed_repeats_are_distinct_and_make_no_reservation() {
    let fixture = Fixture::new().await;
    let service = fixture.service();
    let first = service
        .start(&fixture.scope, fixture.definition.id, None, None, None)
        .await
        .unwrap();
    let second = service
        .start(&fixture.scope, fixture.definition.id, None, None, None)
        .await
        .unwrap();
    assert_ne!(first.state().execution_id, second.state().execution_id);
    assert_ne!(first.bundle().bundle_id(), second.bundle().bundle_id());
    assert_eq!(first.state().workflow_input, Some(Value::Null));
    assert_eq!(
        fixture
            .starts
            .evict_reservations_older_than(std::time::Duration::ZERO)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        InMemoryControlQueue::new(&fixture.executions)
            .claim_pending(&[4; 16], 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn runtime_start_replays_before_republish_drain_and_deleted_workflow_checks() {
    use nebula_storage_port::{PlanFlavorCatalogAdmin, PlanFlavorRevisionTarget};
    let fixture = Fixture::new().await;
    let service = fixture.service();
    let first = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            None,
            Some("original"),
            None,
        )
        .await
        .unwrap();
    fixture
        .activation()
        .activate(
            &fixture.scope,
            fixture.definition.id,
            2,
            fixture.definition.clone(),
        )
        .await
        .unwrap();
    let after_publish = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            Some(Value::Null),
            Some(" original "),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        after_publish.state().execution_id,
        first.state().execution_id
    );
    fixture
        .executions
        .plan_flavor_catalog()
        .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
            first.bundle().executable_plan_revision_id(),
        ))
        .await
        .unwrap();
    let after_drain = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            None,
            Some("original"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(after_drain.bundle(), first.bundle());
    fixture
        .workflows
        .soft_delete(&fixture.scope, &fixture.definition.id.to_string())
        .await
        .unwrap();
    let after_delete = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            None,
            Some("original"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        after_delete.disposition(),
        WorkflowStartDisposition::Replayed
    );
    assert_eq!(after_delete.state().created_at, first.state().created_at);
    assert_eq!(
        after_delete.state().execution_id,
        first.state().execution_id
    );
    assert_eq!(
        InMemoryControlQueue::new(&fixture.executions)
            .claim_pending(&[4; 16], 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn runtime_start_changed_input_does_not_materialize_again() {
    let fixture = Fixture::new().await;
    let service = fixture.service();
    service
        .start(
            &fixture.scope,
            fixture.definition.id,
            Some(serde_json::json!({"a":1})),
            Some("key"),
            None,
        )
        .await
        .unwrap();
    let error = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            Some(serde_json::json!({"a":2})),
            Some("key"),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowStartError::FingerprintMismatch));
    assert_eq!(
        InMemoryControlQueue::new(&fixture.executions)
            .claim_pending(&[4; 16], 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn runtime_start_unknown_commit_reuses_entire_original_attempt() {
    for fault in [
        CommitFault::UnknownBeforeOnce,
        CommitFault::UnknownAfterOnce,
    ] {
        for key in [None, Some("original-key")] {
            let fixture = Fixture::new().await;
            let starts = Arc::new(FaultStartStore::new(
                fixture.starts.clone(),
                fault,
                BundleFault::None,
            ));
            let mut service = fixture.service();
            service.starts = starts.clone();
            let trace = W3cTraceContext::from_traceparent_str(
                "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01",
            )
            .unwrap();
            let receipt = service
                .start(
                    &fixture.scope,
                    fixture.definition.id,
                    Some(serde_json::json!({"private":"secret-canary"})),
                    key,
                    Some(trace),
                )
                .await
                .unwrap();
            {
                let attempts = starts.attempts.lock();
                assert_eq!(attempts.len(), 2);
                assert_eq!(
                    attempts[0].1.execution_id,
                    receipt.state().execution_id.to_string()
                );
                assert_eq!(attempts[0], attempts[1]);
            }
            assert_eq!(
                InMemoryControlQueue::new(&fixture.executions)
                    .claim_pending(&[4; 16], 10)
                    .await
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[tokio::test]
async fn runtime_start_uncertainty_stays_sticky_and_attempts_are_bounded() {
    for fault in [CommitFault::AlwaysUnknown, CommitFault::RejectAfterUnknown] {
        let fixture = Fixture::new().await;
        let starts = Arc::new(FaultStartStore::new(
            fixture.starts.clone(),
            fault,
            BundleFault::None,
        ));
        let mut service = fixture.service();
        service.starts = starts.clone();
        let error = service
            .start(
                &fixture.scope,
                fixture.definition.id,
                Some(serde_json::json!({"private":"secret-canary"})),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
        let WorkflowStartError::MaterializationIndeterminate(original) = error else {
            panic!("unknown acknowledgement must not become a definitive rejection")
        };
        {
            let attempts = starts.attempts.lock();
            assert_eq!(attempts.len(), 2);
            assert_eq!(attempts[0], attempts[1]);
            assert_eq!(
                original.execution_id().to_string(),
                attempts[0].1.execution_id
            );
            assert_eq!(original.bundle_id(), attempts[0].2.identity().bundle_id());
        }
        assert!(
            InMemoryControlQueue::new(&fixture.executions)
                .claim_pending(&[4; 16], 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn runtime_start_receipt_failure_keeps_known_accepted_identity() {
    for bundle_fault in [
        BundleFault::Missing,
        BundleFault::Corrupt,
        BundleFault::WrongScope,
    ] {
        let fixture = Fixture::new().await;
        let starts = Arc::new(FaultStartStore::new(
            fixture.starts.clone(),
            CommitFault::None,
            bundle_fault,
        ));
        let mut service = fixture.service();
        service.starts = starts.clone();
        let error = service
            .start(&fixture.scope, fixture.definition.id, None, None, None)
            .await
            .unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
        let WorkflowStartError::ReceiptUnavailable { execution_id } = error else {
            panic!("commit is known; only its receipt is unavailable")
        };
        assert_eq!(starts.attempts.lock().len(), 1);
        assert!(
            fixture
                .executions
                .get(&fixture.scope, &execution_id.to_string())
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn runtime_start_fingerprint_v2_is_canonical_and_preserves_input_distinctions() {
    let workflow = WorkflowId::new();
    let mut first: Value =
        serde_json::from_str(r#"{"outer":{"b":2,"a":1},"array":[1,2]}"#).unwrap();
    let mut reordered: Value =
        serde_json::from_str(r#"{"array":[1,2],"outer":{"a":1,"b":2}}"#).unwrap();
    let fingerprint = caller_intent_fingerprint(workflow, &mut first).unwrap();
    assert_eq!(fingerprint.version(), 2);
    assert_eq!(
        fingerprint,
        caller_intent_fingerprint(workflow, &mut reordered).unwrap()
    );
    reordered["array"] = serde_json::json!([2, 1]);
    assert_ne!(
        fingerprint,
        caller_intent_fingerprint(workflow, &mut reordered).unwrap()
    );
    assert_ne!(
        fingerprint,
        caller_intent_fingerprint(WorkflowId::new(), &mut first).unwrap()
    );
}

#[tokio::test]
async fn runtime_start_rejects_invalid_admission_without_submitting_a_transaction() {
    let fixture = Fixture::new().await;
    let starts = Arc::new(FaultStartStore::new(
        fixture.starts.clone(),
        CommitFault::None,
        BundleFault::None,
    ));
    let mut service = fixture.service();
    service.starts = starts.clone();
    for key in [String::new(), " ".to_owned(), "x".repeat(256)] {
        assert!(matches!(
            service
                .start(
                    &fixture.scope,
                    fixture.definition.id,
                    None,
                    Some(&key),
                    None
                )
                .await,
            Err(WorkflowStartError::InvalidKey)
        ));
    }
    assert!(matches!(
        service
            .start(
                &Scope::new("invalid", "invalid"),
                fixture.definition.id,
                None,
                None,
                None
            )
            .await,
        Err(WorkflowStartError::InvalidScope)
    ));
    let other_scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    assert!(matches!(
        service
            .start(&other_scope, fixture.definition.id, None, None, None)
            .await,
        Err(WorkflowStartError::MissingWorkflow)
    ));
    assert!(starts.attempts.lock().is_empty());
    assert!(
        InMemoryControlQueue::new(&fixture.executions)
            .claim_pending(&[5; 16], 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn activation_rejects_unsupported_recorded_semantics_before_publication() {
    for case in 0..4 {
        let fixture = Fixture::new().await;
        let mut definition = fixture.definition.clone();
        match case {
            0 => {
                definition
                    .variables
                    .insert("private".into(), Value::String("secret-canary".into()));
            },
            1 => definition.config.checkpointing.enabled = false,
            2 => definition.config.checkpointing.interval = Some(std::time::Duration::from_secs(1)),
            _ => definition.nodes[0].timeout = Some(std::time::Duration::from_secs(1)),
        }
        let before = fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .len();
        let error = fixture
            .activation()
            .activate(&fixture.scope, definition.id, 2, definition)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::WorkflowActivationError::UnsupportedRecordedSemantics
        ));
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
        assert_eq!(
            fixture
                .versions
                .list(&fixture.scope, &fixture.definition.id.to_string())
                .await
                .unwrap()
                .len(),
            before
        );
    }
}

#[tokio::test]
async fn trigger_start_replays_original_event_after_payload_and_activation_change() {
    let fixture = Fixture::new().await;
    let service = fixture.service();
    let key = TriggerStartKey::new("run", "delivery");
    let original = service
        .start_trigger(
            &fixture.scope,
            fixture.definition.id,
            serde_json::json!({"original":1}),
            key,
            None,
        )
        .await
        .unwrap();
    fixture
        .activation()
        .activate(
            &fixture.scope,
            fixture.definition.id,
            2,
            fixture.definition.clone(),
        )
        .await
        .unwrap();
    let replayed = service
        .start_trigger(
            &fixture.scope,
            fixture.definition.id,
            serde_json::json!({"changed":2}),
            key,
            None,
        )
        .await
        .unwrap();
    assert_eq!(replayed.state().execution_id, original.state().execution_id);
    assert_eq!(replayed.bundle(), original.bundle());
    assert_eq!(
        replayed.state().workflow_input,
        Some(serde_json::json!({"original":1}))
    );
    let caller = service
        .start(
            &fixture.scope,
            fixture.definition.id,
            None,
            Some("delivery"),
            None,
        )
        .await
        .unwrap();
    assert_ne!(caller.state().execution_id, original.state().execution_id);
    assert_eq!(
        InMemoryControlQueue::new(&fixture.executions)
            .claim_pending(&[7; 16], 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn trigger_start_unknown_ack_retries_only_the_original_materialization() {
    for fault in [
        CommitFault::UnknownBeforeOnce,
        CommitFault::UnknownAfterOnce,
    ] {
        let fixture = Fixture::new().await;
        let starts = Arc::new(FaultStartStore::new(
            fixture.starts.clone(),
            fault,
            BundleFault::None,
        ));
        let mut service = fixture.service();
        service.starts = starts.clone();
        let receipt = service
            .start_trigger(
                &fixture.scope,
                fixture.definition.id,
                Value::Null,
                TriggerStartKey::new("run", "event"),
                None,
            )
            .await
            .unwrap();
        {
            let attempts = starts.attempts.lock();
            assert_eq!(attempts.len(), 2);
            assert_eq!(
                attempts[0].1.execution_id,
                receipt.state().execution_id.to_string()
            );
            assert_eq!(attempts[0], attempts[1]);
        }
        assert_eq!(
            fixture
                .starts
                .lookup_trigger_start(&fixture.scope, &TriggerStartKey::new("run", "event"))
                .await
                .unwrap(),
            Some(receipt.state().execution_id.to_string())
        );
        assert_eq!(
            InMemoryControlQueue::new(&fixture.executions)
                .claim_pending(&[8; 16], 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn trigger_start_uncertain_or_unreadable_receipt_preserves_execution_identity() {
    for (commit, bundle) in [
        (CommitFault::AlwaysUnknown, BundleFault::None),
        (CommitFault::UnknownAfterOnce, BundleFault::Missing),
    ] {
        let fixture = Fixture::new().await;
        let starts = Arc::new(FaultStartStore::new(fixture.starts.clone(), commit, bundle));
        let mut service = fixture.service();
        service.starts = starts.clone();
        let error = service
            .start_trigger(
                &fixture.scope,
                fixture.definition.id,
                serde_json::json!({"private":"secret-canary"}),
                TriggerStartKey::new("run", "delivery"),
                None,
            )
            .await
            .unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
        let (execution_id, call_count) = match error {
            WorkflowStartError::MaterializationIndeterminate(original) => {
                assert!(matches!(commit, CommitFault::AlwaysUnknown));
                (original.execution_id(), 2)
            },
            WorkflowStartError::ReceiptUnavailable { execution_id } => {
                assert!(matches!(commit, CommitFault::UnknownAfterOnce));
                (execution_id, 2)
            },
            other => panic!("must preserve uncertain or accepted identity: {other:?}"),
        };
        let attempts = starts.attempts.lock();
        assert_eq!(attempts.len(), call_count);
        assert_eq!(attempts[0].1.execution_id, execution_id.to_string());
    }
}

#[tokio::test]
async fn emitter_uncertain_unkeyed_acceptance_forbids_automatic_reentry() {
    use nebula_action::{ExecutionEmitter, IdempotencyKey};
    for keyed in [false, true] {
        let fixture = Fixture::new().await;
        let starts = Arc::new(FaultStartStore::new(
            fixture.starts.clone(),
            CommitFault::AlwaysUnknown,
            BundleFault::None,
        ));
        let mut service = fixture.service();
        service.starts = starts.clone();
        let emitter = crate::DurableExecutionEmitter::new(
            Arc::new(service),
            fixture.definition.id,
            node_key!("run"),
            fixture.scope.clone(),
        );
        let error = emitter
            .emit(
                serde_json::json!({"private":"secret-canary"}),
                keyed.then(|| IdempotencyKey::new("event")),
            )
            .await
            .unwrap_err();
        assert_eq!(error.is_retryable(), keyed);
        let workflow_error = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<WorkflowStartError>())
            .expect("transport must retain the typed admission outcome");
        let WorkflowStartError::MaterializationIndeterminate(original) = workflow_error else {
            panic!("unknown materialization must retain its original attempt")
        };
        let attempts = starts.attempts.lock();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(
            original.execution_id().to_string(),
            attempts[0].1.execution_id
        );
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
    }
}
