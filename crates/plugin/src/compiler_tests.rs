use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nebula_action::{ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata};
use nebula_core::{
    ArtifactSetDigest, Dependencies, PluginKey, WorkflowId, WorkflowVersionId, node_key,
};
use nebula_metadata::PluginManifest;
use nebula_schema::{Field, ObjectField, Schema, SecretField, ValidSchema, field_key};
use nebula_workflow::{NodeDefinition, ParamValue, WorkflowBuilder};
use serde_json::{Value, json};

use super::*;

const SECRET_PAYLOAD: &str = "compiler-secret-that-must-not-leak";

/// Red→green proof that provider selection does not depend on hash order.
///
/// `PluginKey` permits `.`, so `acme` and `acme.storage` can both
/// namespace-own `acme.storage.bucket`. The winner's key is hashed into the
/// content-addressed revision id, so picking whichever provider the
/// registry's `HashMap` yielded first made two replicas compile the same
/// registry and workflow to different `ExecutablePlanRevisionId`s.
#[test]
fn provider_selection_is_lowest_key_regardless_of_iteration_order() {
    let forward =
        lowest_keyed_provider([("acme.storage", "namespaced"), ("acme", "root")].into_iter());
    let reverse =
        lowest_keyed_provider([("acme", "root"), ("acme.storage", "namespaced")].into_iter());

    assert_eq!(
        forward, reverse,
        "the same provider set must resolve identically whatever order it is walked in"
    );
    assert_eq!(
        forward,
        Some("root"),
        "the lowest plugin key is the deterministic winner"
    );
    assert_eq!(
        lowest_keyed_provider(std::iter::empty::<(&str, &str)>()),
        None,
        "no provider still means no provider"
    );
}

/// Red→green proof that a deny-all connection filter is never inverted.
///
/// `None` is "unfiltered" and a present list is "only these", so an
/// explicitly empty list means "accept nothing". Canonicalizing it to
/// `None` admitted every source node onto the port — the exact opposite of
/// what the plugin declared — and produced a record the plan validator
/// rejects as noncanonical anyway.
#[test]
fn empty_connection_filter_is_refused_not_collapsed_to_unfiltered() {
    let empty: &[String] = &[];
    assert!(
        matches!(
            canonical_optional_strings(Some(empty)),
            Err(ContractProjectionError::EmptyConnectionFilter)
        ),
        "an explicitly empty filter must be refused, never read as unfiltered"
    );

    assert!(
        matches!(canonical_optional_strings(None), Ok(None)),
        "an absent filter is genuinely unfiltered"
    );

    let projected =
        canonical_optional_strings(Some(&["b".to_owned(), "a".to_owned(), "b".to_owned()]))
            .expect("a non-empty filter must project")
            .expect("a present filter must stay present");
    assert_eq!(
        &*projected,
        ["a".to_owned(), "b".to_owned()],
        "a present filter is sorted and deduplicated, and stays present"
    );
}

struct TestActionFactory {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}

impl ActionFactory for TestActionFactory {
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
                "the pure compiler test factory is never instantiated",
            ))
        })
    }
}

struct TestPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn ActionFactory>>,
}

impl std::fmt::Debug for TestPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestPlugin")
            .field("key", self.manifest.key())
            .finish()
    }
}

impl crate::Plugin for TestPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        self.actions.clone()
    }
}

fn action_metadata(
    local_key: &str,
    kind: ActionKind,
    input_schema: ValidSchema,
    output_schema: ValidSchema,
) -> ActionMetadata {
    ActionMetadata::new(
        ActionKey::new(format!("demo.{local_key}")).expect("fixture action key is valid"),
        local_key,
        "compiler contract fixture",
    )
    .with_kind(kind)
    .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    .with_schema(input_schema)
    .with_output_schema(output_schema)
}

fn mode_equals(expected: &str) -> nebula_schema::Rule {
    nebula_schema::Rule::predicate(
        nebula_schema::Predicate::eq("mode", expected)
            .expect("the static fixture predicate path is valid"),
    )
}

