use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, OutputPort, StatelessAction,
};
use nebula_core::{
    ActionKey, ArtifactSetDigest, Dependencies, WorkflowId, WorkflowVersionId, node_key,
};
use nebula_metadata::PluginManifest;
use nebula_plugin::{
    ExecutablePlanRevision, PlanRegistryCompatibilityError, Plugin, PluginRegistry,
    RecordedExecutablePlanRevisionV1, ResolvedPlugin, RuntimeContractVersion,
};
use nebula_schema::{Field, ObjectField, Schema, SecretField, ValidSchema, ValuePath, field_key};
use nebula_workflow::{NodeDefinition, ParamValue, WorkflowBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

struct ContractAction;

impl ContractAction {
    fn factory_with_input_schema(input_schema: ValidSchema) -> Arc<dyn ActionFactory> {
        Self::with_contract(
            "demo.echo",
            input_schema,
            ValidSchema::empty(),
            nebula_action::effect::ActionEffectContract::NoExternalEffects,
            None,
        )
    }

    fn with_contract(
        key: &str,
        input_schema: ValidSchema,
        output_schema: ValidSchema,
        effect_contract: nebula_action::effect::ActionEffectContract,
        outputs: Option<Vec<OutputPort>>,
    ) -> Arc<dyn ActionFactory> {
        INPUT_SCHEMA.with(|schema| *schema.borrow_mut() = Some(input_schema));
        OUTPUT_SCHEMA.with(|schema| *schema.borrow_mut() = Some(output_schema));
        let mut draft = ActionMetadataDraft::new(
            ActionKey::new(key).expect("fixture action key is valid"),
            nebula_action::metadata_name!("Echo"),
            "Graph-v1 contract fixture",
        )
        .with_effect_contract(effect_contract);
        if let Some(outputs) = outputs {
            draft = draft.with_outputs(outputs);
        }
        let factory = InstanceFactory::new(draft, ContractImplementation)
            .expect("graph contract admits through its structural factory");
        INPUT_SCHEMA.with(|schema| *schema.borrow_mut() = None);
        OUTPUT_SCHEMA.with(|schema| *schema.borrow_mut() = None);
        Arc::new(factory)
    }
}

thread_local! {
    static INPUT_SCHEMA: RefCell<Option<ValidSchema>> = const { RefCell::new(None) };
    static OUTPUT_SCHEMA: RefCell<Option<ValidSchema>> = const { RefCell::new(None) };
}

#[derive(Deserialize)]
struct ContractInput(Value);

impl nebula_schema::HasSchema for ContractInput {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        Ok(INPUT_SCHEMA.with(|schema| {
            schema
                .borrow()
                .clone()
                .expect("input schema is installed during fixture admission")
        }))
    }
}

#[derive(Serialize)]
struct ContractOutput(Value);

impl nebula_schema::HasSchema for ContractOutput {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        Ok(OUTPUT_SCHEMA.with(|schema| {
            schema
                .borrow()
                .clone()
                .expect("output schema is installed during fixture admission")
        }))
    }
}

struct ContractImplementation;

impl Action for ContractImplementation {
    type Input = ContractInput;
    type Output = ContractOutput;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            nebula_core::action_key!("fixture.contract"),
            nebula_action::metadata_name!("Fixture contract"),
            "Graph contract fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ContractImplementation {
    async fn execute(
        &self,
        input: ContractInput,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<ContractOutput>, ActionError> {
        Ok(ActionResult::success(ContractOutput(input.0)))
    }
}

struct ContractPlugin {
    manifest: PluginManifest,
    action: Arc<dyn ActionFactory>,
}

impl std::fmt::Debug for ContractPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContractPlugin")
            .field("key", self.manifest.key())
            .finish()
    }
}

impl ContractPlugin {
    fn new(input_schema: ValidSchema) -> Self {
        Self {
            manifest: PluginManifest::builder("demo", "Demo")
                .build()
                .expect("fixture manifest is valid"),
            action: ContractAction::factory_with_input_schema(input_schema),
        }
    }
}

impl Plugin for ContractPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![Arc::clone(&self.action)]
    }
}

