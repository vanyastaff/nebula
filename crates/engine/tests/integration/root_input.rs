//! Root-shaped values cross the parameter and typed dispatch boundaries intact.

use std::{
    marker::PhantomData,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionMetadataDraft, ActionResult, ConnectionFilter,
    InputPort, StatelessAction, SupportPort,
};
use nebula_core::{Dependencies, action_key, node_key, port_key};
use nebula_engine::{ActionRegistry, ExecutionResult};
use nebula_execution::context::ExecutionBudget;
use nebula_schema::HasSchema;
use nebula_workflow::{Connection, NodeDefinition, ParamValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{make_engine, make_workflow};

pub(super) struct RootEcho<T> {
    calls: Arc<AtomicUsize>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Action for RootEcho<T>
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    type Input = T;
    type Output = T;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.root_echo"),
            nebula_action::metadata_name!("Root echo"),
            "Preserve root input data",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl<T> StatelessAction for RootEcho<T>
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    async fn execute(
        &self,
        input: T,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<T>, ActionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

#[derive(Serialize, Deserialize, nebula_schema::Schema)]
struct UnitInput;

#[derive(Serialize, Deserialize, nebula_schema::Schema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde object-wire fixture must remain distinct from unit null"
)]
struct EmptyRecordInput {}

#[derive(Serialize, Deserialize, nebula_schema::Schema)]
struct RequiredExpressionInput {
    #[field(expression_required)]
    value: i64,
}

struct SupportEcho;

impl Action for SupportEcho {
    type Input = ();
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.support_echo"),
            nebula_action::metadata_name!("Support echo"),
            "Return support input data",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for SupportEcho {
    async fn execute(
        &self,
        (): (),
        context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let values = context.support_inputs().values(&port_key!("model"));
        Ok(ActionResult::success(Value::Array(values.to_vec())))
    }
}

fn support_registry(port: SupportPort) -> Arc<ActionRegistry> {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            RootEcho::<Value>::metadata(),
            RootEcho::<Value> {
                calls: Arc::new(AtomicUsize::new(0)),
                marker: PhantomData,
            },
        )
        .unwrap();
    registry
        .register_stateless_instance(
            SupportEcho::metadata().add_input(InputPort::Support(port)),
            SupportEcho,
        )
        .unwrap();
    registry
}

fn support_port(required: bool, multi: bool, filter: ConnectionFilter) -> SupportPort {
    SupportPort {
        key: port_key!("model"),
        name: "Model".to_owned(),
        description: "Support data".to_owned(),
        required,
        multi,
        filter,
    }
}

pub(super) fn registry<T>(metadata: ActionMetadataDraft) -> (Arc<ActionRegistry>, Arc<AtomicUsize>)
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            metadata,
            RootEcho::<T> {
                calls: Arc::clone(&calls),
                marker: PhantomData,
            },
        )
        .unwrap();
    (registry, calls)
}

async fn execute<T>(wire: Value, parameter: Option<Value>) -> (ExecutionResult, usize)
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let (registry, calls) = registry::<T>(RootEcho::<T>::metadata());
    let (engine, _) = make_engine(registry);
    let mut node =
        NodeDefinition::new(node_key!("root"), "Root", "test", "test.root_echo").unwrap();
    if let Some(parameter) = parameter {
        node = node.with_parameter("value", ParamValue::literal(parameter));
    }
    let workflow = make_workflow(vec![node], vec![]);
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            wire,
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    (result, calls.load(Ordering::SeqCst))
}

async fn assert_roundtrip<T>(wire: Value)
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let (result, calls) = execute::<T>(wire.clone(), None).await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.node_output(&node_key!("root")), Some(&wire));
    assert_eq!(calls, 1);
}

fn assert_input_rejected(result: &ExecutionResult, calls: usize) {
    assert!(!result.is_success(), "{result:?}");
    let error = &result.node_errors[&node_key!("root")];
    assert_eq!(
        error,
        "parameter resolution failed for node root, param '': input schema validation failed"
    );
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn no_parameters_preserves_unit_null_wire() {
    assert_roundtrip::<()>(serde_json::to_value(()).unwrap()).await;
    assert_roundtrip::<UnitInput>(serde_json::to_value(UnitInput).unwrap()).await;
}

#[tokio::test]
async fn no_parameters_preserves_empty_braced_object_wire() {
    assert_roundtrip::<EmptyRecordInput>(serde_json::to_value(EmptyRecordInput {}).unwrap()).await;
    let (result, calls) = execute::<EmptyRecordInput>(Value::Null, None).await;
    assert_input_rejected(&result, calls);
}

#[tokio::test]
async fn supplied_objects_are_not_coerced_to_unit() {
    for wire in [json!({}), json!({"unused": "supplied-value"})] {
        let (result, calls) = execute::<()>(wire, None).await;
        assert_input_rejected(&result, calls);
    }
}

#[tokio::test]
async fn no_parameters_preserves_scalar_values_without_template_parsing() {
    assert_roundtrip::<String>(json!("{{ $input.keep_this_as_data }}")).await;
    assert_roundtrip::<bool>(json!(true)).await;
    assert_roundtrip::<i64>(json!(42)).await;
}

#[tokio::test]
async fn integer_dispatch_uses_prepared_values_and_rejects_out_of_range_input() {
    let (result, calls) = execute::<i64>(json!(1.0), None).await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.node_output(&node_key!("root")), Some(&json!(1)));
    assert_eq!(calls, 1);
    for wire in [json!(1.5), json!(u64::MAX), json!("1")] {
        let (result, calls) = execute::<i64>(wire, None).await;
        assert_input_rejected(&result, calls);
    }
}