fn conditional_input_schema() -> ValidSchema {
    let for_mode = mode_equals("for");
    Schema::builder()
        .add(
            Field::select(field_key!("mode"))
                .option("for", "For a duration")
                .option("until", "Until an instant")
                .required(),
        )
        .add(
            Field::integer(field_key!("amount"))
                .min_int(1)
                .required_when(for_mode.clone()),
        )
        .add(
            Field::select(field_key!("unit"))
                .option("milliseconds", "Milliseconds")
                .option("seconds", "Seconds")
                .required_when(for_mode),
        )
        .build()
        .expect("fixture input schema is valid")
}

fn reference_discriminator_error(
    input_schema: ValidSchema,
    nested_key: &str,
    nested_value: Value,
    identity_byte: u8,
) -> PlanCompilationError {
    let mode_output = Schema::builder()
        .add(Field::string(field_key!("mode")))
        .build()
        .expect("fixture output schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "source",
            ActionKind::Stateless,
            ValidSchema::empty(),
            mode_output,
        ),
        action_metadata(
            "conditional",
            ActionKind::Stateless,
            input_schema,
            ValidSchema::empty(),
        ),
    ]);
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
        .expect("fixture source node is valid");
    let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "conditional")
        .expect("fixture target node is valid")
        .with_parameter("mode", ParamValue::reference(node_key!("source"), "$.mode"))
        .with_parameter(nested_key, ParamValue::literal(nested_value));
    let workflow = WorkflowBuilder::new("Reference-backed nested condition")
        .id(WorkflowId::from_bytes([identity_byte; 16]))
        .add_node(source)
        .add_node(target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");

    compile_error(&registry, &workflow)
}

fn frozen(actions: Vec<ActionMetadata>) -> FrozenPluginRegistry {
    let plugin = TestPlugin {
        manifest: PluginManifest::builder("demo", "Demo")
            .build()
            .expect("fixture manifest is valid"),
        actions: actions
            .into_iter()
            .map(|metadata| {
                Arc::new(TestActionFactory {
                    metadata,
                    dependencies: Dependencies::new(),
                }) as Arc<dyn ActionFactory>
            })
            .collect(),
    };
    let resolved =
        Arc::new(ResolvedPlugin::from(plugin).expect("fixture plugin contracts resolve"));
    let mut registry = crate::PluginRegistry::new();
    registry
        .register(resolved)
        .expect("fixture plugin registers once");
    registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x61; 32]),
            "1.0.0"
                .parse()
                .expect("fixture runtime contract version is valid"),
        )
        .expect("fixture registry freezes")
}

fn one_node_workflow(node: NodeDefinition) -> WorkflowDefinition {
    WorkflowBuilder::new("Compiler contract")
        .id(WorkflowId::from_bytes([0x62; 16]))
        .add_node(node)
        .build()
        .expect("fixture workflow is structurally valid")
}

fn compile_error(
    registry: &FrozenPluginRegistry,
    workflow: &WorkflowDefinition,
) -> PlanCompilationError {
    registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x63; 16]), workflow)
        .expect_err("fixture must fail Graph-v1 compilation")
}

#[test]
fn reference_paths_normalize_only_ratified_aliases() {
    assert_eq!(normalize_reference_path("$").as_deref(), Some(""));
    assert_eq!(
        normalize_reference_path("$.payload.0").as_deref(),
        Some("payload.0")
    );
    assert_eq!(
        normalize_reference_path("payload.0").as_deref(),
        Some("payload.0")
    );
    assert!(normalize_reference_path("$payload").is_none());
    assert!(normalize_reference_path("payload..name").is_none());
    assert!(normalize_reference_path("payload.00").is_none());
}

#[test]
fn json_pointer_escapes_dynamic_identifier_segments() {
    assert_eq!(
        JsonPointer::root("nodes")
            .child("a/b~c")
            .child("parameters")
            .into_string(),
        "/nodes/a~1b~0c/parameters"
    );
}

