//! Authored intent survives schema preparation, runtime dispatch, and repeated iterations.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionResult, FromWorkflowNode, StatefulAction,
};
use nebula_core::{Dependencies, action_key, node_key};
use nebula_engine::{ActionRegistry, ExecutionResult};
use nebula_execution::context::ExecutionBudget;
use nebula_schema::{
    Field, HasSchema, Rule, Schema, Transformer, ValidSchema, ValidationReport, field_key,
};
use nebula_workflow::{NodeDefinition, ParamValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{
    make_engine, make_workflow,
    root_input::{RootEcho, registry},
};

#[derive(Serialize, Deserialize, nebula_schema::Schema)]
struct RequiredInput {
    #[field(expression_required)]
    value: i64,
}

#[derive(Serialize, Deserialize, nebula_schema::Schema)]
struct TextInput {
    value: String,
}

#[derive(Serialize, Deserialize)]
struct ForbiddenInput {
    value: i64,
}

impl HasSchema for ForbiddenInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::number(field_key!("value")).integer().no_expression())
            .build()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct OpaqueInput(Value);

impl HasSchema for OpaqueInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::object(field_key!("payload")))
            .build()
    }
}

#[derive(Serialize, Deserialize)]
struct TransformedInput {
    label: String,
    literal: String,
}

static ITERATION_INPUT_DECODES: AtomicUsize = AtomicUsize::new(0);

struct IterationInput {
    label: String,
    literal: String,
}

impl<'de> Deserialize<'de> for IterationInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ITERATION_INPUT_DECODES.fetch_add(1, Ordering::SeqCst);
        let input = TransformedInput::deserialize(deserializer)?;
        Ok(Self {
            label: input.label,
            literal: input.literal,
        })
    }
}

impl HasSchema for IterationInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        TransformedInput::schema()
    }
}

impl HasSchema for TransformedInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(
                Field::string(field_key!("label"))
                    .required()
                    .read_alias("legacy_label")?
                    .with_transformer(Transformer::regex("^.(.*)$", 1)?)
                    .with_rule(
                        Rule::one_of([json!("ready")]).expect("bounded fixture rule must admit"),
                    ),
            )
            .add(
                Field::string(field_key!("literal"))
                    .required()
                    .with_transformer(Transformer::regex("^.(.*)$", 1)?),
            )
            .build()
    }
}

async fn execute_parameters<T>(
    wire: Value,
    parameters: &[(&str, ParamValue)],
) -> (ExecutionResult, usize)
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let (registry, calls) = registry::<T>(RootEcho::<T>::metadata());
    let (engine, _) = make_engine(registry);
    let mut node =
        NodeDefinition::new(node_key!("proof"), "Proof", "test", "test.root_echo").unwrap();
    for (key, value) in parameters {
        node = node.with_parameter(*key, value.clone());
    }
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &make_workflow(vec![node], vec![]),
            wire,
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    (result, calls.load(Ordering::SeqCst))
}

