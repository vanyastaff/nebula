use std::sync::OnceLock;

use nebula_action::{
    ActionRuntimeContext, FromWorkflowNode, InstanceFactory, TriggerRuntimeContext, action::Action,
    context::CredentialContextExt, error::ActionError, metadata::ActionMetadataDraft,
    stateful::StatefulAction, stateless::StatelessAction,
};
use nebula_core::{
    BaseContext, Dependencies, action_key,
    context::Context,
    id::{ExecutionId, WorkflowId},
    node_key,
    scope::{Principal, Scope},
};

use crate::runtime::runner::InProcessRunner;

use super::*;

fn pure_metadata(
    key: nebula_core::ActionKey,
    name: &str,
    description: &str,
) -> ActionMetadataDraft {
    ActionMetadataDraft::new(
        key,
        nebula_action::MetadataName::try_from(name).expect("fixture display name"),
        description,
    )
    .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
}

struct CountingRunner {
    called: Arc<std::sync::atomic::AtomicBool>,
    inner: InProcessRunner,
}

#[async_trait::async_trait]
impl ActionRunner for CountingRunner {
    async fn execute_stateless(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn nebula_action::StatelessHandle>,
        input: nebula_action::ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .execute_stateless(run_context, handle, input, action_context)
            .await
    }

    async fn execute_stream(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn StreamHandle>,
        input: nebula_action::ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .execute_stream(run_context, handle, input, action_context)
            .await
    }
}

/// Echo fixture — Variant A unit struct. Per-test metadata is supplied
/// via [`ActionRegistry::register_stateless_instance`] (the
/// R-NEW-7 test escape), so the static `<Self as Action>::metadata()`
/// is only consulted when the test escape is bypassed.
struct EchoAction;

impl Action for EchoAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(action_key!("test.echo.static"), "Echo", "echoes input")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for EchoAction {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

struct FailAction;

impl Action for FailAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(action_key!("test.fail.static"), "Fail", "always fails")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for FailAction {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Err(ActionError::retryable("transient failure"))
    }
}

fn test_context() -> ActionRuntimeContext {
    ActionRuntimeContext::new(
        Arc::new(
            BaseContext::builder(Scope::default())
                .principal(Principal::System)
                .build()
                .expect("scope + principal must produce a valid BaseContext"),
        ),
        ExecutionId::new(),
        node_key!("test"),
        WorkflowId::new(),
    )
}

fn test_trigger_context() -> TriggerRuntimeContext {
    TriggerRuntimeContext::new(
        Arc::new(
            BaseContext::builder(Scope::default())
                .principal(Principal::System)
                .build()
                .expect("scope + principal must produce a valid BaseContext"),
        ),
        WorkflowId::new(),
        node_key!("test"),
    )
}

fn make_runtime(registry: Arc<ActionRegistry>) -> ActionRuntime {
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();

    ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics).unwrap()
}

/// Build a runtime with a metrics registry we hand back to the caller,
/// so tests can assert on counters/histograms that the runtime wrote
/// through its private `metrics` field.
fn make_runtime_with_metrics(registry: Arc<ActionRegistry>) -> (ActionRuntime, MetricsRegistry) {
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy::default(),
        metrics.clone(),
    )
    .unwrap();
    (rt, metrics)
}

#[tokio::test]
async fn generic_dispatch_rejects_undeclared_effect_before_action_code() {
    use nebula_action::effect::ActionEffectContract;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountedAction(Arc<AtomicUsize>);

    impl Action for CountedAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.owner_required"),
                nebula_action::metadata_name!("Guard"),
                "effect gate",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for CountedAction {
        async fn execute(
            &self,
            input: serde_json::Value,
            _: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(ActionResult::success(input))
        }
    }

    let executions = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn ActionFactory> = Arc::new(
        InstanceFactory::new(
            ActionMetadataDraft::new(
                action_key!("test.owner_required"),
                nebula_action::metadata_name!("Guard"),
                "effect gate",
            )
            .with_effect_contract(ActionEffectContract::Undeclared),
            CountedAction(Arc::clone(&executions)),
        )
        .expect("typed fixture metadata admits"),
    );
    let (runtime, metrics) = make_runtime_with_metrics(Arc::new(ActionRegistry::new()));
    let node = NodeDefinition::new(node_key!("test"), "Guard", "test", "owner_required").unwrap();
    let result = runtime
        .run_factory(
            "test.owner_required",
            factory,
            &node,
            nebula_action::ActionInput::Raw(serde_json::Value::Null),
            &test_context(),
            None,
        )
        .await;

    std::assert_matches!(result, Err(RuntimeError::EffectRequiresOwner));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    let labels = metrics
        .interner()
        .label_set(&[("reason", "effect_requires_owner")]);
    assert_eq!(
        metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
            .unwrap()
            .get(),
        1
    );
    assert_eq!(
        metrics
            .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
            .unwrap()
            .get(),
        0
    );
    assert_eq!(
        metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
        0
    );
}

