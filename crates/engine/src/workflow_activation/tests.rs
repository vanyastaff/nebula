use std::{future::Future, pin::Pin};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, accessor::SystemClock, node_key};
use nebula_plugin::{Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_schema::ValidSchema;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{PlanFlavorCatalog, dto::WorkflowRecord};
use nebula_workflow::{NodeDefinition, WorkflowBuilder};

use super::*;

#[derive(Debug)]
struct CountingCatalog {
    inner: nebula_storage::InMemoryPlanFlavorCatalog,
    calls: std::sync::atomic::AtomicUsize,
    failure: Option<nebula_storage_port::RevisionCatalogError>,
}

#[async_trait::async_trait]
impl nebula_storage_port::PlanFlavorCatalogWriter for CountingCatalog {
    async fn insert(
        &self,
        record: &nebula_storage_port::PlanFlavorRevisionRecord,
    ) -> Result<nebula_storage_port::RevisionInsertOutcome, nebula_storage_port::RevisionCatalogError>
    {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.failure == Some(nebula_storage_port::RevisionCatalogError::Unavailable) {
            return Err(nebula_storage_port::RevisionCatalogError::Unavailable);
        }
        let result = self.inner.insert(record).await?;
        self.failure.map_or(Ok(result), Err)
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-07-01T12:34:56.123456789Z".parse().unwrap()
    }
    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

#[derive(Debug, Clone, Copy)]
enum PublicationFault {
    BeforeCommit,
    AfterCommit,
    AfterLaterActivation,
    CasLoss,
}

#[derive(Debug)]
struct PublicationFaultStore {
    inner: Arc<InMemoryWorkflowStore>,
    fault: PublicationFault,
    later_service: WorkflowActivationService,
}

#[async_trait::async_trait]
impl WorkflowStore for PublicationFaultStore {
    async fn publish_activated_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: u64,
    ) -> Result<(), WorkflowPublicationError> {
        if matches!(self.fault, PublicationFault::BeforeCommit) {
            return Err(WorkflowPublicationError::OutcomeUnknown);
        }
        if matches!(self.fault, PublicationFault::CasLoss) {
            let mut winner = self.inner.get(scope, &row.id).await?.unwrap();
            winner.version += 1;
            self.inner.update(scope, winner, expected_version).await?;
        }
        self.inner
            .publish_activated_version(scope, row, version.clone(), expected_version)
            .await?;
        if matches!(self.fault, PublicationFault::AfterLaterActivation) {
            let definition: WorkflowDefinition =
                serde_json::from_slice(&serde_json::to_vec(&version.definition).unwrap()).unwrap();
            self.later_service
                .activate(scope, definition.id, expected_version + 1, definition)
                .await
                .unwrap();
        }
        Err(WorkflowPublicationError::OutcomeUnknown)
    }
    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        self.inner.create(scope, record).await
    }
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        self.inner.get(scope, id).await
    }
    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        self.inner.get_by_slug(scope, slug).await
    }
    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected: u64,
    ) -> Result<(), StorageError> {
        self.inner.update(scope, record, expected).await
    }
    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected: Option<u64>,
    ) -> Result<(), StorageError> {
        self.inner
            .save_with_published_version(scope, row, version, expected)
            .await
    }
    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        self.inner.soft_delete(scope, id).await
    }
    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        self.inner.list(scope).await
    }
    async fn count(&self, scope: &Scope) -> Result<u64, StorageError> {
        self.inner.count(scope).await
    }
    async fn is_reachable(&self) -> Result<(), StorageError> {
        self.inner.is_reachable().await
    }
}

#[derive(Debug, Clone, Copy)]
enum ReadFault {
    Missing,
    Unavailable,
    Definition,
    Published,
    Pinned,
    Activation,
}

#[derive(Debug)]
struct VersionFaultStore {
    inner: Arc<InMemoryWorkflowVersionStore>,
    fault: ReadFault,
}