fn frozen(input_schema: ValidSchema, artifact: u8) -> nebula_plugin::FrozenPluginRegistry {
    let plugin = Arc::new(
        ResolvedPlugin::from(ContractPlugin::new(input_schema)).expect("fixture plugin resolves"),
    );
    let mut registry = PluginRegistry::new();
    registry
        .register(plugin)
        .expect("fixture plugin registers once");
    registry
        .freeze(
            ArtifactSetDigest::from_bytes([artifact; 32]),
            "1.0.0"
                .parse::<RuntimeContractVersion>()
                .expect("fixture runtime contract version is valid"),
        )
        .expect("fixture registry freezes")
}

fn workflow_with_variables(order: &[(&str, i64)]) -> nebula_workflow::WorkflowDefinition {
    let node = NodeDefinition::new(node_key!("echo"), "Echo", "demo", "echo")
        .expect("fixture node is valid");
    let mut builder = WorkflowBuilder::new("Graph plan")
        .id(WorkflowId::from_bytes([0x11; 16]))
        .add_node(node);
    for (key, value) in order {
        builder = builder.variable(*key, serde_json::json!(value));
    }
    builder.build().expect("fixture workflow is valid")
}

#[test]
fn readable_legacy_plans_do_not_gain_implicit_execution_permission() {
    use sha2::{Digest, Sha256};
    let registry = frozen(ValidSchema::empty(), 0x81);
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::new(), &workflow_with_variables(&[]))
        .unwrap();
    let mut record = serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    record["compiler_version"] = serde_json::json!(1);
    record["canonical_hash_version"] = serde_json::json!(1);
    record.as_object_mut().unwrap().remove("claimed_id");
    for action in record["content"]["actions"].as_array_mut().unwrap() {
        action.as_object_mut().unwrap().remove("effect_contract");
    }
    let canonical = nebula_schema::canonical_json_v1(&record).unwrap();
    let domain = b"nebula.executable-plan.graph.v1";
    let mut hash = Sha256::new();
    hash.update([1]);
    hash.update((domain.len() as u64).to_be_bytes());
    hash.update(domain);
    hash.update([2]);
    hash.update((canonical.len() as u64).to_be_bytes());
    hash.update(canonical);
    let digest: [u8; 32] = hash.finalize().into();
    record["claimed_id"] =
        serde_json::to_value(nebula_core::ExecutablePlanRevisionId::from_bytes(digest)).unwrap();
    let recorded: RecordedExecutablePlanRevisionV1 = serde_json::from_value(record).unwrap();
    let legacy = ExecutablePlanRevision::try_from(recorded).unwrap();
    assert!(matches!(
        legacy.validate_against(&registry),
        Err(PlanRegistryCompatibilityError::UnsupportedEffectProtocol)
    ));
}

#[test]
fn graph_v3_records_remain_executable_against_identical_legacy_contracts() {
    use sha2::{Digest, Sha256};
    let registry = frozen(ValidSchema::empty(), 0x82);
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::new(), &workflow_with_variables(&[]))
        .unwrap();
    let mut record = serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    record["compiler_version"] = serde_json::json!(3);
    record.as_object_mut().unwrap().remove("claimed_id");
    let canonical = nebula_schema::canonical_json_v1(&record).unwrap();
    let domain = b"nebula.executable-plan.graph.v2";
    let mut hash = Sha256::new();
    hash.update([1]);
    hash.update((domain.len() as u64).to_be_bytes());
    hash.update(domain);
    hash.update([2]);
    hash.update((canonical.len() as u64).to_be_bytes());
    hash.update(canonical);
    let digest: [u8; 32] = hash.finalize().into();
    record["claimed_id"] =
        serde_json::to_value(nebula_core::ExecutablePlanRevisionId::from_bytes(digest)).unwrap();
    let encoded = serde_json::to_vec(&record).unwrap();
    let legacy =
        ExecutablePlanRevision::try_from_recorded_v1(serde_json::from_slice(&encoded).unwrap())
            .unwrap();
    legacy.validate_against(&registry).unwrap();
    assert_eq!(
        serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&legacy)).unwrap(),
        record
    );
    assert_ne!(legacy.id(), plan.id());
}