#[test]
fn diagnostics_are_stably_sorted_and_payload_free() {
    let mut diagnostics = Diagnostics::default();
    let plugin = PluginKey::new("sample").unwrap();
    diagnostics.push(
        DiagnosticCode::MissingPlugin,
        JsonPointer::root("nodes").child("b"),
        DiagnosticValue::RegisteredPlugin,
        DiagnosticValue::Plugin(&plugin),
        Remediation::RegisterPlugin,
    );
    diagnostics.push(
        DiagnosticCode::MissingPlugin,
        JsonPointer::root("nodes").child("a"),
        DiagnosticValue::RegisteredPlugin,
        DiagnosticValue::Plugin(&plugin),
        Remediation::RegisterPlugin,
    );
    let error = diagnostics.into_error().unwrap();
    assert_eq!(error.diagnostics().len(), 2);
    assert_eq!(error.diagnostics()[0].path(), "/nodes/a");
    assert_eq!(
        error.diagnostics()[0].code(),
        "PLUGIN_PLAN_GRAPH_V1:MISSING_PLUGIN"
    );
    assert!(!format!("{error:?}").contains("credential-value"));
}

#[test]
fn tagged_literal_is_not_reclassified_but_expression_is_rejected() {
    let input_schema = Schema::builder()
        .add(Field::string(field_key!("value")).no_expression())
        .build()
        .expect("fixture schema is valid");
    let registry = frozen(vec![action_metadata(
        "literal",
        ActionKind::Stateless,
        input_schema,
        ValidSchema::empty(),
    )]);
    let literal = NodeDefinition::new(node_key!("run"), "Run", "demo", "literal")
        .expect("fixture node is valid")
        .with_parameter("value", ParamValue::literal(json!("{{ $workflow.input }}")));
    registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x64; 16]),
            &one_node_workflow(literal),
        )
        .expect("the explicitly tagged literal remains a literal");

    let expression = NodeDefinition::new(node_key!("run"), "Run", "demo", "literal")
        .expect("fixture node is valid")
        .with_parameter("value", ParamValue::expression("{{ $workflow.input }}"));
    let error = compile_error(&registry, &one_node_workflow(expression));
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/run/parameters/value"
    }));
}

#[test]
fn gradual_any_accepts_canonical_arbitrary_parameters() {
    let registry = frozen(vec![action_metadata(
        "dynamic",
        ActionKind::Stateless,
        ValidSchema::any(),
        ValidSchema::empty(),
    )]);
    let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "dynamic")
        .expect("fixture node is valid")
        .with_parameter("mode", ParamValue::literal(json!("for")))
        .with_parameter(
            "payload",
            ParamValue::literal(json!({"nested": [true, 7, null]})),
        );

    registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x68; 16]),
            &one_node_workflow(node),
        )
        .expect("a gradual Any input accepts a canonical parameter bag");
}

#[test]
fn gradual_any_accepts_a_whole_output_reference_under_an_arbitrary_key() {
    let output_schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .expect("fixture output schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "source",
            ActionKind::Stateless,
            ValidSchema::empty(),
            output_schema,
        ),
        action_metadata(
            "dynamic",
            ActionKind::Stateless,
            ValidSchema::any(),
            ValidSchema::empty(),
        ),
    ]);
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
        .expect("fixture source node is valid");
    let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "dynamic")
        .expect("fixture target node is valid")
        .with_parameter(
            "arbitrary_payload",
            ParamValue::reference(node_key!("source"), "$"),
        );
    let workflow = WorkflowBuilder::new("Gradual whole-output reference")
        .id(WorkflowId::from_bytes([0x69; 16]))
        .add_node(source)
        .add_node(target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");

    registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x6a; 16]), &workflow)
        .expect("a gradual Any input accepts a whole-output reference");
}

#[test]
fn gradual_any_accepts_an_existing_authored_path_under_an_arbitrary_key() {
    let output_schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .expect("fixture output schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "source",
            ActionKind::Stateless,
            ValidSchema::empty(),
            output_schema,
        ),
        action_metadata(
            "dynamic",
            ActionKind::Stateless,
            ValidSchema::any(),
            ValidSchema::empty(),
        ),
    ]);
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
        .expect("fixture source node is valid");
    let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "dynamic")
        .expect("fixture target node is valid")
        .with_parameter(
            "arbitrary_value",
            ParamValue::reference(node_key!("source"), "$.value"),
        );
    let workflow = WorkflowBuilder::new("Gradual authored-path reference")
        .id(WorkflowId::from_bytes([0x6b; 16]))
        .add_node(source)
        .add_node(target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");

    registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x6c; 16]), &workflow)
        .expect("a gradual Any input accepts an existing authored path");
}