#[async_trait::async_trait]
impl WorkflowVersionStore for VersionFaultStore {
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        self.inner.create(scope, record).await
    }
    async fn get(
        &self,
        scope: &Scope,
        id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        match self.fault {
            ReadFault::Missing => return Ok(None),
            ReadFault::Unavailable => {
                return Err(StorageError::Connection("secret-driver-canary".into()));
            },
            _ => {},
        }
        let mut version = self.inner.get(scope, id, number).await?.unwrap();
        match self.fault {
            ReadFault::Definition => version.definition["name"] = "different".into(),
            ReadFault::Published => version.published = !version.published,
            ReadFault::Pinned => version.pinned = !version.pinned,
            ReadFault::Activation => {
                version.activation = Some(WorkflowActivation::new(
                    WorkflowVersionId::new(),
                    version.activation.unwrap().revisions(),
                ));
            },
            ReadFault::Missing | ReadFault::Unavailable => unreachable!(),
        }
        Ok(Some(version))
    }
    async fn get_published(
        &self,
        _: &Scope,
        _: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        panic!("reconciliation must never select latest")
    }
    async fn list(&self, _: &Scope, _: &str) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        panic!("reconciliation must never scan versions")
    }
}

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
        Box::pin(async { panic!("activation must not instantiate an action") })
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
                ActionKey::new("activation.run").unwrap(),
                "Run",
                "fixture",
            )
            .with_kind(ActionKind::Stateless)
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
            .with_schema(ValidSchema::empty())
            .with_output_schema(ValidSchema::empty()),
            dependencies: Dependencies::new(),
        })]
    }
}

fn registry() -> Arc<FrozenPluginRegistry> {
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin {
                manifest: PluginManifest::builder("activation", "Activation")
                    .build()
                    .unwrap(),
            })
            .unwrap(),
        ))
        .unwrap();
    Arc::new(
        plugins
            .freeze(
                ArtifactSetDigest::from_bytes([0x31; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    )
}

struct Fixture {
    executions: InMemoryExecutionStore,
    workflows: Arc<InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
    definition: WorkflowDefinition,
    scope: Scope,
}

impl Fixture {
    async fn new() -> Self {
        let executions = InMemoryExecutionStore::new();
        let versions = Arc::new(InMemoryWorkflowVersionStore::new());
        let workflows = Arc::new(InMemoryWorkflowStore::new_with_versions(
            &versions,
            &executions,
        ));
        let scope = Scope::new("workspace", "organization");
        let definition = WorkflowBuilder::new("secret-definition-canary")
            .add_node(NodeDefinition::new(node_key!("run"), "Run", "activation", "run").unwrap())
            .build()
            .unwrap();
        workflows
            .create(
                &scope,
                WorkflowRecord {
                    id: definition.id.to_string(),
                    scope: scope.clone(),
                    version: 1,
                    slug: "preserved-slug".into(),
                    deleted: false,
                },
            )
            .await
            .unwrap();
        Self {
            executions,
            workflows,
            versions,
            definition,
            scope,
        }
    }

    fn service(&self) -> WorkflowActivationService {
        WorkflowActivationService::new(
            self.workflows.clone(),
            self.versions.clone(),
            registry(),
            PlanFlavorRevisionInstaller::new(Arc::new(self.executions.plan_flavor_catalog())),
            Arc::new(SystemClock),
        )
    }

    fn fault_service(
        &self,
        fault: PublicationFault,
        versions: Arc<dyn WorkflowVersionStore>,
    ) -> WorkflowActivationService {
        WorkflowActivationService::new(
            Arc::new(PublicationFaultStore {
                inner: self.workflows.clone(),
                fault,
                later_service: self.service(),
            }),
            versions,
            registry(),
            PlanFlavorRevisionInstaller::new(Arc::new(self.executions.plan_flavor_catalog())),
            Arc::new(SystemClock),
        )
    }
}

#[tokio::test]
async fn workflow_activation_compiles_installs_and_returns_exact_publication() {
    let fixture = Fixture::new().await;
    let receipt = fixture
        .service()
        .activate(
            &fixture.scope,
            fixture.definition.id,
            1,
            fixture.definition.clone(),
        )
        .await
        .unwrap();
    let stored = fixture
        .versions
        .get(&fixture.scope, &fixture.definition.id.to_string(), 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.version(), &stored);
    assert!(stored.published);
    assert!(!stored.pinned);
    let activation = stored.activation.unwrap();
    let pair = fixture
        .executions
        .plan_flavor_catalog()
        .load_exact(activation.revisions())
        .await
        .unwrap();
    assert_eq!(pair.ids(), activation.revisions());
    let row = fixture
        .workflows
        .get(&fixture.scope, &fixture.definition.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.version, 2);
    assert_eq!(row.slug, "preserved-slug");
    assert!(!format!("{receipt:?}").contains("secret-definition-canary"));
}

#[tokio::test]
async fn workflow_activation_lost_ack_returns_original_even_after_later_activation() {
    for fault in [
        PublicationFault::AfterCommit,
        PublicationFault::AfterLaterActivation,
    ] {
        let fixture = Fixture::new().await;
        let receipt = fixture
            .fault_service(fault, fixture.versions.clone())
            .activate(
                &fixture.scope,
                fixture.definition.id,
                1,
                fixture.definition.clone(),
            )
            .await
            .unwrap();
        let original = fixture
            .versions
            .get(&fixture.scope, &fixture.definition.id.to_string(), 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.version(), &original);
        if matches!(fault, PublicationFault::AfterLaterActivation) {
            let later = fixture
                .versions
                .get(&fixture.scope, &fixture.definition.id.to_string(), 3)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(original.activation, later.activation);
        }
    }
}

#[tokio::test]
async fn workflow_activation_lost_ack_requires_whole_original_version() {
    for fault in [
        ReadFault::Missing,
        ReadFault::Unavailable,
        ReadFault::Definition,
        ReadFault::Published,
        ReadFault::Pinned,
        ReadFault::Activation,
    ] {
        let fixture = Fixture::new().await;
        let versions = Arc::new(VersionFaultStore {
            inner: fixture.versions.clone(),
            fault,
        });
        let error = fixture
            .fault_service(PublicationFault::AfterCommit, versions)
            .activate(
                &fixture.scope,
                fixture.definition.id,
                1,
                fixture.definition.clone(),
            )
            .await
            .unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("secret-"));
        let WorkflowActivationError::PublicationIndeterminate(attempt) = error else {
            panic!("expected indeterminate {fault:?}")
        };
        assert_eq!(attempt.scope(), &fixture.scope);
        assert_eq!(attempt.workflow_id(), fixture.definition.id);
        assert_eq!(attempt.number(), 2);
        let rows = fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "must not publish a fresh identity on an uncertain read"
        );
        assert_eq!(Some(attempt.activation()), rows[0].activation);
    }
}