fn assert_rejected_before_execute(result: &ExecutionResult, calls: usize) {
    assert!(!result.is_success(), "{result:?}");
    let error = &result.node_errors[&node_key!("proof")];
    assert!(
        error.starts_with("parameter resolution failed for node proof, param '"),
        "{error}"
    );
    assert!(error.ends_with("input schema validation failed"), "{error}");
    assert_eq!(result.node_output(&node_key!("proof")), None);
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn required_expression_rejects_raw_and_named_literals() {
    for parameters in [vec![], vec![("value", ParamValue::literal(json!(7)))]] {
        let (result, calls) =
            execute_parameters::<RequiredInput>(json!({"value": 7}), &parameters).await;
        assert_rejected_before_execute(&result, calls);
    }
}

#[tokio::test]
async fn forbidden_expression_is_not_authorized_by_a_resolved_number() {
    let (result, calls) = execute_parameters::<ForbiddenInput>(
        json!({"value": 7}),
        &[("value", ParamValue::expression("{{ $input.value }}"))],
    )
    .await;
    assert_rejected_before_execute(&result, calls);
}

#[tokio::test]
async fn any_and_undeclared_opaque_fields_do_not_authorize_expressions() {
    let parameters = [("undeclared", ParamValue::expression("{{ 7 }}"))];
    let (result, calls) = execute_parameters::<Value>(Value::Null, &parameters).await;
    assert_rejected_before_execute(&result, calls);
    let (result, calls) = execute_parameters::<OpaqueInput>(Value::Null, &parameters).await;
    assert_rejected_before_execute(&result, calls);
}

#[tokio::test]
async fn template_lone_expression_stays_string_and_output_is_never_reparsed() {
    for (parameter, wire, expected) in [
        (ParamValue::template("{{ 7 }}"), Value::Null, json!("7")),
        (
            ParamValue::expression("{{ $input }}"),
            json!("{{ 7 }}"),
            json!("{{ 7 }}"),
        ),
        (
            ParamValue::literal(json!("{{ 7 }}")),
            Value::Null,
            json!("{{ 7 }}"),
        ),
    ] {
        let (result, calls) = execute_parameters::<TextInput>(wire, &[("value", parameter)]).await;
        assert!(result.is_success(), "{result:?}");
        assert_eq!(
            result.node_output(&node_key!("proof")),
            Some(&json!({"value": expected}))
        );
        assert_eq!(calls, 1);
    }
}

#[tokio::test]
async fn expression_and_literal_transforms_are_consumed_once() {
    let (result, calls) = execute_parameters::<TransformedInput>(
        json!({"label": "xready"}),
        &[
            ("legacy_label", ParamValue::expression("{{ $input.label }}")),
            ("literal", ParamValue::literal(json!("xdata"))),
        ],
    )
    .await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        result.node_output(&node_key!("proof")),
        Some(&json!({"label": "ready", "literal": "data"}))
    );
    assert_eq!(calls, 1);
}

struct IterationProbe;

impl Action for IterationProbe {
    type Input = IterationInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.iteration_probe"),
            nebula_action::metadata_name!("Iteration probe"),
            "Observe the same prepared input each iteration",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl FromWorkflowNode for IterationProbe {
    type Error = ActionError;

    async fn from_workflow_node<'a>(
        _: &'a NodeDefinition,
        _: &'a dyn ActionContext,
    ) -> Result<Self, ActionError> {
        Ok(Self)
    }
}

impl StatefulAction for IterationProbe {
    type State = Vec<(String, String)>;

    fn init_state(&self) -> Self::State {
        Vec::new()
    }

    async fn execute(
        &self,
        input: &IterationInput,
        state: &mut Self::State,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        state.push((input.label.clone(), input.literal.clone()));
        let output = serde_json::to_value(state.as_slice()).unwrap();
        Ok(if state.len() == 3 {
            ActionResult::success(output)
        } else {
            ActionResult::continue_with(output, None)
        })
    }
}

#[tokio::test]
async fn stateful_iterations_reuse_the_resolved_proof() {
    ITERATION_INPUT_DECODES.store(0, Ordering::SeqCst);
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateful_factory::<IterationProbe>()
        .unwrap();
    let (engine, _) = make_engine(registry);
    let node = NodeDefinition::new(
        node_key!("iterations"),
        "Iterations",
        "test",
        "test.iteration_probe",
    )
    .unwrap()
    .with_parameter("legacy_label", ParamValue::expression("{{ $input.label }}"))
    .with_parameter("literal", ParamValue::literal(json!("xdata")));
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &make_workflow(vec![node], vec![]),
            json!({"label": "xready"}),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        result.node_output(&node_key!("iterations")),
        Some(&json!([
            ["ready", "data"],
            ["ready", "data"],
            ["ready", "data"]
        ]))
    );
    assert_eq!(ITERATION_INPUT_DECODES.load(Ordering::SeqCst), 1);
}