#[test]
fn intrinsic_error_edge_uses_runtime_payload_schema_and_survives_record_roundtrip() {
    struct ErrorFixture {
        manifest: PluginManifest,
        actions: Vec<Arc<dyn ActionFactory>>,
    }
    impl std::fmt::Debug for ErrorFixture {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ErrorFixture").finish_non_exhaustive()
        }
    }
    impl Plugin for ErrorFixture {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }
        fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
            self.actions.clone()
        }
    }
    let mut source_outputs = nebula_action::port::default_output_ports();
    source_outputs.push(OutputPort::error(nebula_core::port_key!("error")));
    let source = ContractAction::with_contract(
        "demo.source",
        ValidSchema::empty(),
        Schema::builder()
            .add(Field::string(field_key!("success_only")).required())
            .build()
            .unwrap(),
        nebula_action::effect::ActionEffectContract::NoExternalEffects,
        Some(source_outputs),
    );
    let target = ContractAction::with_contract(
        "demo.target",
        nebula_schema::schema_of::<nebula_workflow::ErrorPortPayload>()
            .expect("valid test catalog definition"),
        ValidSchema::empty(),
        nebula_action::effect::ActionEffectContract::NoExternalEffects,
        None,
    );
    let incompatible = ContractAction::with_contract(
        "demo.incompatible",
        Schema::builder()
            .add(Field::string(field_key!("success_only")).required())
            .build()
            .unwrap(),
        ValidSchema::empty(),
        nebula_action::effect::ActionEffectContract::NoExternalEffects,
        None,
    );
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(
            ResolvedPlugin::from(ErrorFixture {
                manifest: PluginManifest::builder("demo", "Error fixture")
                    .build()
                    .unwrap(),
                actions: vec![source, target, incompatible],
            })
            .unwrap(),
        ))
        .unwrap();
    let frozen = plugins
        .freeze(
            ArtifactSetDigest::from_bytes([0x84; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let mut workflow = WorkflowBuilder::new("Intrinsic error edge")
        .add_node(NodeDefinition::new(node_key!("source"), "Source", "demo", "source").unwrap())
        .add_node(
            NodeDefinition::new(node_key!("handler"), "Handler", "demo", "target")
                .unwrap()
                .with_parameter(
                    "error",
                    ParamValue::reference(
                        node_key!("source"),
                        ValuePath::from_pointer("/error").unwrap(),
                    ),
                )
                .with_parameter("node_id", ParamValue::literal(serde_json::json!("source"))),
        )
        .build()
        .unwrap();
    workflow.connections.push(
        nebula_workflow::Connection::new(node_key!("source"), node_key!("handler"))
            .with_from_port(nebula_core::port_key!("error")),
    );
    let plan = frozen
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x85; 16]), &workflow)
        .unwrap_or_else(|error| panic!("error edge must compile: {:?}", error.diagnostics()));
    let bytes = serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    let recorded: RecordedExecutablePlanRevisionV1 = serde_json::from_slice(&bytes).unwrap();
    let loaded = ExecutablePlanRevision::try_from(recorded).unwrap();
    let graph = loaded.execution_graph().unwrap();
    assert_eq!(
        graph.connections()[0].from_port.as_ref().unwrap().as_str(),
        "error"
    );
    assert!(graph.connections()[0].to_port.is_none());
    assert_eq!(loaded.id(), plan.id());
    let mut wrong_reference = workflow.clone();
    wrong_reference.nodes[1].parameters.insert(
        "error".into(),
        ParamValue::reference(
            node_key!("source"),
            ValuePath::from_pointer("/success_only").unwrap(),
        ),
    );
    let error = frozen
        .compile_graph_v1(WorkflowVersionId::new(), &wrong_reference)
        .unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code().ends_with("INVALID_REFERENCE_CONTRACT"))
    );
    let mut support = workflow.clone();
    support.connections[0].to_port = Some(nebula_core::port_key!("support"));
    let error = frozen
        .compile_graph_v1(WorkflowVersionId::new(), &support)
        .unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code().ends_with("UNSUPPORTED_TARGET_PORT"))
    );
    workflow.nodes[1] =
        NodeDefinition::new(node_key!("handler"), "Handler", "demo", "incompatible")
            .unwrap()
            .with_parameter(
                "success_only",
                ParamValue::literal(serde_json::json!("fixture")),
            );
    let error = frozen
        .compile_graph_v1(WorkflowVersionId::new(), &workflow)
        .unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code().ends_with("SCHEMA_INCOMPATIBLE"))
    );
}

