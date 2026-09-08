//! Provider observations through real activation, start admission and durable turns.

use std::{future::Future, pin::Pin, sync::Arc};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
    ActionResult,
    effect::{
        ActionEffectContract, EffectInvocationContext, EffectInvocationOutcome,
        EffectPreparationContext, EffectPreparationError, EffectQueryContext,
        EffectReconciliationOutcome, PreparedEffectAdapter, PreparedRemoteEffect,
        ReadOnlyEffectQuery, RemoteDestinationGuarantee, RemoteEffectDescriptor,
        RemoteEffectFactory, RemoteEffectPolicy,
    },
};
use nebula_core::{
    ArtifactSetDigest, Dependencies, OperationId, OrgId, WorkspaceId, accessor::SystemClock,
    action_key, node_key,
};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
    PlanFlavorRevisionInstaller, PlanFlavorRevisionLoader, WorkflowActivationService,
    WorkflowEngine, WorkflowStartService,
};
use nebula_execution::{ExecutionBudget, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_schema::ValidSchema;
use nebula_storage_port::{
    Scope,
    dto::{EffectOccurrenceKey, EffectPhase, WorkflowRecord},
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder, WorkflowDefinition};
use serde_json::{Value, json};

#[path = "effect_protocol/ports.rs"]
mod ports;
#[path = "support/postgres_schema.rs"]
mod postgres_schema;
use ports::Ports;
#[path = "effect_protocol/faults.rs"]
mod faults;
use faults::{Boundary, Fault, FaultLedger};
#[path = "effect_protocol/matrix.rs"]
mod matrix;
#[path = "effect_protocol/observations.rs"]
mod observations;
#[path = "effect_protocol/restart.rs"]
mod restart;
#[path = "effect_protocol/stale_owner.rs"]
mod stale_owner;

#[derive(Debug, Clone, Copy)]
enum ProviderBehavior {
    Applied,
    Ambiguous,
    StableRetry,
    StableExhausted,
    QueryApplied,
    QueryInconclusive,
    QueryRejected,
    Oversized,
    PendingInvocation,
    PreparationTimeout,
    InvocationTimeout,
    BeforeBoundary,
    Rejected,
}

#[derive(Debug)]
struct Provider {
    behavior: ProviderBehavior,
    calls: parking_lot::Mutex<Vec<OperationId>>,
    queries: parking_lot::Mutex<Vec<OperationId>>,
    applied: parking_lot::Mutex<std::collections::HashMap<String, Value>>,
    committed: parking_lot::Mutex<Vec<OperationId>>,
    request_override: parking_lot::Mutex<Option<Value>>,
    destination: parking_lot::Mutex<Vec<u8>>,
    entered: tokio::sync::Notify,
    dropped: tokio::sync::Notify,
}

struct ProviderFactory {
    metadata: ActionMetadata,
    dependencies: Dependencies,
    descriptor: RemoteEffectDescriptor,
    provider: Arc<Provider>,
}

impl ActionFactory for ProviderFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
    fn remote_effect_factory(&self) -> Option<&dyn RemoteEffectFactory> {
        Some(self)
    }
    fn instantiate<'a>(
        &'a self,
        _: &'a NodeDefinition,
        _: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async { panic!("remote effects must never use generic dispatch") })
    }
}

#[async_trait::async_trait]
impl RemoteEffectFactory for ProviderFactory {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        &self.descriptor
    }
    async fn prepare(
        &self,
        input: Value,
        _: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        if matches!(self.provider.behavior, ProviderBehavior::PreparationTimeout) {
            self.provider.entered.notify_one();
            return std::future::pending().await;
        }
        let request = self
            .provider
            .request_override
            .lock()
            .clone()
            .unwrap_or(input);
        PreparedRemoteEffect::new(
            serde_json::to_vec(&request).unwrap().into_boxed_slice(),
            self.provider.destination.lock().clone().into_boxed_slice(),
            Box::new(ProviderAdapter {
                provider: self.provider.clone(),
                request,
            }),
        )
    }
}