#[tokio::test]
async fn runtime_rejects_a_handle_from_an_equal_schema_foreign_factory() {
    let left: Arc<dyn ActionFactory> = Arc::new(
        InstanceFactory::new(
            pure_metadata(action_key!("test.left"), "Left", "left contract"),
            EchoAction,
        )
        .unwrap(),
    );
    let right: Arc<dyn ActionFactory> = Arc::new(
        InstanceFactory::new(
            pure_metadata(action_key!("test.right"), "Right", "right contract"),
            EchoAction,
        )
        .unwrap(),
    );
    assert_eq!(
        left.metadata().base().schema(),
        right.metadata().base().schema()
    );
    assert!(!Arc::ptr_eq(left.metadata(), right.metadata()));

    let node = NodeDefinition::new(node_key!("test"), "Right", "test", "right").unwrap();
    let handle = right.instantiate(&node, &test_context()).await.unwrap();
    let runtime = make_runtime(Arc::new(ActionRegistry::new()));

    std::assert_matches!(
        runtime.validate_factory_handle("test.left", left.as_ref(), &handle),
        Err(RuntimeError::FactoryHandleMetadataMismatch { key }) if key == "test.left"
    );
}

#[tokio::test]
async fn max_total_execution_bytes_across_dispatches() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.echo"), "Echo", "echoes input"),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 1024,
            max_total_execution_bytes: 10,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let eid = ExecutionId::new();
    let ctx = ActionRuntimeContext::new(
        Arc::new(
            BaseContext::builder(Scope::default())
                .principal(Principal::System)
                .build()
                .expect("scope + principal must produce a valid BaseContext"),
        ),
        eid,
        node_key!("test"),
        WorkflowId::new(),
    );

    rt.execute_action("test.echo", serde_json::json!(null), &ctx)
        .await
        .expect("first dispatch under total cap");

    let err = rt
        .execute_action("test.echo", serde_json::json!("1234567890"), &ctx)
        .await
        .expect_err("second dispatch exceeds max_total_execution_bytes");

    assert!(
        matches!(
            err,
            RuntimeError::DataLimitExceeded {
                limit_bytes: 10,
                ..
            }
        ),
        "expected total cap error, got {err:?}"
    );

    rt.clear_execution_output_totals(eid);
}

#[tokio::test]
async fn execute_trusted_action() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.echo"), "Echo", "echoes input"),
            EchoAction,
        )
        .expect("valid test catalog definition");

    let rt = make_runtime(registry);
    let input = serde_json::json!({"hello": "world"});
    let result = rt
        .execute_action("test.echo", input.clone(), &test_context())
        .await;
    let action_result = result.unwrap();
    match action_result {
        ActionResult::Success { output } => {
            assert_eq!(output.as_value(), Some(&input));
        },
        other => panic!("expected Success, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_unknown_action_returns_error() {
    let registry = Arc::new(ActionRegistry::new());
    let rt = make_runtime(registry);
    let result = rt
        .execute_action("nonexistent", serde_json::json!(null), &test_context())
        .await;
    assert!(matches!(result, Err(RuntimeError::ActionNotFound { .. })));
}

#[tokio::test]
async fn execute_failing_action_propagates_error() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.fail"), "Fail", "always fails"),
            FailAction,
        )
        .expect("valid test catalog definition");

    let rt = make_runtime(registry);
    let result = rt
        .execute_action("test.fail", serde_json::json!(null), &test_context())
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().is_retryable());
}

#[tokio::test]
async fn data_limit_enforcement() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.big"), "Big", "returns big output"),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();

    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 5, // very small
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let input = serde_json::json!({"big_payload": "this is way too large for 5 bytes"});
    let result = rt.execute_action("test.big", input, &test_context()).await;
    assert!(matches!(
        result,
        Err(RuntimeError::DataLimitExceeded { .. })
    ));
}

#[tokio::test]
async fn metrics_recorded_on_execution() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.tele"), "Tele", "test"),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();

    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy::default(),
        metrics.clone(),
    )
    .unwrap();

    rt.execute_action("test.tele", serde_json::json!("ok"), &test_context())
        .await
        .unwrap();

    // Metrics should be recorded.
    assert_eq!(
        metrics
            .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
            .unwrap()
            .get(),
        1
    );
    assert_eq!(
        metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
        0
    );
}