#[test]
fn conditional_parameter_contract_is_checked_with_the_complete_parameter_bag() {
    let registry = frozen(vec![action_metadata(
        "conditional",
        ActionKind::Stateless,
        conditional_input_schema(),
        ValidSchema::empty(),
    )]);
    let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "conditional")
        .expect("fixture node is valid")
        .with_parameter("mode", ParamValue::literal(json!("for")))
        .with_parameter("amount", ParamValue::literal(json!(60_000)))
        .with_parameter("unit", ParamValue::literal(json!("milliseconds")));

    registry
        .compile_graph_v1(
            WorkflowVersionId::from_bytes([0x6d; 16]),
            &one_node_workflow(node),
        )
        .expect("conditional fields are validated with their discriminator present");
}

#[test]
fn conditional_parameter_contract_rejects_a_value_outside_the_field_rules() {
    let registry = frozen(vec![action_metadata(
        "conditional",
        ActionKind::Stateless,
        conditional_input_schema(),
        ValidSchema::empty(),
    )]);
    let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "conditional")
        .expect("fixture node is valid")
        .with_parameter("mode", ParamValue::literal(json!("for")))
        .with_parameter("amount", ParamValue::literal(json!(0)))
        .with_parameter("unit", ParamValue::literal(json!("milliseconds")));

    let error = compile_error(&registry, &one_node_workflow(node));
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/run/parameters"
    }));
}

#[test]
fn conditional_parameter_contract_rejects_a_reference_backed_discriminator() {
    let mode_output = Schema::builder()
        .add(Field::string(field_key!("mode")))
        .build()
        .expect("fixture output schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "source",
            ActionKind::Stateless,
            ValidSchema::empty(),
            mode_output,
        ),
        action_metadata(
            "conditional",
            ActionKind::Stateless,
            conditional_input_schema(),
            ValidSchema::empty(),
        ),
    ]);
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
        .expect("fixture source node is valid");
    let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "conditional")
        .expect("fixture target node is valid")
        .with_parameter("mode", ParamValue::reference(node_key!("source"), "$.mode"))
        .with_parameter("amount", ParamValue::literal(json!(60_000)))
        .with_parameter("unit", ParamValue::literal(json!("milliseconds")));
    let workflow = WorkflowBuilder::new("Reference-backed conditional discriminator")
        .id(WorkflowId::from_bytes([0x6e; 16]))
        .add_node(source)
        .add_node(target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");

    let error = compile_error(&registry, &workflow);
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/target/parameters"
    }));
}

#[test]
fn nested_object_condition_cannot_depend_on_a_reference_backed_discriminator() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("mode")).required())
        .add(
            Field::object(field_key!("config"))
                .add(Field::string(field_key!("token")).required_when(mode_equals("advanced"))),
        )
        .build()
        .expect("fixture nested-object schema is valid");

    let error = reference_discriminator_error(schema, "config", json!({}), 0x6f);
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/target/parameters"
    }));
}

#[test]
fn nested_list_condition_cannot_depend_on_a_reference_backed_discriminator() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("mode")).required())
        .add(
            Field::list(field_key!("items"))
                .item(Field::object(field_key!("item")).add(
                    Field::string(field_key!("token")).required_when(mode_equals("advanced")),
                )),
        )
        .build()
        .expect("fixture nested-list schema is valid");

    let error = reference_discriminator_error(schema, "items", json!([{}]), 0x70);
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
            && diagnostic.path() == "/nodes/target/parameters"
    }));
}

