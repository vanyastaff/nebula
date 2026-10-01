//! A journaled node's exits before its action runs.
//!
//! The ledger seeding writes an earlier dispatch's crash residue directly:
//! a slot of the node with a granted call that was never explained.

use serde_json::json;

use nebula_credential::default_credential_accessor;
use nebula_storage_port::{
    FencingToken,
    dto::{
        AttemptGeneration, DestinationCapability, EffectSlotBinding, OperationCommand,
        PreparedEffectContract, PreparedEffectPolicy, RequestFingerprint,
    },
    store::OperationLedger,
};

use crate::{EffectExecutionError, resolver::NodeInputRequest};

use super::*;

#[derive(serde::Deserialize, nebula_schema::Schema)]
struct ProbeInput {
    #[field(expression_required)]
    amount: i64,
}

/// A stateless action with the default (`Journaled`) effect contract that
/// counts its dispatches.
struct JournaledProbe(Arc<AtomicU32>);

impl Action for JournaledProbe {
    type Input = ProbeInput;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.journaled_probe"),
            nebula_action::metadata_name!("Journaled probe"),
            "Counts the dispatches of a journaled node",
        )
    }

    fn dependencies() -> &'static Dependencies {
        EchoHandler::dependencies()
    }
}

impl StatelessAction for JournaledProbe {
    async fn execute(
        &self,
        input: ProbeInput,
        _: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(json!({ "amount": input.amount })))
    }
}

/// How the node's input resolves.
#[derive(Debug, Clone, Copy)]
enum Input {
    Valid,
    Failing,
}

/// One leased execution of node `charge` over an in-memory ledger.
struct JournaledNode {
    _executions: nebula_storage::InMemoryExecutionStore,
    ledger: Arc<nebula_storage::inmem::InMemoryOperationLedger>,
    scope: Scope,
    execution_id: ExecutionId,
    fencing: FencingToken,
    dispatches: Arc<AtomicU32>,
    engine: WorkflowEngine,
    factory: Arc<dyn nebula_action::ActionFactory>,
}

impl JournaledNode {
    async fn new() -> Self {
        let executions = nebula_storage::InMemoryExecutionStore::new();
        let ledger = Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
            &executions,
        ));
        let scope = Scope::new("workspace-a", "org-a");
        let execution_id = ExecutionId::new();
        executions
            .create(
                &scope,
                &execution_id.to_string(),
                "workflow",
                json!({"status": "Created"}),
            )
            .await
            .expect("execution row");
        let fencing = executions
            .acquire_lease(
                &scope,
                &execution_id.to_string(),
                "runner",
                Duration::from_secs(30),
            )
            .await
            .expect("lease")
            .expect("granted");
        let dispatches = Arc::new(AtomicU32::new(0));
        let registry = Arc::new(ActionRegistry::new());
        registry
            .register_stateless_instance(
                JournaledProbe::metadata(),
                JournaledProbe(Arc::clone(&dispatches)),
            )
            .expect("register the probe");
        let (_, factory) = registry
            .get_factory(&action_key!("test.journaled_probe"))
            .expect("the probe's factory");
        let (engine, _) = make_engine(registry);
        Self {
            _executions: executions,
            ledger,
            scope,
            execution_id,
            fencing,
            dispatches,
            engine,
            factory,
        }
    }

    /// Writes an earlier dispatch's crash residue: a slot of the node whose
    /// granted call was never explained.
    async fn seed_unexplained_call(&self) {
        let policy = PreparedEffectPolicy::builder(DestinationCapability::Opaque)
            .recovery_window(nebula_resource::call::OPERATION_DEADLINE_CAP)
            .maximum_invocations(1)
            .maximum_queries(0)
            .build()
            .expect("policy");
        let contract = PreparedEffectContract::new(RequestFingerprint::new(1, [1; 32]), policy)
            .expect("contract");
        let execution = self.execution_id.to_string();
        let prepared = self
            .ledger
            .prepare(
                &EffectSlotBinding {
                    scope: &self.scope,
                    execution_id: &execution,
                    node_key: "charge",
                    occurrence: "unit/v1/test.payments/op/#000000",
                    attempt_generation: AttemptGeneration::new(1),
                    fingerprint: RequestFingerprint::new(1, [2; 32]),
                    destination: DestinationCapability::Opaque,
                    contract: &contract,
                    provider_key: None,
                },
                self.fencing,
            )
            .await
            .expect("prepare");
        let slot_id = prepared.operation().slot_id();
        let record = self
            .ledger
            .read_exact(&self.scope, slot_id)
            .await
            .expect("record");
        let revision = record.protocol().expect("protocol").revision();
        self.ledger
            .advance(
                &self.scope,
                slot_id,
                self.fencing,
                &OperationCommand::GrantInvocation {
                    expected_revision: revision,
                },
            )
            .await
            .expect("grant");
    }

    async fn slots(&self) -> usize {
        self.ledger
            .read_occurrences(&self.scope, &self.execution_id.to_string(), "charge")
            .await
            .expect("occurrences")
            .len()
    }

    /// The node's task, with `input`, `cancel`, the credential `refresh`
    /// hook and the `rate_limiter`.
    fn task(
        &self,
        input: Input,
        cancel: CancellationToken,
        credential_refresh: Option<CredentialRefreshFn>,
        rate_limiter: Option<Arc<nebula_resilience::rate_limiter::TokenBucket>>,
    ) -> NodeTask {
        let mut expressions = ExpressionEngine::new();
        expressions.register_function("fail_input", |_, _, _, _| {
            Err(nebula_expression::ExpressionError::invalid_argument(
                "fail_input",
                "input unavailable",
            ))
        });
        let amount = match input {
            Input::Valid => "{{ 7 }}",
            Input::Failing => "{{ fail_input() }}",
        };
        let node = NodeDefinition::new(
            node_key!("charge"),
            "Charge",
            "test",
            "test.journaled_probe",
        )
        .expect("node")
        .with_parameter("amount", nebula_workflow::ParamValue::expression(amount));
        let prepared = ParamResolver::new(Arc::new(expressions))
            .prepare(NodeInputRequest {
                node_key: &node.id,
                parameters: &node.parameters,
                predecessor_input: json!(null),
                outputs: &DashMap::new(),
                shared_outputs: &DashMap::new(),
                schema: &nebula_schema::schema_of::<ProbeInput>().expect("schema"),
                cancellation: cancel.clone(),
            })
            .expect("prepared input");
        let metadata = self.factory.metadata();
        NodeTask {
            runtime: self.engine.runtime.clone(),
            factory_dispatch: NodeFactoryDispatch::Frozen {
                factory: Arc::clone(&self.factory),
                effect_contract: metadata.effect_contract().clone(),
                action_version: metadata.base().version().clone(),
            },
            cancel,
            sem: Arc::new(Semaphore::new(1)),
            outputs: Arc::new(DashMap::new()),
            execution_id: self.execution_id,
            node_key: node.id.clone(),
            workflow_id: WorkflowId::new(),
            action_key: node.action_key.to_string(),
            node: Arc::new(node),
            input: prepared,
            support_inputs: HashMap::new(),
            credentials: default_credential_accessor(),
            resources: nebula_action::capability::default_resource_accessor(),
            credential_refresh,
            rate_limiter,
            scope: self.scope.clone(),
            fencing: Some(self.fencing),
            operation_ledger: Some(Arc::clone(&self.ledger) as Arc<dyn OperationLedger>),
            clock: Arc::new(SystemClock),
            attempt_generation: 1,
            engine_resources: None,
            execution_deadline: None,
            metrics: MetricsRegistry::new(),
        }
    }
}