struct ProviderAdapter {
    provider: Arc<Provider>,
    request: Value,
}

#[async_trait::async_trait]
impl PreparedEffectAdapter for ProviderAdapter {
    async fn invoke(&self, context: &dyn EffectInvocationContext) -> EffectInvocationOutcome {
        let _drop = scopeguard::guard((), |()| self.provider.dropped.notify_one());
        self.provider.calls.lock().push(context.operation_id());
        if matches!(self.provider.behavior, ProviderBehavior::BeforeBoundary)
            && self.provider.calls.lock().len() == 1
        {
            return EffectInvocationOutcome::BeforeBoundary(
                nebula_action::effect::EffectFailureCode::UnavailableBeforeBoundary,
            );
        }
        if !matches!(
            self.provider.behavior,
            ProviderBehavior::Rejected | ProviderBehavior::QueryRejected
        ) {
            let provider_key = serde_json::to_string(&self.request)
                .expect("fixture requests are JSON values and always serialize");
            let mut applied = self.provider.applied.lock();
            let already_applied = applied.contains_key(&provider_key);
            let previous = applied
                .entry(provider_key)
                .or_insert_with(|| self.request.clone());
            assert_eq!(
                previous, &self.request,
                "provider idempotency identity must bind the same request"
            );
            if !already_applied {
                self.provider.committed.lock().push(context.operation_id());
            }
        }
        self.provider.entered.notify_one();
        match self.provider.behavior {
            ProviderBehavior::Applied | ProviderBehavior::BeforeBoundary => {
                EffectInvocationOutcome::Applied(Box::new(ActionResult::success(
                    json!({"receipt": "provider-receipt-a"}),
                )))
            },
            ProviderBehavior::Ambiguous | ProviderBehavior::StableExhausted => {
                EffectInvocationOutcome::Ambiguous
            },
            ProviderBehavior::StableRetry if self.provider.calls.lock().len() == 1 => {
                EffectInvocationOutcome::Ambiguous
            },
            ProviderBehavior::StableRetry => EffectInvocationOutcome::Applied(Box::new(
                ActionResult::success(json!({"receipt": "deduplicated-receipt"})),
            )),
            ProviderBehavior::QueryApplied
            | ProviderBehavior::QueryInconclusive
            | ProviderBehavior::QueryRejected => EffectInvocationOutcome::Ambiguous,
            ProviderBehavior::Oversized => EffectInvocationOutcome::Applied(Box::new(
                ActionResult::success(Value::String("a".repeat(1_048_577))),
            )),
            ProviderBehavior::PendingInvocation | ProviderBehavior::InvocationTimeout => {
                std::future::pending().await
            },
            ProviderBehavior::PreparationTimeout => {
                panic!("pending preparation cannot authorize invocation")
            },
            ProviderBehavior::Rejected => EffectInvocationOutcome::Rejected(
                nebula_action::effect::EffectFailureCode::Rejected,
            ),
        }
    }
    fn read_only_query(&self) -> Option<&dyn ReadOnlyEffectQuery> {
        matches!(
            self.provider.behavior,
            ProviderBehavior::QueryApplied
                | ProviderBehavior::QueryInconclusive
                | ProviderBehavior::QueryRejected
        )
        .then_some(self)
    }
}

#[async_trait::async_trait]
impl ReadOnlyEffectQuery for ProviderAdapter {
    async fn reconcile(&self, context: &dyn EffectQueryContext) -> EffectReconciliationOutcome {
        self.provider.queries.lock().push(context.operation_id());
        let provider_key = serde_json::to_string(&self.request)
            .expect("fixture requests are JSON values and always serialize");
        if matches!(self.provider.behavior, ProviderBehavior::QueryRejected) {
            return EffectReconciliationOutcome::Rejected(
                nebula_action::effect::EffectFailureCode::Rejected,
            );
        }
        if matches!(self.provider.behavior, ProviderBehavior::QueryInconclusive)
            || !self.provider.applied.lock().contains_key(&provider_key)
        {
            return EffectReconciliationOutcome::Inconclusive;
        }
        EffectReconciliationOutcome::Applied(Box::new(ActionResult::success(
            json!({"receipt": "queried-receipt"}),
        )))
    }
}

