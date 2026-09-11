//! Actual typed builder and strict schema-validation rejections.
use nebula_core::{ActionKey, node_key};
use nebula_schema::{Field, Schema, ValidSchema, ValuePath, field_key};
use nebula_workflow::{
    NodeDefinition, NodeIoSchemas, NodeSchemaResolver, ParamValue, SchemaCheckMode, WorkflowBuilder,
};
use serde_json::json;

use super::{Boundary, ScenarioObservation, observe_rejection};

struct Resolver {
    source: NodeIoSchemas,
    target: NodeIoSchemas,
}
impl NodeSchemaResolver for Resolver {
    fn io_schemas(&self, action: &ActionKey, _: Option<&semver::Version>) -> Option<NodeIoSchemas> {
        Some(if action.as_str() == "source" {
            self.source.clone()
        } else {
            self.target.clone()
        })
    }
}

pub(super) fn observations() -> Vec<ScenarioObservation> {
    let mut observations = Vec::new();
    for (scenario, expected, input, rejection) in [
        (
            "invalid_action_key",
            "INVALID_ACTION_KEY",
            json!({"plugin":"core", "action":"Invalid Action"}),
            NodeDefinition::new(node_key!("step"), "Step", "core", "Invalid Action").unwrap_err(),
        ),
        (
            "invalid_plugin_key",
            "INVALID_PLUGIN_KEY",
            json!({"plugin":"Invalid Plugin", "action":"echo"}),
            NodeDefinition::new(node_key!("step"), "Step", "Invalid Plugin", "echo").unwrap_err(),
        ),
        (
            "invalid_owner",
            "INVALID_OWNER_ID",
            json!({"name":"Fixture", "owner":" "}),
            WorkflowBuilder::new("Fixture").owner(" ").err().unwrap(),
        ),
    ] {
        let observation = observe_rejection(
            &format!("workflow_validation.{scenario}"),
            &input,
            Boundary::WorkflowValidation,
            &rejection,
        );
        assert_eq!(observation.events[0].code, format!("WORKFLOW:{expected}"));
        observations.push(observation);
    }
    for (scenario, expected) in [
        ("port_incompatible", "PORT_SCHEMA_INCOMPATIBLE"),
        ("port_undecidable", "PORT_SCHEMA_UNDECIDABLE"),
        ("reference_path", "REFERENCE_PATH_UNRESOLVED"),
        ("reference_incompatible", "REFERENCE_TYPE_INCOMPATIBLE"),
        ("reference_undecidable", "REFERENCE_TYPE_UNDECIDABLE"),
    ] {
        let (output, input, reference) = match scenario {
            "port_incompatible" => (
                Schema::builder()
                    .add(Field::string(field_key!("value")))
                    .build()
                    .unwrap(),
                Schema::builder()
                    .add(Field::boolean(field_key!("value")).required())
                    .build()
                    .unwrap(),
                None,
            ),
            "port_undecidable" => (
                Schema::builder()
                    .add(Field::dynamic(field_key!("value")))
                    .build()
                    .unwrap(),
                Schema::builder()
                    .add(Field::string(field_key!("value")).required())
                    .build()
                    .unwrap(),
                None,
            ),
            "reference_path" => (
                Schema::builder()
                    .add(Field::list(field_key!("items")).item(Field::string(field_key!("item"))))
                    .build()
                    .unwrap(),
                ValidSchema::empty(),
                Some("/items/first"),
            ),
            "reference_incompatible" => (
                Schema::builder()
                    .add(Field::string(field_key!("value")))
                    .build()
                    .unwrap(),
                Schema::builder()
                    .add(Field::boolean(field_key!("input")).required())
                    .build()
                    .unwrap(),
                Some("/value"),
            ),
            "reference_undecidable" => (
                Schema::builder()
                    .add(Field::number(field_key!("value")))
                    .build()
                    .unwrap(),
                Schema::builder()
                    .add(Field::integer(field_key!("input")).required())
                    .build()
                    .unwrap(),
                Some("/value"),
            ),
            _ => unreachable!(),
        };
        let mut target =
            NodeDefinition::new(node_key!("target"), "Target", "core", "target").unwrap();
        if let Some(path) = reference {
            target = target.with_parameter(
                "input",
                ParamValue::reference(node_key!("source"), ValuePath::from_pointer(path).unwrap()),
            );
        }
        let workflow = WorkflowBuilder::new("Schema fixture")
            .add_node(NodeDefinition::new(node_key!("source"), "Source", "core", "source").unwrap())
            .add_node(target)
            .connect(node_key!("source"), node_key!("target"))
            .build()
            .unwrap();
        let actual_input = json!({"workflow":workflow, "source_output":output, "target_input":input, "mode":"strict"});
        let resolver = Resolver {
            source: NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: output.into(),
            },
            target: NodeIoSchemas {
                input: input.into(),
                output: ValidSchema::empty().into(),
            },
        };
        let errors = nebula_workflow::validate_workflow_with_resolver_mode(
            &workflow,
            &resolver,
            SchemaCheckMode::Strict,
        );
        let mut observation = ScenarioObservation {
            scenario: format!("workflow_validation.{scenario}"),
            input_sha256: String::new(),
            events: Vec::new(),
        };
        for error in &errors {
            let rejected = observe_rejection(
                &observation.scenario,
                &actual_input,
                Boundary::WorkflowValidation,
                error,
            );
            observation.input_sha256 = rejected.input_sha256;
            for mut event in rejected.events {
                event.sequence = observation.events.len();
                observation.events.push(event);
            }
        }
        assert!(
            observation
                .events
                .iter()
                .any(|event| event.code == format!("WORKFLOW:{expected}")),
            "{scenario}: {errors:?}"
        );
        observations.push(observation);
    }
    observations
}