fn workflow_with_parameter(value: Option<ParamValue>) -> nebula_workflow::WorkflowDefinition {
    let mut node = NodeDefinition::new(node_key!("echo"), "Echo", "demo", "echo")
        .expect("fixture node is valid");
    if let Some(value) = value {
        node = node.with_parameter("value", value);
    }
    WorkflowBuilder::new("Graph parameter plan")
        .id(WorkflowId::from_bytes([0x12; 16]))
        .add_node(node)
        .build()
        .expect("fixture workflow is valid")
}

#[test]
fn plan_roundtrip_and_exact_registry_compatibility_are_checked() {
    let registry = frozen(ValidSchema::empty(), 0x22);
    let workflow = workflow_with_variables(&[("b", 2), ("a", 1)]);
    let version_id = WorkflowVersionId::from_bytes([0x33; 16]);

    let plan = registry
        .compile_graph_v1(version_id, &workflow)
        .expect("closed Graph-v1 fixture compiles");
    assert_eq!(plan.workflow_version_id(), version_id);
    plan.validate_against(&registry)
        .expect("originating frozen registry is compatible");

    let recorded = RecordedExecutablePlanRevisionV1::from(&plan);
    let wire = serde_json::to_vec(&recorded).expect("recorded plan serializes");
    let decoded: RecordedExecutablePlanRevisionV1 =
        serde_json::from_slice(&wire).expect("recorded plan decodes");
    let loaded = ExecutablePlanRevision::try_from(decoded).expect("record integrity is checked");
    assert_eq!(loaded.id(), plan.id());
    loaded
        .validate_against(&registry)
        .expect("loaded plan remains compatible");

    let reordered = workflow_with_variables(&[("a", 1), ("b", 2)]);
    let reordered_plan = registry
        .compile_graph_v1(version_id, &reordered)
        .expect("map insertion order is not semantic");
    assert_eq!(reordered_plan.id(), plan.id());
}

#[test]
fn newly_compiled_plan_records_explicit_effect_protocol() {
    let mut plugin = ContractPlugin::new(ValidSchema::empty());
    let action = ContractAction::with_contract(
        "demo.echo",
        ValidSchema::empty(),
        ValidSchema::empty(),
        nebula_action::effect::ActionEffectContract::NoExternalEffects,
        None,
    );
    plugin.action = action;
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let frozen = registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x98; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let workflow = workflow_with_variables(&[]);
    let plan = frozen
        .compile_graph_v1(WorkflowVersionId::new(), &workflow)
        .unwrap();
    let record = serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    assert_eq!(record["compiler_version"], 4);
    assert_eq!(record["canonical_hash_version"], 2);
    assert_eq!(
        record["content"]["actions"][0]["effect_contract"],
        "NoExternalEffects"
    );
}

#[test]
fn scalar_contracts_roundtrip_under_the_new_epoch_without_named_parameters() {
    for schema in [
        nebula_schema::schema_of::<()>().unwrap(),
        nebula_schema::schema_of::<u64>().unwrap(),
    ] {
        let registry = frozen(schema.clone(), 0xa1);
        let plan = registry
            .compile_graph_v1(WorkflowVersionId::new(), &workflow_with_variables(&[]))
            .unwrap();
        let wire = serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
        assert_eq!(wire["compiler_version"], 4);
        assert_eq!(wire["canonical_hash_version"], 2);
        assert_eq!(
            wire["content"]["actions"][0]["input_schema"],
            serde_json::json!({
                "schema_wire_version": 2, "schema": schema,
            })
        );
        let loaded =
            ExecutablePlanRevision::try_from_recorded_v1(serde_json::from_value(wire).unwrap())
                .unwrap();
        assert_eq!(loaded.id(), plan.id());
        loaded.validate_against(&registry).unwrap();
        assert!(
            loaded.execution_graph().unwrap().nodes()[0]
                .parameters
                .is_empty()
        );

        let mut workflow = workflow_with_variables(&[]);
        workflow.nodes[0]
            .parameters
            .insert("value".into(), ParamValue::literal(serde_json::json!(42)));
        assert!(
            registry
                .compile_graph_v1(WorkflowVersionId::new(), &workflow)
                .is_err()
        );
    }
}