struct ProviderPlugin {
    manifest: PluginManifest,
    factory: Arc<ProviderFactory>,
}
impl std::fmt::Debug for ProviderPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderPlugin")
            .finish_non_exhaustive()
    }
}
impl Plugin for ProviderPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![self.factory.clone()]
    }
}

struct Fixture {
    ports: Ports,
    frozen: Arc<FrozenPluginRegistry>,
    definition: WorkflowDefinition,
    scope: Scope,
    provider: Arc<Provider>,
}

impl Fixture {
    async fn new(behavior: ProviderBehavior, ports: Ports) -> Self {
        let provider = Arc::new(Provider {
            behavior,
            calls: parking_lot::Mutex::new(Vec::new()),
            queries: parking_lot::Mutex::new(Vec::new()),
            applied: parking_lot::Mutex::new(std::collections::HashMap::new()),
            committed: parking_lot::Mutex::new(Vec::new()),
            request_override: parking_lot::Mutex::new(None),
            destination: parking_lot::Mutex::new(b"provider/account-a/auth-binding-a".to_vec()),
            entered: tokio::sync::Notify::new(),
            dropped: tokio::sync::Notify::new(),
        });
        let policy = match behavior {
            ProviderBehavior::BeforeBoundary => {
                RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                    .maximum_invocations(2)
                    .maximum_queries(0)
                    .recovery_window(std::time::Duration::from_mins(1))
                    .build()
            },
            ProviderBehavior::StableRetry | ProviderBehavior::StableExhausted => {
                RemoteEffectPolicy::builder(RemoteDestinationGuarantee::stable_key(60_000).unwrap())
                    .maximum_invocations(2)
                    .maximum_queries(0)
                    .recovery_window(std::time::Duration::from_mins(1))
                    .build()
            },
            ProviderBehavior::QueryApplied
            | ProviderBehavior::QueryInconclusive
            | ProviderBehavior::QueryRejected => {
                RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Reconcilable)
                    .maximum_invocations(1)
                    .maximum_queries(1)
                    .recovery_window(std::time::Duration::from_mins(1))
                    .build()
            },
            ProviderBehavior::PreparationTimeout | ProviderBehavior::InvocationTimeout => {
                RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                    .maximum_invocations(1)
                    .maximum_queries(0)
                    .recovery_window(std::time::Duration::from_millis(100))
                    .build()
            },
            _ => RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(std::time::Duration::from_mins(1))
                .build(),
        }
        .unwrap();
        let descriptor = RemoteEffectDescriptor::new("test.provider/v1", 1, policy).unwrap();
        let factory = Arc::new(ProviderFactory {
            metadata: ActionMetadata::new(
                action_key!("provider.send"),
                "Send",
                "Trusted test provider",
            )
            .with_kind(ActionKind::Stateless)
            .with_schema(ValidSchema::empty())
            .with_output_schema(ValidSchema::empty())
            .with_effect_contract(ActionEffectContract::Remote(Box::new(descriptor.clone()))),
            dependencies: Dependencies::new(),
            descriptor,
            provider: provider.clone(),
        });
        let mut plugins = PluginRegistry::new();
        plugins
            .register(Arc::new(
                ResolvedPlugin::from(ProviderPlugin {
                    manifest: PluginManifest::builder("provider", "Provider")
                        .build()
                        .unwrap(),
                    factory,
                })
                .unwrap(),
            ))
            .unwrap();
        let frozen = Arc::new(
            plugins
                .freeze(
                    ArtifactSetDigest::from_bytes([0x78; 32]),
                    "1.0.0".parse().unwrap(),
                )
                .unwrap(),
        );
        let definition = WorkflowBuilder::new("Provider effect")
            .add_node(NodeDefinition::new(node_key!("send"), "Send", "provider", "send").unwrap())
            .build()
            .unwrap();
        let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
        ports
            .workflows
            .workflow
            .create(
                &scope,
                WorkflowRecord {
                    id: definition.id.to_string(),
                    scope: scope.clone(),
                    version: 1,
                    slug: "provider-effect".into(),
                    deleted: false,
                },
            )
            .await
            .unwrap();
        WorkflowActivationService::new(
            ports.workflows.workflow.clone(),
            ports.workflows.versions.clone(),
            frozen.clone(),
            PlanFlavorRevisionInstaller::new(ports.writer.clone()),
            Arc::new(SystemClock),
        )
        .activate(&scope, definition.id, 1, definition.clone())
        .await
        .unwrap();
        Self {
            ports,
            frozen,
            definition,
            scope,
            provider,
        }
    }

    async fn start(&self) -> nebula_core::ExecutionId {
        WorkflowStartService::new(
            self.ports.workflows.clone(),
            self.ports.stores.execution.clone(),
            self.ports.starts.clone(),
            PlanFlavorRevisionLoader::new(self.ports.catalog.clone()),
            self.frozen.clone(),
            Arc::new(SystemClock),
            ExecutionBudget::default(),
        )
        .unwrap()
        .start(
            &self.scope,
            self.definition.id,
            Some(json!({"amount": 7})),
            Some("start"),
            None,
        )
        .await
        .unwrap()
        .state()
        .execution_id
    }

    fn engine(&self) -> WorkflowEngine {
        let executor: ActionExecutor = Arc::new(|_, _, _| {
            Box::pin(async { panic!("remote effects must never call the generic executor") })
        });
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                Arc::new(ActionRegistry::new()),
                Arc::new(InProcessRunner::new(executor)),
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_lease_ttl(std::time::Duration::from_secs(1))
            .with_lease_heartbeat_interval(std::time::Duration::from_millis(250))
            .with_execution_stores(self.ports.stores.clone())
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(self.ports.catalog.clone())),
                self.frozen.clone(),
                self.ports.starts.clone(),
            )
    }
}

