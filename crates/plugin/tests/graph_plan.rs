use std::{future::Future, pin::Pin, sync::Arc};

use nebula_action::{ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata};
use nebula_core::{
    ActionKey, ArtifactSetDigest, Dependencies, WorkflowId, WorkflowVersionId, node_key,
};
use nebula_metadata::PluginManifest;
use nebula_plugin::{
    ExecutablePlanRevision, PlanRegistryCompatibilityError, Plugin, PluginRegistry,
    RecordedExecutablePlanRevisionV1, ResolvedPlugin, RuntimeContractVersion,
};
use nebula_schema::{Field, ObjectField, Schema, SecretField, ValidSchema, field_key};
use nebula_workflow::{NodeDefinition, ParamValue, WorkflowBuilder};

struct ContractAction {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}

impl ContractAction {
    fn new(input_schema: ValidSchema) -> Self {
        Self {
            metadata: ActionMetadata::new(
                ActionKey::new("demo.echo").expect("fixture action key is valid"),
                "Echo",
                "Graph-v1 contract fixture",
            )
            .with_schema(input_schema),
            dependencies: Dependencies::new(),
        }
    }
}

impl ActionFactory for ContractAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _context: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "contract fixture is never instantiated by the pure compiler",
            ))
        })
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
            action: Arc::new(ContractAction::new(input_schema)),
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