#[test]
fn null_and_empty_record_contracts_have_distinct_plan_identities_and_exact_admission() {
    let record_registry = frozen(ValidSchema::empty(), 0xa2);
    let null_registry = frozen(nebula_schema::schema_of::<()>().unwrap(), 0xa2);
    let workflow = workflow_with_variables(&[]);
    let version = WorkflowVersionId::new();
    let record = record_registry
        .compile_graph_v1(version, &workflow)
        .unwrap();
    let null = null_registry.compile_graph_v1(version, &workflow).unwrap();
    assert_eq!(record.plugin_set_id(), null.plugin_set_id());
    assert_ne!(record.id(), null.id());
    std::assert_matches!(
        record.validate_against(&null_registry),
        Err(PlanRegistryCompatibilityError::ContractMismatch { section: "actions" })
    );
    std::assert_matches!(
        null.validate_against(&record_registry),
        Err(PlanRegistryCompatibilityError::ContractMismatch { section: "actions" })
    );
}

#[test]
fn undeclared_effects_cannot_be_compiled_for_durable_execution() {
    let mut plugin = ContractPlugin::new(ValidSchema::empty());
    let action = ContractAction::with_contract(
        "demo.echo",
        ValidSchema::empty(),
        ValidSchema::empty(),
        nebula_action::effect::ActionEffectContract::Undeclared,
        None,
    );
    plugin.action = action;
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let registry = registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x99; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let error = registry
        .compile_graph_v1(WorkflowVersionId::new(), &workflow_with_variables(&[]))
        .expect_err("undeclared effects must fail before durable activation");
    assert!(!error.diagnostics().is_empty());
}

#[test]
fn compatibility_detects_flavor_and_unfingerprinted_contract_drift() {
    let workflow = workflow_with_variables(&[]);
    let version_id = WorkflowVersionId::from_bytes([0x44; 16]);
    let original = frozen(ValidSchema::empty(), 0x55);
    let plan = original
        .compile_graph_v1(version_id, &workflow)
        .expect("closed Graph-v1 fixture compiles");

    let another_flavor = frozen(ValidSchema::empty(), 0x56);
    assert!(matches!(
        plan.validate_against(&another_flavor),
        Err(PlanRegistryCompatibilityError::WorkerFlavorMismatch { .. })
    ));

    let changed_schema = Schema::builder()
        .add(Field::string(field_key!("message")))
        .build()
        .expect("fixture schema is valid");
    let same_ids_but_changed_contract = frozen(changed_schema, 0x55);
    assert_eq!(
        original.plugin_set().id(),
        same_ids_but_changed_contract.plugin_set().id()
    );
    assert_eq!(
        original.revision().id(),
        same_ids_but_changed_contract.revision().id()
    );
    assert!(matches!(
        plan.validate_against(&same_ids_but_changed_contract),
        Err(PlanRegistryCompatibilityError::ContractMismatch { section: "actions" })
    ));
}

#[test]
fn recorded_templates_use_template_grammar_and_preserve_parameter_bytes() {
    let registry = frozen(
        Schema::builder()
            .add(Field::string(field_key!("value")))
            .build()
            .unwrap(),
        0x87,
    );
    for source in ["plain text", r"\{{ incomplete", "{{ 7 }}", "'{{ 7 }}'"] {
        let authored = ParamValue::template(source);
        let plan = registry
            .compile_graph_v1(
                WorkflowVersionId::from_bytes([0x88; 16]),
                &workflow_with_parameter(Some(authored.clone())),
            )
            .unwrap();
        let bytes = serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
        let loaded =
            ExecutablePlanRevision::try_from_recorded_v1(serde_json::from_slice(&bytes).unwrap())
                .unwrap();
        assert_eq!(loaded.id(), plan.id());
        assert_eq!(
            serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&loaded)).unwrap(),
            bytes
        );
        let graph = loaded.execution_graph().unwrap();
        assert_eq!(graph.nodes()[0].parameters["value"], authored);
    }
}