#[test]
fn nested_secret_parameter_is_rejected_without_payload_disclosure() {
    let input_schema = Schema::builder()
        .add(ObjectField::new(field_key!("auth")).add(SecretField::new(field_key!("token"))))
        .build()
        .expect("fixture schema is valid");
    let registry = frozen(vec![action_metadata(
        "secret",
        ActionKind::Stateless,
        input_schema,
        ValidSchema::empty(),
    )]);
    let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "secret")
        .expect("fixture node is valid")
        .with_parameter(
            "auth",
            ParamValue::literal(json!({"token": SECRET_PAYLOAD})),
        );
    let error = compile_error(&registry, &one_node_workflow(node));
    let diagnostic = error
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
                && diagnostic.path() == "/nodes/run/parameters/auth"
        })
        .expect("secret parameter has an exact safe diagnostic");
    for field in [
        diagnostic.code(),
        diagnostic.path(),
        diagnostic.expected(),
        diagnostic.actual(),
        diagnostic.remediation(),
    ] {
        assert!(!field.contains(SECRET_PAYLOAD));
    }
    assert!(!error.to_string().contains(SECRET_PAYLOAD));
    assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
}

#[test]
fn trigger_secret_configuration_is_rejected_at_exact_path() {
    let trigger_schema = Schema::builder()
        .add(SecretField::new(field_key!("token")))
        .build()
        .expect("fixture schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "run",
            ActionKind::Stateless,
            ValidSchema::empty(),
            ValidSchema::empty(),
        ),
        action_metadata(
            "start",
            ActionKind::Trigger,
            trigger_schema,
            ValidSchema::empty(),
        ),
    ]);
    let node =
        NodeDefinition::new(node_key!("run"), "Run", "demo", "run").expect("fixture node is valid");
    let workflow = WorkflowBuilder::new("Compiler trigger contract")
        .id(WorkflowId::from_bytes([0x65; 16]))
        .add_node(node)
        .add_trigger(
            node_key!("hook"),
            PluginKey::new("demo").expect("fixture plugin key is valid"),
            ActionKey::new("start").expect("fixture action key is valid"),
            json!({"token": SECRET_PAYLOAD}),
        )
        .build()
        .expect("fixture workflow is structurally valid");
    let error = compile_error(&registry, &workflow);
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_TRIGGER_CONFIGURATION"
            && diagnostic.path() == "/trigger_bindings/hook/config"
    }));
    assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
}

#[test]
fn reference_alias_is_normalized_and_bad_contract_has_exact_path() {
    let value_schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .expect("fixture schema is valid");
    let registry = frozen(vec![
        action_metadata(
            "source",
            ActionKind::Stateless,
            ValidSchema::empty(),
            value_schema.clone(),
        ),
        action_metadata(
            "target",
            ActionKind::Stateless,
            value_schema,
            ValidSchema::empty(),
        ),
    ]);
    let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
        .expect("fixture source node is valid");
    let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "target")
        .expect("fixture target node is valid")
        .with_parameter(
            "value",
            ParamValue::reference(node_key!("source"), "$.value"),
        );
    let workflow = WorkflowBuilder::new("Compiler reference contract")
        .id(WorkflowId::from_bytes([0x66; 16]))
        .add_node(source.clone())
        .add_node(target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x67; 16]), &workflow)
        .expect("the ratified reference alias compiles");
    let recorded = RecordedExecutablePlanRevisionV1::from(&plan);
    let target = recorded
        .content
        .nodes
        .iter()
        .find(|node| node.id == "target")
        .expect("target node is recorded");
    assert!(matches!(
        &target.parameters[0].value,
        RecordedParameterValueV1::Reference { output_path, .. } if output_path == "value"
    ));

    let bad_target = NodeDefinition::new(node_key!("target"), "Target", "demo", "target")
        .expect("fixture target node is valid")
        .with_parameter(
            "value",
            ParamValue::reference(node_key!("source"), "$.missing"),
        );
    let bad_workflow = WorkflowBuilder::new("Compiler bad reference contract")
        .id(WorkflowId::from_bytes([0x68; 16]))
        .add_node(source)
        .add_node(bad_target)
        .connect(node_key!("source"), node_key!("target"))
        .build()
        .expect("fixture workflow is structurally valid");
    let error = compile_error(&registry, &bad_workflow);
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_REFERENCE_CONTRACT"
            && diagnostic.path() == "/nodes/target/parameters/value"
    }));
}