#[tokio::test]
async fn trigger_context_construction_is_usable_in_runtime() {
    let ctx = test_trigger_context();
    assert!(!ctx.has_credential_id("missing").await);
    assert!(
        ctx.schedule_after(std::time::Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(
        ctx.emit_execution(serde_json::json!({"tick": true}), None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn execute_uses_runner_for_capability_gated() {
    use std::sync::atomic::{AtomicBool, Ordering};

    // Track whether the runner was invoked.
    let runner_called = Arc::new(AtomicBool::new(false));
    let runner = Arc::new(CountingRunner {
        called: Arc::clone(&runner_called),
        inner: InProcessRunner::new(),
    });

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.gated"), "Gated", "capability gated")
                .with_isolation_level(IsolationLevel::CapabilityGated),
            EchoAction,
        )
        .expect("valid test catalog definition");

    let metrics = MetricsRegistry::new();
    let rt =
        ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics).unwrap();

    let result = rt
        .execute_action(
            "test.gated",
            serde_json::json!({"data": 1}),
            &test_context(),
        )
        .await;
    assert!(result.is_ok());
    assert!(
        runner_called.load(Ordering::SeqCst),
        "runner should have been called for CapabilityGated action"
    );
}

#[tokio::test]
async fn spill_to_blob_rejects_when_no_storage() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.spill"), "Spill", "large output"),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();

    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 5,
            large_data_strategy: LargeDataStrategy::SpillToBlob,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    // No blob storage configured -- should reject.
    let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
    let result = rt
        .execute_action("test.spill", input, &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "expected DataLimitExceeded when no blob storage configured"
    );
}

#[tokio::test]
async fn spill_to_blob_succeeds_with_storage() {
    use super::super::blob::{BlobRef, BlobStorage};

    struct FakeBlobStorage;

    #[async_trait::async_trait]
    impl BlobStorage for FakeBlobStorage {
        async fn write(&self, data: &[u8], content_type: &str) -> Result<BlobRef, RuntimeError> {
            Ok(BlobRef {
                uri: "mem://test/blob-1".into(),
                size_bytes: data.len() as u64,
                content_type: content_type.into(),
            })
        }
        async fn read(&self, _blob_ref: &BlobRef) -> Result<Vec<u8>, RuntimeError> {
            Ok(vec![])
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(
                action_key!("test.spill_ok"),
                "SpillOk",
                "large output with storage",
            ),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();

    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 5,
            large_data_strategy: LargeDataStrategy::SpillToBlob,
            ..Default::default()
        },
        metrics,
    )
    .unwrap()
    .with_blob_storage(Arc::new(FakeBlobStorage));

    let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
    let result = rt
        .execute_action("test.spill_ok", input, &test_context())
        .await;
    let action_result = result.expect("should succeed when blob storage is configured");

    // Verify the large inline payload was replaced with an external reference.
    match action_result {
        ActionResult::Success {
            output: ActionOutput::Reference(data_ref),
        } => {
            assert_eq!(data_ref.storage_type, "blob");
            assert_eq!(data_ref.path, "mem://test/blob-1");
            assert!(data_ref.size.is_some());
            assert_eq!(data_ref.content_type.as_deref(), Some("application/json"));
        },
        other => panic!("expected Success with Reference output after spill, got {other:?}"),
    }
}

/// Regression: previously, `enforce_data_limit` only inspected a single
/// "primary" output slot. A `MultiOutput` with oversized fan-out ports
/// sailed through the limit silently — any port could carry an
/// arbitrarily large payload downstream as long as `main_output` was
/// small (or absent). This test pins the fix: every port slot is
/// checked.
#[tokio::test]
async fn multi_output_fanout_port_respects_reject_limit() {
    use std::collections::HashMap;

    use nebula_action::{PortKey, port_key, result::ActionResult as AR};

    struct MultiOutAction;
    impl Action for MultiOutAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(
                action_key!("test.multi_out.static"),
                "MultiOut",
                "multi-port fan-out",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for MultiOutAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<AR<<Self as Action>::Output>, ActionError> {
            // `main_output` is tiny; a fan-out port is huge. Before the
            // fix, only `main_output` was checked and this result passed
            // a byte limit of 16.
            let mut outputs: HashMap<PortKey, ActionOutput<serde_json::Value>> = HashMap::new();
            outputs.insert(
                port_key!("big_port"),
                ActionOutput::Value(serde_json::json!(
                    "this payload is definitely larger than the 16 byte limit"
                )),
            );
            Ok(AR::MultiOutput {
                outputs,
                main_output: Some(ActionOutput::Value(serde_json::json!("ok"))),
            })
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(
                action_key!("test.multi_out"),
                "MultiOut",
                "multi-port fan-out",
            ),
            MultiOutAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 16,
            large_data_strategy: LargeDataStrategy::Reject,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let result = rt
        .execute_action("test.multi_out", serde_json::json!(null), &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "MultiOutput fan-out port must not bypass the data-passing limit \
         — got {result:?}"
    );
}

/// Regression: `Branch.alternatives` previously bypassed the size limit
/// too. A branch node could ship a GB-sized preview alongside the
/// selected output and it would pass through silently.
#[tokio::test]
async fn branch_alternatives_respect_reject_limit() {
    use std::collections::HashMap;

    use nebula_action::result::ActionResult as AR;

    struct BranchAction;
    impl Action for BranchAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(action_key!("test.branch.static"), "Branch", "static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for BranchAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<AR<<Self as Action>::Output>, ActionError> {
            let mut alternatives = HashMap::new();
            alternatives.insert(
                nebula_action::BranchKey::new("else").expect("'else' is a valid branch key"),
                ActionOutput::Value(serde_json::json!(
                    "alternative branch holds way more than 16 bytes of data"
                )),
            );
            Ok(AR::Branch {
                selected: nebula_action::BranchKey::new("then")
                    .expect("'then' is a valid branch key"),
                output: ActionOutput::Value(serde_json::json!("ok")),
                alternatives,
            })
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.branch"), "Branch", "branch with alts"),
            BranchAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 16,
            large_data_strategy: LargeDataStrategy::Reject,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let result = rt
        .execute_action("test.branch", serde_json::json!(null), &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "Branch.alternatives must not bypass the data-passing limit — got {result:?}"
    );
}

#[tokio::test]
async fn collection_children_respect_reject_limit() {
    use nebula_action::result::ActionResult as AR;

    struct CollectionAction;
    impl Action for CollectionAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(action_key!("test.collection.static"), "Coll", "static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for CollectionAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<AR<<Self as Action>::Output>, ActionError> {
            Ok(AR::Success {
                output: ActionOutput::Collection(vec![
                    ActionOutput::Value(serde_json::json!("ok")),
                    ActionOutput::Value(serde_json::json!("this payload is larger than 16 bytes")),
                ]),
            })
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(
                action_key!("test.collection"),
                "Collection",
                "nested values",
            ),
            CollectionAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 16,
            large_data_strategy: LargeDataStrategy::Reject,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let result = rt
        .execute_action("test.collection", serde_json::json!(null), &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "nested collection values must not bypass the data-passing limit — got {result:?}"
    );
}

#[tokio::test]
async fn binary_inline_respects_reject_limit() {
    use nebula_action::{
        output::{BinaryData, BinaryStorage},
        result::ActionResult as AR,
    };

    struct BinaryAction;
    impl Action for BinaryAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(action_key!("test.binary.static"), "Bin", "static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for BinaryAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<AR<<Self as Action>::Output>, ActionError> {
            Ok(AR::Success {
                output: ActionOutput::Binary(BinaryData {
                    content_type: "application/octet-stream".to_owned(),
                    data: BinaryStorage::Inline(vec![0_u8; 64]),
                    size: 1, // intentionally wrong; effective_size() must win
                    metadata: None,
                }),
            })
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.binary"), "Binary", "inline bytes"),
            BinaryAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 16,
            large_data_strategy: LargeDataStrategy::Reject,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let result = rt
        .execute_action("test.binary", serde_json::json!(null), &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "inline binary output must be checked via effective_size() — got {result:?}"
    );
}

#[tokio::test]
async fn reference_metadata_respects_reject_limit() {
    use nebula_action::result::ActionResult as AR;

    struct RefAction;
    impl Action for RefAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(action_key!("test.ref.static"), "Ref", "static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for RefAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<AR<<Self as Action>::Output>, ActionError> {
            Ok(AR::Success {
                output: ActionOutput::Reference(DataReference {
                    storage_type: "blob".to_owned(),
                    path: "x".repeat(128),
                    size: Some(1),
                    content_type: Some("application/json".to_owned()),
                }),
            })
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.ref"), "Reference", "large metadata"),
            RefAction,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 32,
            large_data_strategy: LargeDataStrategy::Reject,
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let result = rt
        .execute_action("test.ref", serde_json::json!(null), &test_context())
        .await;
    assert!(
        matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
        "reference metadata must be included in size enforcement — got {result:?}"
    );
}

// ── #305 regression: dispatch-rejection paths do not skew histogram ─────

/// Register an action that resolves to a kind not executable via
/// `ActionRuntime` — trigger or resource — and assert that `run_factory`
/// does not record duration samples or bump the executions / failures
/// counters. Instead the dispatch-rejected counter increments once with the
/// correct reason label.
#[tokio::test]
async fn trigger_rejection_does_not_observe_histogram() {
    use nebula_action::{FromWorkflowNode, TriggerAction, TriggerEventOutcome, TriggerSource};

    // Minimal TriggerAction fixture — never invoked, only its ActionHandle
    // variant matters for the rejection test.
    struct FakeTrigger;
    struct FakeTriggerSource;
    impl TriggerSource for FakeTriggerSource {
        type Event = ();
    }

    impl Action for FakeTrigger {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.trigger_reject"),
                nebula_action::metadata_name!("FakeTrigger"),
                "rejection fixture",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl TriggerAction for FakeTrigger {
        type Source = FakeTriggerSource;
        type Error = ActionError;

        async fn start(
            &self,
            _ctx: &(impl nebula_action::TriggerContext + ?Sized),
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn stop(
            &self,
            _ctx: &(impl nebula_action::TriggerContext + ?Sized),
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn handle(
            &self,
            _ctx: &(impl nebula_action::TriggerContext + ?Sized),
            _event: (),
        ) -> Result<TriggerEventOutcome, Self::Error> {
            Err(ActionError::fatal(
                "trigger does not accept external events",
            ))
        }
    }

    impl FromWorkflowNode for FakeTrigger {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(FakeTrigger)
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_trigger_factory::<FakeTrigger>()
        .expect("valid test catalog definition");
    let (rt, metrics) = make_runtime_with_metrics(registry);

    let result = rt
        .execute_action(
            "test.trigger_reject",
            serde_json::json!(null),
            &test_context(),
        )
        .await;
    assert!(
        matches!(result, Err(RuntimeError::TriggerNotExecutable { .. })),
        "expected TriggerNotExecutable, got {result:?}"
    );

    // Histogram and execution/failure counters must NOT observe this path.
    assert_eq!(
        metrics
            .histogram(NEBULA_ACTION_DURATION_SECONDS)
            .unwrap()
            .count(),
        0,
        "duration histogram must not sample rejection paths"
    );
    assert_eq!(
        metrics
            .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
            .unwrap()
            .get(),
        0,
        "executions counter must not bump on rejection"
    );
    assert_eq!(
        metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
        0,
        "failures counter must not bump on rejection"
    );

    // Dispatch-rejected counter MUST be labelled and bumped exactly once.
    let labels = metrics
        .interner()
        .label_set(&[("reason", dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE)]);
    assert_eq!(
        metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
            .unwrap()
            .get(),
        1,
        "dispatch-rejected counter should be bumped once with reason=trigger_not_executable"
    );
}

#[tokio::test]
async fn resource_rejection_does_not_increment_execution_metrics() {
    use nebula_action::{FromWorkflowNode, ResourceAction, ResourceProduces};

    // Minimal ResourceAction fixture — never invoked, only its ActionHandle
    // variant matters for the rejection test.
    struct FakeResource;

    impl Action for FakeResource {
        type Input = serde_json::Value;
        // ResourceAction requires Output = ResourceProduces<Self::Resource>.
        type Output = ResourceProduces<serde_json::Value>;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.resource_reject"),
                nebula_action::metadata_name!("FakeResource"),
                "rejection fixture",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl ResourceAction for FakeResource {
        type Resource = serde_json::Value;

        async fn configure(
            &self,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<Self::Resource, ActionError> {
            Ok(serde_json::json!(null))
        }

        async fn cleanup(
            &self,
            _resource: Self::Resource,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    impl FromWorkflowNode for FakeResource {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(FakeResource)
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_resource_factory::<FakeResource>()
        .expect("valid test catalog definition");
    let (rt, metrics) = make_runtime_with_metrics(registry);

    let result = rt
        .execute_action(
            "test.resource_reject",
            serde_json::json!(null),
            &test_context(),
        )
        .await;
    assert!(
        matches!(result, Err(RuntimeError::ResourceNotExecutable { .. })),
        "expected ResourceNotExecutable, got {result:?}"
    );

    assert_eq!(
        metrics
            .histogram(NEBULA_ACTION_DURATION_SECONDS)
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        metrics
            .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
            .unwrap()
            .get(),
        0
    );
    assert_eq!(
        metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
        0
    );

    let labels = metrics
        .interner()
        .label_set(&[("reason", dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE)]);
    assert_eq!(
        metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
            .unwrap()
            .get(),
        1
    );
}

/// Counterpart to the rejection test: a successful stateless dispatch
/// must observe the histogram and bump the executions counter. Pin
/// both so the rejection fix does not regress the dispatched path.
#[tokio::test]
async fn dispatched_stateless_observes_histogram_and_counter() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            pure_metadata(action_key!("test.dispatched"), "Disp", "dispatched"),
            EchoAction,
        )
        .expect("valid test catalog definition");
    let (rt, metrics) = make_runtime_with_metrics(registry);

    rt.execute_action("test.dispatched", serde_json::json!("ok"), &test_context())
        .await
        .expect("dispatched execution must succeed");
    assert_eq!(
        metrics
            .histogram(NEBULA_ACTION_DURATION_SECONDS)
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        metrics
            .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
            .unwrap()
            .get(),
        1
    );
    assert_eq!(
        metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
        0
    );

    let labels = metrics
        .interner()
        .label_set(&[("reason", dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE)]);
    assert_eq!(
        metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
            .unwrap()
            .get(),
        0,
        "dispatch-rejected counter must stay at zero for successful dispatch"
    );
}

// ── Stream action dispatch ───────────────────────────────────────────────

/// Prove the end-to-end stream dispatch path:
/// register via `register_stream_factory` → execute → folded value reaches
/// the result. If the `ActionHandle::Stream` arm in `dispatch_action` is
/// reverted, this test goes red (the action would hit `_ => UNKNOWN_VARIANT`
/// and return `RuntimeError::Internal`).
#[tokio::test]
async fn stream_action_dispatch_yields_folded_value() {
    use futures::stream;
    use nebula_action::{FromWorkflowNode, stream::StreamAction};

    struct CountingStream;

    impl Action for CountingStream {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(
                action_key!("test.stream.counting"),
                "CountingStream",
                "yields 1,2,3 and sums",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StreamAction for CountingStream {
        type Chunk = u64;

        fn open_stream(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> impl futures::Stream<Item = Result<u64, ActionError>> + Send {
            stream::iter([Ok(1u64), Ok(2), Ok(3)])
        }

        fn init(&self) -> serde_json::Value {
            serde_json::json!(0u64)
        }

        fn fold(&self, acc: serde_json::Value, chunk: u64) -> serde_json::Value {
            let running = acc.as_u64().unwrap_or(0);
            serde_json::json!(running + chunk)
        }
    }

    impl FromWorkflowNode for CountingStream {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(CountingStream)
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stream_factory::<CountingStream>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let result = rt
        .execute_action(
            "test.stream.counting",
            serde_json::json!(null),
            &test_context(),
        )
        .await
        .expect("stream dispatch must succeed");

    match result {
        ActionResult::Success { output } => {
            let value = output.into_value().expect("output must be inline Value");
            assert_eq!(value, serde_json::json!(6u64), "1+2+3 must fold to 6");
        },
        other => panic!("expected Success, got {other:?}"),
    }
}

/// D-2 regression: a stream-produced payload that exceeds the per-node
/// limit must be rejected (or spilled). The folded Value goes through
/// `ActionOutput::Value`, which IS measured by `enforce_data_limit`.
/// This test proves the D-2 path is not bypassed by the new kind.
#[tokio::test]
async fn stream_output_respects_data_limit() {
    use futures::stream;
    use nebula_action::{FromWorkflowNode, stream::StreamAction};

    struct BigStream;

    impl Action for BigStream {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            pure_metadata(
                action_key!("test.stream.big"),
                "BigStream",
                "produces an oversized folded value",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StreamAction for BigStream {
        type Chunk = String;

        fn open_stream(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> impl futures::Stream<Item = Result<String, ActionError>> + Send {
            // One chunk whose folded form exceeds any tiny limit.
            stream::iter([Ok("x".repeat(1024))])
        }

        fn init(&self) -> serde_json::Value {
            serde_json::json!("")
        }

        fn fold(&self, _acc: serde_json::Value, chunk: String) -> serde_json::Value {
            serde_json::json!(chunk)
        }
    }

    impl FromWorkflowNode for BigStream {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(BigStream)
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stream_factory::<BigStream>()
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
    let metrics = MetricsRegistry::new();
    let rt = ActionRuntime::try_new(
        registry,
        runner,
        DataPassingPolicy {
            max_node_output_bytes: 10, // far below the 1024-char chunk
            ..Default::default()
        },
        metrics,
    )
    .unwrap();

    let err = rt
        .execute_action("test.stream.big", serde_json::json!(null), &test_context())
        .await
        .expect_err("oversized stream output must be rejected");

    assert!(
        matches!(err, RuntimeError::DataLimitExceeded { .. }),
        "expected DataLimitExceeded, got {err:?}"
    );
}

// ── #304 + #308 regression: stateful cancel + checkpoint ────────────────

use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use serde_json::Value as JsonValue;
use tokio::sync::Mutex as TokioMutex;

// ── Shared counting logic used by multiple fixtures ─────────────────────

fn counting_step(state: &mut JsonValue, target: u32) -> ActionResult<JsonValue> {
    let count = state
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let next = count + 1;
    *state = serde_json::json!({ "count": next });
    if next >= target {
        ActionResult::Break {
            output: ActionOutput::Value(serde_json::json!({ "final": next })),
            reason: nebula_action::result::BreakReason::Completed,
        }
    } else {
        ActionResult::Continue {
            output: ActionOutput::Value(serde_json::json!({ "step": next })),
            progress: None,
            delay: None,
        }
    }
}

// ── CountingTo3 — #308 checkpoint test (3-iteration break) ───────────────

/// Counts `state.count` from 0 to 3; used by checkpoint + resume tests.
struct CountingTo3;

impl Action for CountingTo3 {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(action_key!("test.count"), "CountTo3", "counts to 3")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for CountingTo3 {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({ "count": 0u32 })
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(counting_step(state, 3))
    }
}
impl FromWorkflowNode for CountingTo3 {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CountingTo3)
    }
}

// ── CountingTo5 — #308 resume test (5-iteration break) ───────────────────

struct CountingTo5;

impl Action for CountingTo5 {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(action_key!("test.count5"), "CountTo5", "counts to 5")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for CountingTo5 {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({ "count": 0u32 })
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(counting_step(state, 5))
    }
}
impl FromWorkflowNode for CountingTo5 {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CountingTo5)
    }
}

// ── CountingTo2 — #308 resume-from-checkpoint test (breaks at 2) ─────────

struct CountingTo2;

impl Action for CountingTo2 {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(action_key!("test.count2"), "CountTo2", "counts to 2")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for CountingTo2 {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({ "count": 0u32 })
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(counting_step(state, 2))
    }
}
impl FromWorkflowNode for CountingTo2 {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CountingTo2)
    }
}

// ── SleepyStateful — #304 cancel-aborts-handler test ─────────────────────

/// Awaits a 1-hour sleep inside `execute`; used to prove cancellation aborts it.
struct SleepyStateful;

impl Action for SleepyStateful {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(
            action_key!("test.sleepy"),
            "SleepyStateful",
            "hangs in execute",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for SleepyStateful {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        _state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        tokio::time::sleep(std::time::Duration::from_hours(1)).await;
        Ok(ActionResult::Break {
            output: ActionOutput::Value(serde_json::json!(null)),
            reason: nebula_action::result::BreakReason::Completed,
        })
    }
}
impl FromWorkflowNode for SleepyStateful {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(SleepyStateful)
    }
}

/// Recording sink — stores every save/clear call so tests can assert
/// on the exact sequence of checkpoint operations.
#[derive(Default)]
struct RecordingSink {
    preload: std::sync::Mutex<Option<StatefulCheckpoint>>,
    saves: TokioMutex<Vec<StatefulCheckpoint>>,
    clears: AtomicU32,
    fail_load: std::sync::atomic::AtomicBool,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }
    fn with_preload(cp: StatefulCheckpoint) -> Self {
        let s = Self::default();
        *s.preload.lock().unwrap() = Some(cp);
        s
    }
    fn with_failing_load() -> Self {
        let s = Self::default();
        s.fail_load
            .store(true, std::sync::atomic::Ordering::Relaxed);
        s
    }
}

#[async_trait::async_trait]
impl StatefulCheckpointSink for RecordingSink {
    async fn load(&self) -> Result<Option<StatefulCheckpoint>, ActionError> {
        if self.fail_load.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ActionError::fatal("simulated checkpoint load failure"));
        }
        Ok(self.preload.lock().unwrap().clone())
    }
    async fn save(&self, cp: &StatefulCheckpoint) -> Result<(), ActionError> {
        self.saves.lock().await.push(cp.clone());
        Ok(())
    }
    async fn clear(&self) -> Result<(), ActionError> {
        self.clears.fetch_add(1, AtomicOrdering::Relaxed);
        Ok(())
    }
}

/// #304 regression: a stateful action that awaits a 1-hour sleep inside
/// `execute` must abort the moment the cancellation token fires — not
/// 1 hour later. Uses `start_paused = true` so the sleep never
/// naturally advances.
#[tokio::test(start_paused = true)]
async fn execute_stateful_aborts_handler_on_cancel() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<SleepyStateful>()
        .expect("valid test catalog definition");
    let rt = Arc::new(make_runtime(registry));

    let ctx = test_context();
    let cancel = ctx.cancellation().clone();

    // Dispatch on a task so we can cancel after 10ms of virtual time.
    let rt_clone = Arc::clone(&rt);
    let handle = tokio::spawn(async move {
        rt_clone
            .execute_action("test.sleepy", serde_json::json!(null), &ctx)
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    cancel.cancel();

    // Use a bounded timeout so a broken fix presents as a hang, not a
    // success. 500ms of virtual time is three orders of magnitude
    // less than the handler's 1-hour sleep.
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
        .await
        .expect("execute_stateful must observe cancel inside handler.execute()")
        .expect("task panicked");
    assert!(
        matches!(
            result,
            Err(RuntimeError::ActionError(ActionError::Cancelled))
        ),
        "expected ActionError::Cancelled, got {result:?}"
    );
}

/// #308 regression: every iteration boundary is checkpointed.
/// Counting 0→3 produces two `save()` calls (at iterations 1 and 2)
/// and one `clear()` call on the terminal `Break` at iteration 3.
#[tokio::test]
async fn execute_stateful_checkpoints_each_iteration() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<CountingTo3>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let sink = Arc::new(RecordingSink::new());
    let result = rt
        .execute_action_with_checkpoint(
            "test.count",
            None,
            serde_json::json!(null),
            &test_context(),
            Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
        )
        .await;
    assert!(
        matches!(result, Ok(ActionResult::Break { .. })),
        "{result:?}"
    );

    let saves = sink.saves.lock().await;
    assert_eq!(
        saves.len(),
        2,
        "expected 2 saves (iterations 1 and 2), got {:?}",
        *saves
    );
    assert_eq!(saves[0].iteration, 1);
    assert_eq!(saves[0].state, serde_json::json!({"count": 1u32}));
    assert_eq!(saves[1].iteration, 2);
    assert_eq!(saves[1].state, serde_json::json!({"count": 2u32}));
    assert_eq!(
        sink.clears.load(AtomicOrdering::Relaxed),
        1,
        "expected exactly one clear() on terminal iteration"
    );
}

/// #308 regression: seeding the sink with a checkpoint at iteration 3
/// must make the handler visibly resume from `count=3`, not 0.
/// Counting to 5 from a checkpoint of 3 is 2 more iterations: one
/// `Continue` at 4 (save), one `Break` at 5 (clear).
#[tokio::test]
async fn execute_stateful_resumes_from_checkpoint() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<CountingTo5>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let seed = StatefulCheckpoint::new(3, serde_json::json!({ "count": 3u32 }));
    let sink = Arc::new(RecordingSink::with_preload(seed));

    let result = rt
        .execute_action_with_checkpoint(
            "test.count5",
            None,
            serde_json::json!(null),
            &test_context(),
            Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
        )
        .await
        .expect("execute should succeed");
    match result {
        ActionResult::Break { output, .. } => {
            assert_eq!(output.as_value(), Some(&serde_json::json!({"final": 5u32})));
        },
        other => panic!("expected Break, got {other:?}"),
    }

    let saves = sink.saves.lock().await;
    assert_eq!(
        saves.len(),
        1,
        "only one Continue should have happened on resume, got {:?}",
        *saves
    );
    assert_eq!(saves[0].iteration, 4);
    assert_eq!(sink.clears.load(AtomicOrdering::Relaxed), 1);
}

/// #308 gotcha regression: a checkpoint sink that fails `load()` must
/// still complete via fallback to `init_state`. This test pins the
/// functional fallback path and checkpoint side effects.
#[tokio::test]
async fn execute_stateful_load_failure_falls_back_to_init_state() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<CountingTo2>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let sink = Arc::new(RecordingSink::with_failing_load());
    let result = rt
        .execute_action_with_checkpoint(
            "test.count2",
            None,
            serde_json::json!(null),
            &test_context(),
            Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
        )
        .await
        .expect("fallback must still run the action to completion");

    match result {
        ActionResult::Break { output, .. } => {
            assert_eq!(output.as_value(), Some(&serde_json::json!({"final": 2u32})));
        },
        other => panic!("expected Break, got {other:?}"),
    }
    // One Continue at 1, one Break at 2, starting from init_state.
    let saves = sink.saves.lock().await;
    assert_eq!(saves.len(), 1, "expected one save at iteration 1");
    assert_eq!(saves[0].iteration, 1);
    assert_eq!(sink.clears.load(AtomicOrdering::Relaxed), 1);
}

// ── NoProgressStateful — spec 28 stuck-state guard ───────────────────────

/// Returns `Continue` on every iteration without mutating state — pins the
/// spec 28 stuck-state guard (a `Continue` with byte-identical state must
/// surface as `RuntimeError::StatefulStuck`).
struct NoProgressStateful;

impl Action for NoProgressStateful {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(
            action_key!("test.stuck"),
            "NoProgress",
            "never advances state",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for NoProgressStateful {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({ "cursor": 0u32 })
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        _state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::Continue {
            output: ActionOutput::Value(serde_json::json!(null)),
            progress: None,
            delay: None,
        })
    }
}
impl FromWorkflowNode for NoProgressStateful {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(NoProgressStateful)
    }
}

/// Spec 28: a stateful action that Continues without mutating its state
/// must surface as a typed `RuntimeError::StatefulStuck`, NOT as an opaque
/// `ActionError::Fatal`. Retry/error routing depends on the typed
/// classification.
#[tokio::test]
async fn execute_stateful_stuck_surfaces_typed_variant() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<NoProgressStateful>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let result = rt
        .execute_action("test.stuck", serde_json::json!(null), &test_context())
        .await;

    match result {
        Err(RuntimeError::StatefulStuck {
            action_key,
            iteration,
            ..
        }) => {
            assert_eq!(action_key.as_str(), "test.stuck");
            assert_eq!(iteration, 1, "stall detected on the first Continue");
        },
        other => panic!("expected RuntimeError::StatefulStuck, got {other:?}"),
    }
}

// ── EndlessStateful — iteration-cap test ─────────────────────────────────

/// Advances `state.count` on every iteration but never breaks — exercises
/// the iteration cap without tripping the stuck-state guard.
struct EndlessStateful;

impl Action for EndlessStateful {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        pure_metadata(
            action_key!("test.endless"),
            "EndlessStateful",
            "never breaks",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatefulAction for EndlessStateful {
    type State = JsonValue;
    fn init_state(&self) -> Self::State {
        serde_json::json!({ "count": 0u32 })
    }
    async fn execute(
        &self,
        _input: &Self::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let count = state
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        *state = serde_json::json!({ "count": count + 1 });
        Ok(ActionResult::Continue {
            output: ActionOutput::Value(serde_json::json!({ "n": count + 1 })),
            progress: None,
            delay: None,
        })
    }
}
impl FromWorkflowNode for EndlessStateful {
    type Error = ActionError;
    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(EndlessStateful)
    }
}

/// A stateful action whose state evolves every iteration must still be
/// capped at `MAX_ITERATIONS` and surface as a typed
/// `RuntimeError::IterationCapExceeded` — not a generic action fatal.
#[tokio::test(flavor = "current_thread")]
async fn execute_stateful_iteration_cap_surfaces_typed_variant() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<EndlessStateful>()
        .expect("valid test catalog definition");
    let rt = make_runtime(registry);

    let result = rt
        .execute_action("test.endless", serde_json::json!(null), &test_context())
        .await;

    match result {
        Err(RuntimeError::IterationCapExceeded {
            action_key, cap, ..
        }) => {
            assert_eq!(action_key.as_str(), "test.endless");
            assert_eq!(cap, 10_000);
        },
        other => panic!("expected RuntimeError::IterationCapExceeded, got {other:?}"),
    }
}

// ── FIX 2: SHA-256 state-digest determinism ───────────────────────────
//
// `stateful_state_digest` previously used `DefaultHasher`, which is
// seeded with a random value per process instance — meaning two calls
// in different processes (or after a Rust upgrade) could return different
// digests for the same JSON value, causing the stuck-state guard to fire
// spuriously on resume. The SHA-256 replacement MUST produce the same
// `u64` for the same `serde_json::Value` every time, including across
// processes and after restarts.
//
// These tests are RED-ON-REVERT: they pin the exact SHA-256-derived u64
// so that any reversion to DefaultHasher (whose output is process-seeded)
// or any other non-deterministic hasher immediately breaks them. The
// constants were computed by running the SHA-256 implementation once and
// recording the output.

/// The expected SHA-256 digest of `serde_json::to_vec(&Value::Null)` (`b"null"`),
/// truncated to `u64` little-endian from the first 8 bytes of the hash.
const DIGEST_NULL: u64 = 0x8f49_e7af_984e_2374;

/// The expected SHA-256 digest of the fixed complex JSON object below.
/// Value: `{"counter":42,"node_a":"completed","node_b":{"output":[1,2,3]}}`.
/// Note: `serde_json` serialises object keys in insertion order; the bytes
/// are stable for a fixed input.
const DIGEST_COMPLEX: u64 = 0xfe1f_b90a_b268_98a5;

#[test]
fn stateful_state_digest_null_equals_pinned_sha256_constant() {
    // If DefaultHasher were restored, this would return a process-seeded
    // value that (with overwhelming probability) differs from DIGEST_NULL.
    assert_eq!(
        stateful_state_digest(&serde_json::Value::Null),
        DIGEST_NULL,
        "digest of null must equal the pinned SHA-256 constant; \
         a mismatch means the hasher was changed (or reverted to DefaultHasher)"
    );
}

#[test]
fn stateful_state_digest_complex_equals_pinned_sha256_constant() {
    let state = serde_json::json!({
        "node_a": "completed",
        "node_b": { "output": [1, 2, 3] },
        "counter": 42
    });
    assert_eq!(
        stateful_state_digest(&state),
        DIGEST_COMPLEX,
        "digest of fixed complex JSON must equal the pinned SHA-256 constant; \
         a mismatch means the hasher was changed (or reverted to DefaultHasher)"
    );
}

#[test]
fn stateful_state_digest_is_cross_instance_deterministic() {
    // Belt-and-suspenders: repeated calls within the same process also match.
    // The pinned-constant tests above are the red-on-revert guards;
    // this test confirms the implementation is at least self-consistent.
    let state = serde_json::json!({ "node_a": "completed", "counter": 99 });
    assert_eq!(stateful_state_digest(&state), stateful_state_digest(&state),);

    // Different values must produce different digests.
    let other = serde_json::json!({ "node_a": "failed" });
    assert_ne!(stateful_state_digest(&state), stateful_state_digest(&other));
}