#[tokio::test]
async fn applied_effect_has_durable_evidence_and_one_provider_call() {
    let fixture = Fixture::new(ProviderBehavior::Applied, Ports::memory()).await;
    let execution = fixture.start().await;
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(fixture.provider.calls.lock().len(), 1);
    let record = fixture
        .ports
        .ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &fixture.scope,
            &execution.to_string(),
            "send",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.protocol().unwrap().phase(), EffectPhase::Resolved);
    assert!(record.protocol().unwrap().evidence().is_some());
    assert_eq!(
        fixture.provider.calls.lock()[0],
        record.operation().operation_id()
    );
}

#[tokio::test]
async fn opaque_ambiguity_is_durable_unknown_and_engine_recreation_never_reinvokes() {
    let fixture = Fixture::new(ProviderBehavior::Ambiguous, Ports::memory()).await;
    let execution = fixture.start().await;
    let _first = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await;
    assert_eq!(fixture.provider.calls.lock().len(), 1);
    let record = fixture
        .ports
        .ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &fixture.scope,
            &execution.to_string(),
            "send",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.protocol().unwrap().phase(),
        EffectPhase::OutcomeUnknown
    );
    assert!(record.protocol().unwrap().evidence().is_none());
    let _recovery = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await;
    assert_eq!(fixture.provider.calls.lock().len(), 1);
    assert_eq!(
        fixture
            .ports
            .ledger
            .read_exact(&fixture.scope, record.operation().slot_id())
            .await
            .unwrap(),
        record
    );
}