#[tokio::test]
async fn workflow_activation_unknown_without_commit_remains_indeterminate() {
    let fixture = Fixture::new().await;
    let error = fixture
        .fault_service(PublicationFault::BeforeCommit, fixture.versions.clone())
        .activate(
            &fixture.scope,
            fixture.definition.id,
            1,
            fixture.definition.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WorkflowActivationError::PublicationIndeterminate(_)
    ));
    assert!(
        fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn workflow_activation_cas_loser_does_not_publish_a_version() {
    let fixture = Fixture::new().await;
    let error = fixture
        .fault_service(PublicationFault::CasLoss, fixture.versions.clone())
        .activate(
            &fixture.scope,
            fixture.definition.id,
            1,
            fixture.definition.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowActivationError::CasConflict));
    assert!(
        fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture
            .workflows
            .get(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );
}

#[tokio::test]
async fn workflow_activation_rejects_definition_identity_before_installation() {
    let fixture = Fixture::new().await;
    let catalog = Arc::new(CountingCatalog {
        inner: fixture.executions.plan_flavor_catalog(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        failure: None,
    });
    let service = WorkflowActivationService::new(
        fixture.workflows.clone(),
        fixture.versions.clone(),
        registry(),
        PlanFlavorRevisionInstaller::new(catalog.clone()),
        Arc::new(FixedClock),
    );
    let mut wrong = fixture.definition.clone();
    wrong.id = WorkflowId::new();
    let error = service
        .activate(&fixture.scope, fixture.definition.id, 1, wrong)
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowActivationError::InvalidDefinition));
    assert_eq!(catalog.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn workflow_activation_scope_cas_and_version_bounds_reject_before_installation() {
    for (stored_version, expected_version, wrong_scope) in [
        (1, 0, false),
        (1, 1, true),
        (u64::from(u32::MAX), u64::from(u32::MAX), false),
        (u64::MAX, u64::MAX, false),
    ] {
        let fixture = Fixture::new().await;
        let mut row = fixture
            .workflows
            .get(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .unwrap();
        row.version = stored_version;
        fixture
            .workflows
            .update(&fixture.scope, row, 1)
            .await
            .unwrap();
        let catalog = Arc::new(CountingCatalog {
            inner: fixture.executions.plan_flavor_catalog(),
            calls: std::sync::atomic::AtomicUsize::new(0),
            failure: None,
        });
        let service = WorkflowActivationService::new(
            fixture.workflows.clone(),
            fixture.versions.clone(),
            registry(),
            PlanFlavorRevisionInstaller::new(catalog.clone()),
            Arc::new(FixedClock),
        );
        let scope = if wrong_scope {
            Scope::new("another-workspace", "organization")
        } else {
            fixture.scope.clone()
        };
        let error = service
            .activate(
                &scope,
                fixture.definition.id,
                expected_version,
                fixture.definition.clone(),
            )
            .await
            .unwrap_err();
        if wrong_scope {
            assert!(matches!(error, WorkflowActivationError::MissingWorkflow));
        } else if expected_version != stored_version {
            assert!(matches!(error, WorkflowActivationError::CasConflict));
        } else {
            assert!(matches!(error, WorkflowActivationError::InvalidDefinition));
        }
        assert_eq!(catalog.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            fixture
                .versions
                .list(&fixture.scope, &fixture.definition.id.to_string())
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn workflow_activation_catalog_failure_or_unknown_never_publishes() {
    for failure in [
        nebula_storage_port::RevisionCatalogError::Unavailable,
        nebula_storage_port::RevisionCatalogError::OutcomeUnknown,
    ] {
        let fixture = Fixture::new().await;
        let catalog = Arc::new(CountingCatalog {
            inner: fixture.executions.plan_flavor_catalog(),
            calls: std::sync::atomic::AtomicUsize::new(0),
            failure: Some(failure),
        });
        let service = WorkflowActivationService::new(
            fixture.workflows.clone(),
            fixture.versions.clone(),
            registry(),
            PlanFlavorRevisionInstaller::new(catalog.clone()),
            Arc::new(FixedClock),
        );
        let error = service
            .activate(
                &fixture.scope,
                fixture.definition.id,
                1,
                fixture.definition.clone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            (&failure, error),
            (
                nebula_storage_port::RevisionCatalogError::Unavailable,
                WorkflowActivationError::BackendUnavailable
            ) | (
                nebula_storage_port::RevisionCatalogError::OutcomeUnknown,
                WorkflowActivationError::RevisionInstallationIndeterminate
            )
        ));
        assert_eq!(catalog.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            fixture
                .versions
                .list(&fixture.scope, &fixture.definition.id.to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fixture
                .workflows
                .get(&fixture.scope, &fixture.definition.id.to_string())
                .await
                .unwrap()
                .unwrap()
                .version,
            1
        );
    }
}

#[tokio::test]
async fn workflow_activation_timestamp_and_diagnostics_are_preserved_without_debug_payloads() {
    let fixture = Fixture::new().await;
    let mut service = fixture.service();
    service.clock = Arc::new(FixedClock);
    let receipt = service
        .activate(
            &fixture.scope,
            fixture.definition.id,
            1,
            fixture.definition.clone(),
        )
        .await
        .unwrap();
    let published: WorkflowDefinition =
        serde_json::from_slice(&serde_json::to_vec(&receipt.version().definition).unwrap())
            .unwrap();
    assert_eq!(published.updated_at, FixedClock.now());
    let mut invalid = fixture.definition.clone();
    invalid.nodes[0].action_key = ActionKey::new("missing").unwrap();
    let error = service
        .activate(&fixture.scope, fixture.definition.id, 2, invalid)
        .await
        .unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("secret-definition-canary"));
    let WorkflowActivationError::Compilation(diagnostics) = error else {
        panic!("expected compiler diagnostics")
    };
    assert!(!diagnostics.diagnostics().is_empty());
    assert_eq!(
        fixture
            .versions
            .list(&fixture.scope, &fixture.definition.id.to_string())
            .await
            .unwrap()
            .len(),
        1
    );
}
