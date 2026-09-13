use std::sync::Mutex;

use serde_json::json;

use super::*;

#[derive(serde::Deserialize, nebula_schema::Schema)]
struct CancelInput {
    #[field(expression_required)]
    first: i64,
    #[field(expression_required)]
    second: i64,
}

struct CancelProbe(Arc<AtomicU32>);

impl Action for CancelProbe {
    type Input = CancelInput;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.cancel_input"),
            nebula_action::metadata_name!("Cancel input"),
            "Observe input cancellation before execution",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        EchoHandler::dependencies()
    }
}

impl StatelessAction for CancelProbe {
    async fn execute(
        &self,
        input: CancelInput,
        _: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(
            json!({"first": input.first, "second": input.second}),
        ))
    }
}

#[tokio::test]
async fn cancellation_during_program_resolution_never_refreshes_or_executes() {
    // Builtins are function pointers and evaluation may run on another worker.
    // This test-local static makes cancellation deterministic without sleeping.
    static CANCEL: Mutex<Option<CancellationToken>> = Mutex::new(None);
    let cancellation = CancellationToken::new();
    assert!(
        CANCEL
            .lock()
            .expect("cancellation fixture mutex remains healthy")
            .replace(cancellation.clone())
            .is_none()
    );
    let mut expression_engine = ExpressionEngine::new();
    expression_engine.register_function("cancel_input", |_, _, _, output| {
        CANCEL
            .lock()
            .expect("cancellation fixture mutex remains healthy")
            .as_ref()
            .expect("cancellation fixture installs the token before evaluation")
            .cancel();
        output.signed_integer(7)
    });
    let resolver = ParamResolver::new(Arc::new(expression_engine));
    let node = NodeDefinition::new(node_key!("cancel"), "Cancel", "test", "test.cancel_input")
        .unwrap()
        .with_parameter(
            "first",
            nebula_workflow::ParamValue::expression("{{ cancel_input() }}"),
        )
        .with_parameter("second", nebula_workflow::ParamValue::expression("{{ 7 }}"));
    let prepared = resolver
        .prepare(NodeInputRequest {
            node_key: &node.id,
            parameters: &node.parameters,
            predecessor_input: json!(null),
            outputs: &DashMap::new(),
            shared_outputs: &DashMap::new(),
            schema: &nebula_schema::schema_of::<CancelInput>().unwrap(),
            cancellation: cancellation.clone(),
        })
        .unwrap();
    let calls = Arc::new(AtomicU32::new(0));
    let refreshes = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(CancelProbe::metadata(), CancelProbe(calls.clone()))
        .unwrap();
    let (_, factory) = registry
        .get_factory(&action_key!("test.cancel_input"))
        .unwrap();
    let (engine, _) = make_engine(registry);
    let refresh_count = refreshes.clone();
    let outputs = Arc::new(DashMap::new());
    let task = NodeTask {
        runtime: engine.runtime.clone(),
        factory_dispatch: NodeFactoryDispatch::DirectRegistry { factory },
        cancel: cancellation.clone(),
        sem: Arc::new(Semaphore::new(1)),
        outputs: outputs.clone(),
        execution_id: ExecutionId::new(),
        node_key: node.id.clone(),
        workflow_id: WorkflowId::new(),
        action_key: node.action_key.to_string(),
        node: Arc::new(node),
        input: prepared,
        support_inputs: HashMap::new(),
        credentials: default_credential_accessor(),
        resources: default_resource_accessor(),
        credential_refresh: Some(Arc::new(move |_| {
            refresh_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })),
        rate_limiter: None,
        scope: crate::store_seam::single_tenant_scope(),
        fencing: None,
        operation_ledger: None,
        clock: Arc::new(SystemClock),
        attempt_generation: 1,
    };
    let (_, result) = task.run().await;
    let _ = CANCEL
        .lock()
        .expect("cancellation fixture mutex remains healthy")
        .take();
    std::assert_matches!(result, Err(EngineError::Cancelled));
    assert!(
        cancellation.is_cancelled(),
        "the expression builtin must have evaluated"
    );
    assert_eq!(refreshes.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        outputs.is_empty(),
        "cancelled resolution must publish no partial output"
    );
}
