//! Every JSON-erased input adapter crosses the same schema preparation boundary.

use std::{assert_matches, error::Error, sync::OnceLock};

use futures::{Stream, stream};
use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionHandle, ActionInput, ActionResult,
    AgentAction, AgentActionAdapter, AgentHandle, ControlAction, ControlActionAdapter,
    ControlHandle, ControlOutcome, FromWorkflowNode, GenericAgentFactory, GenericControlFactory,
    GenericStatefulFactory, GenericStatelessFactory, GenericStreamFactory, InstanceFactory,
    StatefulAction, StatefulActionAdapter, StatefulHandle, StatelessAction, StatelessActionAdapter,
    StatelessHandle, StreamAction, TestContextBuilder,
};
use nebula_core::{Dependencies, action_key, node_key};
use nebula_schema::{
    AuthoredValue, Field, HasSchema, Predicate, Rule, Schema, Transformer, ValidSchema,
    ValidationReport, field_key,
};
use nebula_workflow::NodeDefinition;
use rstest::rstest;
use serde::Deserialize;
use serde_json::{Value, json};

const SECRET: &str = "ACTION_PREPARED_SECRET_CANARY";
const LITERAL: &str = "{{ $input.not_an_expression }}";
const SERDE_CANARY: &str = "ACTION_CUSTOM_DESERIALIZER_SECRET_CANARY";

#[derive(Deserialize)]
struct PreparedInput {
    label: String,
    token: String,
    approved: bool,
    #[serde(deserialize_with = "deserialize_literal")]
    literal: String,
}

fn deserialize_literal<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let literal = String::deserialize(deserializer)?;
    if literal == SERDE_CANARY {
        return Err(serde::de::Error::custom(format!(
            "invalid literal {literal}"
        )));
    }
    Ok(literal)
}

impl HasSchema for PreparedInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(
                Field::string(field_key!("label"))
                    .required()
                    .read_alias("legacy_label")?
                    .with_transformer(Transformer::Trim)
                    .with_transformer(Transformer::regex("^.(.*)$", 1)?)
                    .with_rule(
                        Rule::one_of([json!("ready")]).expect("single bounded rule must admit"),
                    ),
            )
            .add(
                Field::secret(field_key!("token"))
                    .required()
                    .read_alias("legacy_token")?,
            )
            .add(Field::boolean(field_key!("approved")).required())
            .add(Field::string(field_key!("literal")).required())
            .root_rule(
                Rule::predicate(Predicate::eq("/approved", json!(true)).unwrap())
                    .expect("single bounded predicate must admit"),
            )
            .build()
    }
}

fn wire(aliases: bool) -> Value {
    let label = if aliases { "legacy_label" } else { "label" };
    let token = if aliases { "legacy_token" } else { "token" };
    json!({label: "  xready  ", token: SECRET, "approved": true, "literal": LITERAL})
}

fn observed(input: &PreparedInput) -> Value {
    json!({
        "label": input.label,
        "secret_exposed": input.token == SECRET,
        "approved": input.approved,
        "literal": input.literal,
    })
}

struct Probe;

impl Action for Probe {
    type Input = PreparedInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.input_probe"),
            nebula_action::metadata_name!("Input probe"),
            "Observe prepared input at the action boundary",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl FromWorkflowNode for Probe {
    type Error = ActionError;

    async fn from_workflow_node<'a>(
        _node: &'a NodeDefinition,
        _context: &'a dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

impl StatelessAction for Probe {
    async fn execute(
        &self,
        input: PreparedInput,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(observed(&input)))
    }
}

impl StatefulAction for Probe {
    type State = u32;

    fn init_state(&self) -> u32 {
        0
    }

    async fn execute(
        &self,
        input: &PreparedInput,
        state: &mut u32,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        *state += 1;
        Ok(ActionResult::success(observed(input)))
    }
}

impl StreamAction for Probe {
    type Chunk = Value;

    fn open_stream(
        &self,
        input: PreparedInput,
        _context: &(impl ActionContext + ?Sized),
    ) -> impl Stream<Item = Result<Value, ActionError>> + Send {
        stream::once(std::future::ready(Ok(observed(&input))))
    }

    fn init(&self) -> Value {
        Value::Null
    }

    fn fold(&self, _accumulator: Value, chunk: Value) -> Value {
        chunk
    }
}

impl AgentAction for Probe {
    type Turn = Value;

    fn init_turn(&self, input: &PreparedInput) -> Value {
        observed(input)
    }