/// A credential refresh hook that always fails.
fn failing_refresh() -> CredentialRefreshFn {
    Arc::new(|_| Box::pin(async { Err(ActionError::retryable("credential store down")) }))
}

/// Asserts `result` is the journal's unknown-outcome verdict.
fn assert_unknown(result: &Result<ActionResult<serde_json::Value>, EngineError>, exit: &str) {
    assert!(
        matches!(
            result,
            Err(EngineError::Effect(
                EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. }
            ))
        ),
        "{exit}: {result:?}"
    );
}

#[tokio::test]
async fn every_pre_dispatch_exit_after_an_unexplained_call_fails_the_node_unknown() {
    let node = JournaledNode::new().await;
    node.seed_unexplained_call().await;
    // A spent bucket: the node's acquire is refused.
    let spent = Arc::new(
        nebula_resilience::rate_limiter::TokenBucket::new(1, 0.001).expect("token bucket"),
    );
    {
        use nebula_resilience::rate_limiter::RateLimiter;
        spent.acquire().await.expect("the bucket's only token");
    }
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let exits = [
        (
            "input resolution",
            node.task(Input::Failing, CancellationToken::new(), None, None),
        ),
        (
            "credential refresh",
            node.task(
                Input::Valid,
                CancellationToken::new(),
                Some(failing_refresh()),
                None,
            ),
        ),
        (
            "rate limit",
            node.task(Input::Valid, CancellationToken::new(), None, Some(spent)),
        ),
        (
            "cancellation",
            node.task(Input::Valid, cancelled, None, None),
        ),
    ];
    for (exit, task) in exits {
        let (_, result) = task.run().await;
        assert_unknown(&result, exit);
    }
    assert_eq!(
        node.dispatches.load(Ordering::SeqCst),
        0,
        "no exit dispatched the action"
    );
    assert_eq!(node.slots().await, 1, "concluding wrote no slot");
}

#[tokio::test]
async fn a_pre_dispatch_exit_with_no_unknown_call_keeps_its_error() {
    let node = JournaledNode::new().await;
    let (_, result) = node
        .task(
            Input::Valid,
            CancellationToken::new(),
            Some(failing_refresh()),
            None,
        )
        .run()
        .await;
    assert!(
        matches!(
            &result,
            Err(EngineError::Action(
                ActionError::CredentialRefreshFailed { .. }
            ))
        ),
        "{result:?}"
    );
    let (_, result) = node
        .task(Input::Failing, CancellationToken::new(), None, None)
        .run()
        .await;
    assert!(
        matches!(&result, Err(EngineError::ParameterResolution { .. })),
        "{result:?}"
    );
    assert_eq!(node.slots().await, 0, "nothing was prepared");
}

#[tokio::test]
async fn a_dispatched_node_after_an_unexplained_call_fails_unknown() {
    let node = JournaledNode::new().await;
    node.seed_unexplained_call().await;
    let (_, result) = node
        .task(Input::Valid, CancellationToken::new(), None, None)
        .run()
        .await;
    assert_unknown(&result, "after the action");
    assert_eq!(node.dispatches.load(Ordering::SeqCst), 1);
}