#[test]
fn recorded_expression_auto_priority_does_not_make_malformed_templates_valid() {
    let registry = frozen(
        Schema::builder()
            .add(Field::string(field_key!("value")))
            .build()
            .unwrap(),
        0x89,
    );
    let source = "'{{ incomplete'";
    let plan = registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x90; 16]),
            &workflow_with_parameter(Some(ParamValue::expression(source))),
        )
        .unwrap();
    assert_eq!(
        plan.execution_graph().unwrap().nodes()[0].parameters["value"],
        ParamValue::expression(source)
    );
    let error = registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x91; 16]),
            &workflow_with_parameter(Some(ParamValue::template(source))),
        )
        .unwrap_err();
    assert!(
        error.diagnostics().iter().any(
            |diagnostic| diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
        )
    );
}

#[test]
fn compiler_validates_the_complete_parameter_set_and_redacts_secret_payloads() {
    let required_schema = Schema::builder()
        .add(Field::string(field_key!("value")).required())
        .build()
        .expect("fixture schema is valid");
    let registry = frozen(required_schema, 0x61);
    let error = registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x62; 16]),
            &workflow_with_parameter(None),
        )
        .expect_err("a missing required parameter cannot reach a recorded plan");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/echo/parameters"
    }));

    let literal_schema = Schema::builder()
        .add(Field::string(field_key!("value")).no_expression())
        .build()
        .expect("fixture schema is valid");
    let literal_registry = frozen(literal_schema, 0x63);
    literal_registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x64; 16]),
            &workflow_with_parameter(Some(ParamValue::literal(serde_json::json!(
                "{{ $workflow.input }}"
            )))),
        )
        .expect("a tagged literal remains a literal");
    let expression_error = literal_registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x65; 16]),
            &workflow_with_parameter(Some(ParamValue::expression("{{ $workflow.input }}"))),
        )
        .expect_err("an expression-forbidden field rejects an expression");
    assert_eq!(
        expression_error.diagnostics()[0].code(),
        "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
    );

    const SECRET_PAYLOAD: &str = "must-never-appear-in-a-diagnostic";
    let secret_schema = Schema::builder()
        .add(ObjectField::new(field_key!("value")).add(SecretField::new(field_key!("token"))))
        .build()
        .expect("fixture schema is valid");
    let secret_registry = frozen(secret_schema, 0x66);
    let secret_error = secret_registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x67; 16]),
            &workflow_with_parameter(Some(ParamValue::literal(
                serde_json::json!({"token": SECRET_PAYLOAD}),
            ))),
        )
        .expect_err("secret material cannot enter a Graph-v1 record");
    for diagnostic in secret_error.diagnostics() {
        assert!(!diagnostic.expected().contains(SECRET_PAYLOAD));
        assert!(!diagnostic.actual().contains(SECRET_PAYLOAD));
        assert!(!diagnostic.remediation().contains(SECRET_PAYLOAD));
    }
    assert!(!format!("{secret_error}").contains(SECRET_PAYLOAD));
    assert!(!format!("{secret_error:?}").contains(SECRET_PAYLOAD));
}