    async fn step(
        &self,
        turn: &mut Value,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(turn.clone()))
    }
}

impl ControlAction for Probe {
    async fn evaluate(
        &self,
        input: PreparedInput,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        Ok(ControlOutcome::Pass {
            output: observed(&input),
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum IngressPath {
    Stateless,
    Instance,
    Stateful,
    Stream,
    Agent,
    Control,
    StatelessAdapter,
    StatefulAdapter,
    ControlAdapter,
    AgentAdapter,
}

fn factory(adapter: IngressPath) -> Box<dyn ActionFactory> {
    match adapter {
        IngressPath::Stateless => Box::new(GenericStatelessFactory::<Probe>::new().unwrap()),
        IngressPath::Instance => Box::new(InstanceFactory::new(Probe::metadata(), Probe).unwrap()),
        IngressPath::Stateful => Box::new(GenericStatefulFactory::<Probe>::new().unwrap()),
        IngressPath::Stream => Box::new(GenericStreamFactory::<Probe>::new().unwrap()),
        IngressPath::Agent => Box::new(GenericAgentFactory::<Probe>::new().unwrap()),
        IngressPath::Control => Box::new(GenericControlFactory::<Probe>::new().unwrap()),
        _ => panic!("direct adapters do not use factories"),
    }
}

async fn dispatch_proof(
    factory: Box<dyn ActionFactory>,
    input: ActionInput,
) -> Result<Value, ActionError> {
    let context = TestContextBuilder::new().build();
    let node =
        NodeDefinition::new(node_key!("probe"), "Probe", "test", "test.input_probe").unwrap();
    let mut state = json!(0);
    let result = match factory.instantiate(&node, &context).await? {
        ActionHandle::Stateless(handle) => {
            let input = handle.prepare_input(input)?;
            handle.dispatch(input, &context).await
        },
        ActionHandle::Stateful(handle) => {
            let input = handle.prepare_input(input)?;
            handle.dispatch(&input, &mut state, &context).await
        },
        ActionHandle::Stream(handle) => {
            let input = handle.prepare_input(input)?;
            handle.dispatch(input, &context).await
        },
        ActionHandle::Control(handle) => {
            let input = handle.prepare_input(input)?;
            handle.dispatch(input, &context).await
        },
        ActionHandle::Agent(handle) => {
            let input = handle.prepare_input(input)?;
            return handle.init_turn(input);
        },
        _ => panic!("input probe must instantiate an input-consuming handle"),
    };
    if result.is_err() {
        assert_eq!(
            state,
            json!(0),
            "rejected proof must not execute a stateful action"
        );
    }
    let ActionResult::Success { output } = result? else {
        panic!("input probe returns its observation directly");
    };
    Ok(output.into_value().unwrap())
}

async fn dispatch(adapter: IngressPath, input: Value) -> Result<Value, ActionError> {
    let context = TestContextBuilder::new().build();
    let mut state = json!(0);
    let result = match adapter {
        IngressPath::StatelessAdapter => {
            let handler = StatelessActionAdapter::new(Probe).unwrap();
            let input = handler.prepare_input(ActionInput::Raw(input))?;
            handler.dispatch(input, &context).await
        },
        IngressPath::StatefulAdapter => {
            let handler = StatefulActionAdapter::new(Probe).unwrap();
            let input = handler.prepare_input(ActionInput::Raw(input))?;
            handler.dispatch(&input, &mut state, &context).await
        },
        IngressPath::ControlAdapter => {
            let handler = ControlActionAdapter::new(Probe).unwrap();
            let input = handler.prepare_input(ActionInput::Raw(input))?;
            handler.dispatch(input, &context).await
        },
        IngressPath::AgentAdapter => {
            let handler = AgentActionAdapter::new(Probe).unwrap();
            let input = handler.prepare_input(ActionInput::Raw(input))?;
            return handler.init_turn(input);
        },
        _ => {
            let factory = factory(adapter);
            let node = NodeDefinition::new(node_key!("probe"), "Probe", "test", "test.input_probe")
                .unwrap();
            let input = ActionInput::Raw(input);
            match factory.instantiate(&node, &context).await? {
                ActionHandle::Stateless(handle) => {
                    let input = handle.prepare_input(input)?;
                    handle.dispatch(input, &context).await
                },
                ActionHandle::Stateful(handle) => {
                    let input = handle.prepare_input(input)?;
                    handle.dispatch(&input, &mut state, &context).await
                },
                ActionHandle::Stream(handle) => {
                    let input = handle.prepare_input(input)?;
                    handle.dispatch(input, &context).await
                },
                ActionHandle::Control(handle) => {
                    let input = handle.prepare_input(input)?;
                    handle.dispatch(input, &context).await
                },
                ActionHandle::Agent(handle) => {
                    let input = handle.prepare_input(input)?;
                    return handle.init_turn(input);
                },
                _ => panic!("input probe must instantiate an input-consuming handle"),
            }
        },
    };
    if result.is_err() {
        assert_eq!(
            state,
            json!(0),
            "rejected input must not execute a stateful action"
        );
    }
    let ActionResult::Success { output } = result? else {
        panic!("input probe returns its observation directly");
    };
    Ok(output.into_value().unwrap())
}

#[rstest]
#[tokio::test]
async fn adapters_prepare_aliases_transforms_secrets_and_literals_once(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control,
        IngressPath::StatelessAdapter,
        IngressPath::StatefulAdapter,
        IngressPath::ControlAdapter,
        IngressPath::AgentAdapter
    )]
    adapter: IngressPath,
    #[values(false, true)] aliases: bool,
) {
    assert_eq!(
        dispatch(adapter, wire(aliases)).await.unwrap(),
        json!({"label": "ready", "secret_exposed": true, "approved": true, "literal": LITERAL}),
    );
}

#[rstest]
#[tokio::test]
async fn adapters_reject_field_and_root_rules_before_action_code(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control,
        IngressPath::StatelessAdapter,
        IngressPath::StatefulAdapter,
        IngressPath::ControlAdapter,
        IngressPath::AgentAdapter
    )]
    adapter: IngressPath,
    #[values(false, true)] root_rule: bool,
) {
    let mut input = wire(false);
    if root_rule {
        input["approved"] = json!(false);
    } else {
        input["label"] = json!("rejected");
    }
    let error = dispatch(adapter, input).await.unwrap_err();
    assert_matches!(error, ActionError::Validation { field: "input", .. });
    assert!(!format!("{error} {error:?}").contains(SECRET));
}