#[tokio::test]
async fn required_expression_parameter_reaches_typed_action() {
    let (registry, calls) =
        registry::<RequiredExpressionInput>(RootEcho::<RequiredExpressionInput>::metadata());
    let (engine, _) = make_engine(registry);
    let node = NodeDefinition::new(node_key!("root"), "Root", "test", "test.root_echo")
        .unwrap()
        .with_parameter("value", ParamValue::expression("{{ $input.value }}"));
    let workflow = make_workflow(vec![node], vec![]);

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            json!({"value": 7}),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        result.node_output(&node_key!("root")),
        Some(&json!({"value": 7}))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn named_parameters_cannot_be_discarded_for_scalar_roots() {
    let (result, calls) = execute::<()>(Value::Null, Some(json!("supplied-value"))).await;
    assert_input_rejected(&result, calls);
    let (result, calls) = execute::<String>(json!("input"), Some(json!("supplied-value"))).await;
    assert_input_rejected(&result, calls);
}

#[tokio::test]
async fn default_flow_connection_passes_the_whole_scalar_root() {
    let (registry, calls) = registry::<String>(RootEcho::<String>::metadata());
    let (engine, _) = make_engine(registry);
    let source = node_key!("source");
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(source.clone(), "Source", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(target.clone(), "Target", "test", "test.root_echo").unwrap(),
        ],
        vec![Connection::new(source, target.clone())],
    );
    let wire = json!("{{ $input.stays_literal }}");
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            wire.clone(),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.node_output(&target), Some(&wire));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn named_flow_port_is_rejected_before_scalar_input_can_be_lost() {
    use nebula_error::Classify;

    let (registry, calls) = registry::<String>(RootEcho::<String>::metadata());
    let (engine, _) = make_engine(registry);
    let source = node_key!("source");
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(source.clone(), "Source", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(target.clone(), "Target", "test", "test.root_echo").unwrap(),
        ],
        vec![Connection::new(source, target).with_to_port(port_key!("in"))],
    );
    let error = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            json!("scalar-input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().as_str(), "ENGINE:UNSUPPORTED_INPUT_PORT");
    assert_eq!(error.category(), nebula_error::ErrorCategory::Validation);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn declared_support_port_is_delivered_without_replacing_main_input() {
    let registry = support_registry(support_port(false, false, ConnectionFilter::new()));
    let (engine, _) = make_engine(registry);
    let source = node_key!("source");
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(source.clone(), "Source", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(target.clone(), "Target", "test", "test.support_echo").unwrap(),
        ],
        vec![Connection::new(source, target.clone()).with_to_port(port_key!("model"))],
    );
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            json!({"provider": "model-a"}),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        result.node_output(&target),
        Some(&json!([{"provider": "model-a"}]))
    );
}

#[tokio::test]
async fn required_support_port_rejects_missing_connection() {
    use nebula_error::Classify;

    let registry = support_registry(support_port(true, false, ConnectionFilter::new()));
    let (engine, _) = make_engine(registry);
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![NodeDefinition::new(target, "Target", "test", "test.support_echo").unwrap()],
        vec![],
    );
    let error = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            Value::Null,
            ExecutionBudget::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().as_str(), "ENGINE:MISSING_SUPPORT_INPUT");
}

#[tokio::test]
async fn single_support_port_rejects_multiple_connections() {
    use nebula_error::Classify;

    let registry = support_registry(support_port(false, false, ConnectionFilter::new()));
    let (engine, _) = make_engine(registry);
    let first = node_key!("first");
    let second = node_key!("second");
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(first.clone(), "First", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(second.clone(), "Second", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(target.clone(), "Target", "test", "test.support_echo").unwrap(),
        ],
        vec![
            Connection::new(first, target.clone()).with_to_port(port_key!("model")),
            Connection::new(second, target).with_to_port(port_key!("model")),
        ],
    );
    let error = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            Value::Null,
            ExecutionBudget::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().as_str(), "ENGINE:SUPPORT_INPUT_MULTIPLICITY");
}

#[tokio::test]
async fn support_port_filter_rejects_disallowed_action_type() {
    use nebula_error::Classify;

    let filter =
        ConnectionFilter::new().with_allowed_node_types(vec!["test.allowed_provider".to_owned()]);
    let registry = support_registry(support_port(false, false, filter));
    let (engine, _) = make_engine(registry);
    let source = node_key!("source");
    let target = node_key!("target");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(source.clone(), "Source", "test", "test.root_echo").unwrap(),
            NodeDefinition::new(target.clone(), "Target", "test", "test.support_echo").unwrap(),
        ],
        vec![Connection::new(source, target).with_to_port(port_key!("model"))],
    );
    let error = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            Value::Null,
            ExecutionBudget::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().as_str(), "ENGINE:SUPPORT_INPUT_FILTERED");
}
