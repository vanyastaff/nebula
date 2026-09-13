//! Real core factories must carry their input declarations into durable plans.

use std::sync::Arc;

use nebula_core::{ArtifactSetDigest, WorkflowVersionId, node_key};
use nebula_plugin::{
    ExecutablePlanRevision, PluginRegistry, RecordedExecutablePlanRevisionV1, ResolvedPlugin,
};
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{NodeDefinition, ParamValue, WorkflowBuilder};
use serde_json::{Value, json};

fn assert_recorded_input(action: &str, parameters: Value) {
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(
            ResolvedPlugin::from(CorePlugin::try_new().unwrap()).unwrap(),
        ))
        .unwrap();
    let registry = registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x47; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let mut node =
        NodeDefinition::new(node_key!("step"), "Core input contract", "core", action).unwrap();
    for (key, value) in parameters.as_object().unwrap() {
        node = node.with_parameter(key, ParamValue::literal(value.clone()));
    }
    let workflow = WorkflowBuilder::new("Core input contract")
        .add_node(node)
        .build()
        .unwrap();
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::new(), &workflow)
        .unwrap();
    let encoded = serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    let restored =
        ExecutablePlanRevision::try_from_recorded_v1(serde_json::from_slice(&encoded).unwrap())
            .unwrap();
    assert_eq!(restored.id(), plan.id());
    restored.validate_against(&registry).unwrap();
}

#[test]
fn set_fields_literal_data_round_trips_through_the_recorded_plan() {
    assert_recorded_input(
        "set_fields",
        json!({"data": null, "assignments": [
            {"name": "literal", "value": {"$expr": "{{ literal }}", "": [1, null]}}
        ]}),
    );
}

#[test]
fn array_empty_data_and_operations_round_trip_through_the_recorded_plan() {
    assert_recorded_input("array", json!({"data": [], "operations": []}));
}