#[rstest]
#[tokio::test]
async fn adapters_enforce_rust_input_constraints_without_exposing_serde_sources(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control,
        IngressPath::StatelessAdapter,
        IngressPath::StatefulAdapter,
        IngressPath::ControlAdapter,
        IngressPath::AgentAdapter
    )]
    adapter: IngressPath,
) {
    let mut input = wire(true);
    input["literal"] = json!(SERDE_CANARY);
    let error = dispatch(adapter, input).await.unwrap_err();
    assert_redacted_deserializer_error(&error);
}

fn assert_redacted_deserializer_error(error: &ActionError) {
    assert_matches!(error, ActionError::Validation { field: "input", .. });
    let mut cause: &dyn Error = error;
    loop {
        assert!(!format!("{cause} {cause:?}").contains(SERDE_CANARY));
        assert!(cause.downcast_ref::<serde_json::Error>().is_none());
        match cause.source() {
            Some(next) => cause = next,
            None => break,
        }
    }
}

#[rstest]
#[tokio::test]
async fn resolved_proof_decode_errors_never_publish_serde_sources(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control
    )]
    adapter: IngressPath,
) {
    let mut input = wire(true);
    input["literal"] = json!(SERDE_CANARY);
    let factory = factory(adapter);
    let proof = factory
        .metadata()
        .base()
        .schema()
        .validate(AuthoredValue::from_data(input).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    let error = dispatch_proof(factory, ActionInput::Resolved(proof))
        .await
        .unwrap_err();
    assert_redacted_deserializer_error(&error);
}

#[rstest]
#[tokio::test]
async fn adapters_consume_matching_proofs_without_repeating_preparation(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control
    )]
    adapter: IngressPath,
) {
    let factory = factory(adapter);
    let schema = factory.metadata().base().schema();
    let proof = schema
        .validate(AuthoredValue::from_data(wire(true)).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(
        dispatch_proof(factory, ActionInput::Resolved(proof))
            .await
            .unwrap(),
        json!({"label": "ready", "secret_exposed": true, "approved": true, "literal": LITERAL})
    );
}

#[rstest]
#[tokio::test]
async fn adapters_require_the_complete_schema_not_just_the_field_types(
    #[values(
        IngressPath::Stateless,
        IngressPath::Instance,
        IngressPath::Stateful,
        IngressPath::Stream,
        IngressPath::Agent,
        IngressPath::Control
    )]
    adapter: IngressPath,
) {
    let foreign_schema = Schema::builder()
        .add(Field::string(field_key!("label")).required())
        .add(Field::secret(field_key!("token")).required())
        .add(Field::boolean(field_key!("approved")).required())
        .add(Field::string(field_key!("literal")).required())
        .build()
        .unwrap();
    let proof = foreign_schema
        .validate(AuthoredValue::from_data(wire(false)).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    let error = dispatch_proof(factory(adapter), ActionInput::Resolved(proof))
        .await
        .unwrap_err();
    assert_matches!(error, ActionError::Validation { field: "input", .. });
    assert!(
        error
            .to_string()
            .contains("resolved input schema does not match the declared input schema")
    );
    assert!(!format!("{error:?}").contains(SECRET));
}

#[tokio::test]
async fn resolved_proof_rejects_an_independently_admitted_equal_schema() {
    let factory = factory(IngressPath::Stateless);
    let foreign_schema = PreparedInput::schema().unwrap();
    assert_eq!(&foreign_schema, factory.metadata().base().schema());
    assert!(!foreign_schema.ptr_eq(factory.metadata().base().schema()));
    let proof = foreign_schema
        .validate(AuthoredValue::from_data(wire(false)).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();

    let error = dispatch_proof(factory, ActionInput::Resolved(proof))
        .await
        .unwrap_err();

    assert_matches!(error, ActionError::Validation { field: "input", .. });
    assert!(
        error
            .to_string()
            .contains("resolved input schema does not match the declared input schema")
    );
}

#[test]
fn action_input_tokens_debug_never_discloses_values() {
    let raw = ActionInput::Raw(json!({"raw": SECRET, "expression": SERDE_CANARY}));
    assert_eq!(format!("{raw:?}"), "ActionInput::Raw(..)");
    let schema = PreparedInput::schema().unwrap();
    let proof = schema
        .validate(AuthoredValue::from_data(wire(false)).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(
        format!("{:?}", ActionInput::Resolved(proof)),
        "ActionInput::Resolved(..)"
    );

    let adapter = StatefulActionAdapter::new(Probe).unwrap();
    let prepared = adapter
        .prepare_input(ActionInput::Raw(wire(false)))
        .unwrap();
    assert_eq!(prepared.schema(), &schema);
    let rendered = format!("{prepared:?}");
    assert_eq!(rendered, "PreparedActionInput { .. }");
    assert!(!rendered.contains(SECRET));
}

#[tokio::test]
async fn prepared_input_is_bound_to_exact_handle_across_shared_types_and_contracts() {
    let context = TestContextBuilder::new().build();
    let node = NodeDefinition::new(
        node_key!("cross_handle"),
        "Cross handle",
        "test",
        "test.input_probe",
    )
    .unwrap();
    let factory = GenericStatelessFactory::<Probe>::new().unwrap();
    let ActionHandle::Stateless(first) = factory.instantiate(&node, &context).await.unwrap() else {
        panic!("probe factory must produce a stateless handle");
    };
    let ActionHandle::Stateless(second) = factory.instantiate(&node, &context).await.unwrap()
    else {
        panic!("probe factory must produce a stateless handle");
    };
    assert_eq!(
        first.metadata().base().schema(),
        second.metadata().base().schema()
    );
    let other_factory = InstanceFactory::new(
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.input_probe.other"),
            nebula_action::metadata_name!("Other input probe"),
            "A distinct action contract over the same Rust input type",
        ),
        Probe,
    )
    .unwrap();
    let ActionHandle::Stateless(other_contract) =
        other_factory.instantiate(&node, &context).await.unwrap()
    else {
        panic!("instance factory must produce a stateless handle");
    };
    assert_ne!(
        first.metadata().base().key(),
        other_contract.metadata().base().key()
    );
    assert_eq!(
        first.metadata().base().schema(),
        other_contract.metadata().base().schema()
    );

    let prepared = first.prepare_input(ActionInput::Raw(wire(false))).unwrap();
    let error = second.dispatch(prepared, &context).await.unwrap_err();
    assert_matches!(error, ActionError::Fatal { .. });
    assert!(
        error
            .source()
            .expect("contract mismatch retains its typed source")
            .to_string()
            .contains("prepared input belongs to another action contract")
    );

    let prepared = first.prepare_input(ActionInput::Raw(wire(false))).unwrap();
    let error = other_contract
        .dispatch(prepared, &context)
        .await
        .unwrap_err();
    assert_matches!(error, ActionError::Fatal { .. });
    assert!(
        error
            .source()
            .expect("contract mismatch retains its typed source")
            .to_string()
            .contains("prepared input belongs to another action contract")
    );
}