#[test]
fn execution_graph_preserves_recorded_runtime_configuration() {
    use nebula_workflow::{ErrorStrategy, RateLimit, RetryConfig};
    use std::time::Duration;

    let registry = frozen(
        Schema::builder()
            .add(Field::string(field_key!("value")))
            .build()
            .unwrap(),
        0x73,
    );
    let mut workflow = workflow_with_parameter(Some(ParamValue::literal(serde_json::json!(
        "projection-secret-canary"
    ))));
    workflow.variables.insert(
        "private".into(),
        serde_json::json!("projection-secret-canary"),
    );
    workflow.config.timeout = Some(Duration::new(19, 123));
    workflow.config.max_parallel_nodes = 7;
    workflow.config.checkpointing.enabled = false;
    workflow.config.checkpointing.interval = Some(Duration::new(3, 456));
    workflow.config.error_strategy = ErrorStrategy::ContinueOnError;
    let retry = RetryConfig {
        max_attempts: 6,
        initial_delay_ms: 17,
        max_delay_ms: 1234,
        backoff_multiplier: 1.375,
    };
    workflow.config.retry_policy = Some(retry.clone());
    workflow.nodes[0].retry_policy = Some(retry);
    workflow.nodes[0].timeout = Some(Duration::new(11, 789));
    workflow.nodes[0].rate_limit = Some(RateLimit {
        max_requests: 13,
        window_secs: 29,
    });
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x74; 16]), &workflow)
        .unwrap();
    let wire = serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    let loaded = ExecutablePlanRevision::try_from(
        serde_json::from_slice::<RecordedExecutablePlanRevisionV1>(&wire).unwrap(),
    )
    .unwrap();
    drop(registry);
    let graph = loaded.execution_graph().unwrap();
    assert_eq!(loaded.workflow_id(), workflow.id);
    assert_eq!(graph.plan_revision_id(), loaded.id());
    assert_eq!(
        graph.worker_flavor_revision_id(),
        loaded.worker_flavor_revision_id()
    );
    assert_eq!(graph.config(), &workflow.config);
    assert_eq!(graph.variables(), &workflow.variables);
    let projected = &graph.nodes()[0];
    let authored = &workflow.nodes[0];
    assert_eq!(projected.action_key.as_str(), "demo.echo");
    assert_eq!(
        projected.interface_version,
        Some(semver::Version::new(1, 0, 0))
    );
    assert_eq!(projected.parameters, authored.parameters);
    assert_eq!(projected.retry_policy, authored.retry_policy);
    assert_eq!(projected.timeout, authored.timeout);
    assert_eq!(projected.rate_limit, authored.rate_limit);
    assert_eq!(projected.enabled, authored.enabled);
    assert!(!format!("{graph:?}").contains("projection-secret-canary"));
}

#[test]
fn execution_graph_preserves_parameter_variants_and_canonical_reference_ports() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .unwrap();
    let action = ContractAction::with_contract(
        "demo.echo",
        schema.clone(),
        schema,
        nebula_action::effect::ActionEffectContract::NoExternalEffects,
        None,
    );
    let plugin = ContractPlugin {
        manifest: PluginManifest::builder("demo", "Demo").build().unwrap(),
        action,
    };
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let frozen = registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x75; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "echo")
        .unwrap()
        .with_parameter(
            "value",
            ParamValue::literal(serde_json::json!("secret-canary")),
        );
    let mut builder = WorkflowBuilder::new("Projected parameters").add_node(source);
    for (key, value) in [
        (
            node_key!("expression"),
            ParamValue::expression("variables.value"),
        ),
        (
            node_key!("template"),
            ParamValue::template("hello {{ variables.value }}"),
        ),
        (
            node_key!("reference"),
            ParamValue::reference(
                node_key!("source"),
                ValuePath::from_pointer("/value").unwrap(),
            ),
        ),
    ] {
        builder = builder
            .add_node(
                NodeDefinition::new(key.clone(), "Consumer", "demo", "echo")
                    .unwrap()
                    .with_parameter("value", value),
            )
            .connect(node_key!("source"), key);
    }
    let workflow = builder.build().unwrap();
    let plan = frozen
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x76; 16]), &workflow)
        .unwrap();
    let wire = serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    let loaded = ExecutablePlanRevision::try_from(
        serde_json::from_slice::<RecordedExecutablePlanRevisionV1>(&wire).unwrap(),
    )
    .unwrap();
    drop(frozen);
    let graph = loaded.execution_graph().unwrap();
    for authored in &workflow.nodes {
        let projected = graph
            .nodes()
            .iter()
            .find(|node| node.id == authored.id)
            .unwrap();
        if authored.id == node_key!("reference") {
            assert_eq!(
                projected.parameters["value"],
                ParamValue::reference(
                    node_key!("source"),
                    ValuePath::from_pointer("/value").unwrap(),
                )
            );
        } else {
            assert_eq!(projected.parameters, authored.parameters);
        }
    }
    assert_eq!(graph.connections().len(), 3);
    for connection in graph.connections() {
        assert_eq!(connection.from_node, node_key!("source"));
        assert_eq!(connection.from_port.as_ref().unwrap().as_str(), "out");
        assert!(connection.to_port.is_none());
        assert!(
            workflow
                .connections
                .iter()
                .any(|authored| authored.to_node == connection.to_node)
        );
    }
    assert_eq!(
        nebula_workflow::DependencyGraph::from_parts(graph.nodes(), graph.connections())
            .unwrap()
            .topological_sort()
            .unwrap()
            .first(),
        Some(&node_key!("source"))
    );
}
