use super::*;

pub(crate) fn snapshot(revision: u64, value: i64) -> WorkflowDocumentResponse {
    serde_json::from_value(json!({"id":"wf_test","name":"Echo","created_at":0,"updated_at":0,"revision":revision,"definition":{"id":"wf_test","nodes":[{"id":"echo","parameters":{"message":{"type":"literal","value":value}}}],"unknown_future_field":true}})).unwrap()
}

fn two_node_draft() -> Draft {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.add_node("core.http_request", "HTTP request").unwrap();
    draft.connect("echo", "http_request").unwrap();
    draft
}

fn two_node_snapshot(revision: u64, connections: Value) -> WorkflowDocumentResponse {
    let mut document = snapshot(revision, 7);
    document.definition["nodes"] = json!([
        {"id": "echo", "name": "Echo", "action_key": "echo", "parameters": {}},
        {"id": "http_request", "name": "HTTP", "action_key": "http_request", "parameters": {}}
    ]);
    document.definition["connections"] = connections;
    document
}

#[test]
fn a_catalog_key_is_stored_as_plugin_and_action_and_read_back() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();

    let id = draft
        .add_node("core.json_transform", "JSON Transform")
        .unwrap();

    let node = draft.definition["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(node["plugin_key"], "core");
    assert_eq!(node["action_key"], "json_transform");
    assert_eq!(catalog_key(&node), "core.json_transform");
}

#[test]
fn an_action_key_already_qualified_by_its_plugin_is_kept() {
    let node = json!({"plugin_key": "core", "action_key": "core.delay"});
    assert_eq!(catalog_key(&node), "core.delay");
    // Only the plugin's own prefix counts as one.
    let other = json!({"plugin_key": "core", "action_key": "coreutils.run"});
    assert_eq!(catalog_key(&other), "core.coreutils.run");
}

#[test]
fn a_bare_action_key_belongs_to_the_default_plugin() {
    assert_eq!(
        split_catalog_key("json_transform"),
        ("core", "json_transform")
    );
    assert_eq!(split_catalog_key("slack.post"), ("slack", "post"));
    assert_eq!(split_catalog_key(".odd"), ("core", ".odd"));
}

#[test]
fn a_parameter_the_node_lacks_can_be_set_and_undone_to_absence() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.add_node("core.http_request", "HTTP request").unwrap();

    draft
        .set_literal("http_request", "url", json!("https://example.test"))
        .unwrap();
    let node = |draft: &Draft| draft.definition["nodes"][1].clone();
    assert_eq!(
        node(&draft)["parameters"]["url"],
        json!({"type": "literal", "value": "https://example.test"})
    );

    draft.undo().unwrap();
    assert!(node(&draft)["parameters"].get("url").is_none());
}

#[test]
fn a_parameter_switches_to_an_expression_and_back_and_clears() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft
        .set_entry("echo", "message", Some(expression("{{ $input.text }}")))
        .unwrap();
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"],
        json!({"type": "expression", "expr": "{{ $input.text }}"})
    );

    draft.set_entry("echo", "message", None).unwrap();
    assert!(
        draft.definition["nodes"][0]["parameters"]
            .get("message")
            .is_none()
    );

    draft.undo().unwrap();
    draft.undo().unwrap();
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"],
        json!({"type": "literal", "value": 7})
    );
    assert!(!draft.dirty());
}

#[test]
fn clearing_a_parameter_that_is_not_set_is_not_an_undo_step() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.set_entry("echo", "absent", None).unwrap();
    assert!(!draft.can_undo());
}

#[test]
fn keystrokes_of_one_typing_session_are_one_undo_step() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    for text in ["4", "42", "420"] {
        draft
            .type_parameter("echo", "message", Some(literal(json!(text))), 1)
            .unwrap();
    }
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"]["value"],
        "420"
    );
    // A new session in the same field, or another edit in between, starts a new step.
    draft
        .type_parameter("echo", "message", Some(literal(json!("4200"))), 2)
        .unwrap();

    draft.undo().unwrap();
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"]["value"],
        "420"
    );
    draft.undo().unwrap();
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"]["value"],
        7
    );
    assert!(!draft.can_undo());
}

#[test]
fn a_typing_session_that_ends_where_it_began_leaves_no_undo_step() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft
        .type_parameter("echo", "message", Some(literal(json!(78))), 1)
        .unwrap();
    draft
        .type_parameter("echo", "message", Some(literal(json!(7))), 1)
        .unwrap();
    assert!(!draft.can_undo());
    assert!(!draft.dirty());
}

#[test]
fn typed_edits_undo_redo_and_wire_patch_preserve_server_identity() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.edit("echo", "message", "8").unwrap();
    assert!(draft.dirty());
    draft.undo().unwrap();
    assert!(!draft.dirty());
    draft.redo().unwrap();
    let request = draft.save_request();
    assert_eq!(request.expected_revision, Some(1));
    assert_eq!(
        request.update.definition.unwrap()["nodes"][0]["parameters"]["message"]["value"],
        8
    );
    assert_eq!(draft.definition["unknown_future_field"], true);
    assert!(draft.edit("echo", "message", "invalid json").is_err());
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"]["value"],
        8
    );
}

#[test]
fn adding_and_connecting_nodes_undoes_and_redoes_the_exact_graph() {
    let mut draft = two_node_draft();
    assert_eq!(draft.connections().len(), 1);
    draft.undo().unwrap();
    assert!(draft.connections().is_empty());
    draft.undo().unwrap();
    assert_eq!(draft.definition["nodes"].as_array().unwrap().len(), 1);
    assert!(!draft.dirty());
    draft.redo().unwrap();
    draft.redo().unwrap();
    assert_eq!(draft.definition["nodes"][1]["id"], "http_request");
    assert_eq!(draft.connections()[0]["to_node"], "http_request");
}

#[test]
fn removing_a_node_drops_its_connections_and_undo_restores_both_in_place() {
    let mut draft = two_node_draft();
    let before = draft.definition.clone();
    draft.remove_node("http_request").unwrap();
    assert!(draft.connections().is_empty());
    draft.undo().unwrap();
    assert_eq!(draft.definition, before);
}

#[test]
fn node_ids_stay_unique_and_names_are_validated() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    assert_eq!(
        draft.add_node("core.http_request", "HTTP").unwrap(),
        "http_request"
    );
    assert_eq!(
        draft.add_node("core.http_request", "HTTP").unwrap(),
        "http_request_2"
    );
    assert_eq!(draft.rename_node("echo", "   "), Err(EditError::EmptyName));
    assert_eq!(
        draft.connect("echo", "missing"),
        Err(EditError::NodeNotFound)
    );
    draft.connect("echo", "http_request").unwrap();
    assert_eq!(
        draft.connect("echo", "http_request"),
        Err(EditError::ConnectionExists)
    );
}

#[test]
fn conflict_recovery_reapplies_commands_without_losing_other_remote_changes() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.edit("echo", "message", "9").unwrap();
    let mut remote = snapshot(2, 8);
    remote.definition["new_remote_field"] = json!(42);
    draft.remote = Some(remote);
    draft.reapply().unwrap();
    assert_eq!(draft.base.revision, 2);
    assert_eq!(draft.definition["new_remote_field"], 42);
    draft.undo().unwrap();
    assert_eq!(
        draft.definition["nodes"][0]["parameters"]["message"]["value"],
        8
    );
}

#[test]
fn graph_edits_replay_onto_a_newer_revision() {
    let mut draft = two_node_draft();
    draft.remote = Some(snapshot(2, 8));
    draft.reapply().unwrap();
    assert_eq!(draft.base.revision, 2);
    assert_eq!(draft.definition["nodes"][1]["id"], "http_request");
    assert_eq!(draft.connections().len(), 1);
}

#[test]
fn replay_fails_and_keeps_the_draft_when_the_remote_removed_a_connected_node() {
    let mut draft = two_node_draft();
    let mut remote = snapshot(2, 8);
    remote.definition["nodes"] = json!([]);
    draft.remote = Some(remote);
    assert_eq!(draft.reapply(), Err(EditError::NodeNotFound));
    assert!(draft.dirty());
    assert_eq!(draft.base.revision, 1);
}

#[test]
fn replay_counts_a_connection_the_remote_already_has_as_applied() {
    let mut draft = Draft::new(two_node_snapshot(1, json!([]))).unwrap();
    draft.connect("echo", "http_request").unwrap();
    draft.remote = Some(two_node_snapshot(
        2,
        json!([{"from_node": "echo", "to_node": "http_request", "from_port": "out"}]),
    ));
    draft.reapply().unwrap();
    assert_eq!(draft.base.revision, 2);
    assert_eq!(draft.connections().len(), 1);
    assert!(!draft.dirty());
}

#[test]
fn replay_counts_a_node_an_uncertain_save_already_stored_as_applied() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    let id = draft.add_node("core.http_request", "HTTP").unwrap();
    // The save reached the server, which filled in its own defaults; its answer was lost.
    let mut remote = snapshot(2, 7);
    let mut stored = draft.definition["nodes"][1].clone();
    stored["enabled"] = json!(true);
    remote.definition["nodes"]
        .as_array_mut()
        .unwrap()
        .push(stored);
    draft.remote = Some(remote);

    draft.reapply().unwrap();

    assert_eq!(draft.base.revision, 2);
    assert_eq!(draft.definition["nodes"][1]["id"], id.as_str());
    assert_eq!(draft.definition["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn replay_stops_at_another_node_with_the_inserted_id() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    let id = draft.add_node("core.http_request", "HTTP").unwrap();
    let mut remote = snapshot(2, 7);
    remote.definition["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": id, "plugin_key": "core", "action_key": "delay", "name": "Wait"}));
    draft.remote = Some(remote);

    assert_eq!(draft.reapply(), Err(EditError::NodeExists));
    assert_eq!(draft.base.revision, 1);
}

#[test]
fn replay_counts_a_removal_the_remote_already_made_as_applied() {
    let mut draft = Draft::new(two_node_snapshot(1, json!([]))).unwrap();
    draft.remove_node("http_request").unwrap();
    draft.remote = Some(snapshot(2, 8));
    draft.reapply().unwrap();
    assert_eq!(draft.definition["nodes"].as_array().unwrap().len(), 1);
    assert!(!draft.dirty());
}

#[test]
fn a_removed_node_takes_its_canvas_position_with_it() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    let id = draft.add_node("core.http_request", "HTTP").unwrap();
    draft.move_node(&id, 40.0, 50.0).unwrap();

    draft.remove_node(&id).unwrap();
    assert_eq!(draft.placed_position(&id), None);
    // A new node of the same action reuses the id, and the layout places it.
    let again = draft.add_node("core.http_request", "HTTP").unwrap();
    assert_eq!(again, id);
    assert_eq!(draft.placed_position(&again), None);

    draft.undo().unwrap();
    draft.undo().unwrap();
    assert_eq!(draft.placed_position(&id), Some((40.0, 50.0)));
}

#[test]
fn links_on_different_ports_are_separate_edges() {
    let mut draft = Draft::new(two_node_snapshot(
        1,
        json!([
            {"from_node": "echo", "to_node": "http_request"},
            {"from_node": "echo", "to_node": "http_request", "from_port": "error"}
        ]),
    ))
    .unwrap();
    let links = draft.links("echo");
    assert_eq!(links.len(), 2);
    assert_eq!(links[1].source_port(), "error");

    // The main route already exists, written without a port or with the default one.
    assert_eq!(
        draft.connect("echo", "http_request"),
        Err(EditError::ConnectionExists)
    );
    draft.disconnect(&links[1]).unwrap();
    assert_eq!(draft.links("echo"), vec![links[0].clone()]);

    draft.undo().unwrap();
    assert_eq!(draft.links("echo"), links);
    assert!(!draft.dirty());
}

#[test]
fn parameters_match_accepts_server_defaults_but_rejects_a_changed_parameter() {
    let local =
        json!([{"id": "t", "parameters": {"data": {"type": "literal", "value": {"value": 2}}}}]);
    let canonical = json!([{"id": "t", "enabled": true, "retry_policy": null, "parameters": {"data": {"type": "literal", "value": {"value": 2}}}}]);
    let changed =
        json!([{"id": "t", "parameters": {"data": {"type": "literal", "value": {"value": 3}}}}]);
    assert!(parameters_match(&local, &canonical));
    assert!(!parameters_match(&local, &changed));
    assert!(!parameters_match(&local, &json!([])));
}

#[test]
fn undoing_the_first_placement_leaves_the_definition_exactly_as_it_was() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.move_node("echo", 10.0, 20.0).unwrap();
    assert_eq!(draft.placed_position("echo"), Some((10.0, 20.0)));
    assert!(draft.dirty());
    draft.undo().unwrap();
    assert_eq!(draft.placed_position("echo"), None);
    assert!(draft.definition.get("ui_metadata").is_none());
    assert!(!draft.dirty());
}

#[test]
fn placements_are_sent_with_the_save_patch() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.move_node("echo", 10.0, 20.0).unwrap();
    let request = draft.save_request();
    let patch = request.update.definition.unwrap();
    assert_eq!(patch["ui_metadata"]["node_positions"]["echo"]["x"], 10.0);
    assert_eq!(patch["ui_metadata"]["node_positions"]["echo"]["y"], 20.0);
}

#[test]
fn removing_the_last_saved_placement_sends_empty_metadata() {
    let mut placed = snapshot(1, 7);
    placed.definition["ui_metadata"] = json!({"node_positions": {"echo": {"x": 10.0, "y": 20.0}}});
    let mut draft = Draft::new(placed).unwrap();
    draft.remove_node("echo").unwrap();
    assert!(draft.definition.get("ui_metadata").is_none());

    let patch = draft.save_request().update.definition.unwrap();

    assert_eq!(patch["ui_metadata"], json!({}));
}

#[test]
fn a_draft_that_never_had_placements_sends_none() {
    let draft = Draft::new(snapshot(1, 7)).unwrap();
    let patch = draft.save_request().update.definition.unwrap();
    assert!(patch.get("ui_metadata").is_none());
}

#[test]
fn placing_a_node_where_it_already_is_is_not_an_undo_step() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.move_node("echo", 10.0, 20.0).unwrap();
    draft.move_node("echo", 10.0, 20.0).unwrap();
    draft.undo().unwrap();
    assert_eq!(draft.placed_position("echo"), None);
    assert!(!draft.can_undo());
}

#[test]
fn a_placement_survives_a_replay_onto_a_newer_revision() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.move_node("echo", 10.0, 20.0).unwrap();
    draft.remote = Some(snapshot(2, 8));
    draft.reapply().unwrap();
    assert_eq!(draft.placed_position("echo"), Some((10.0, 20.0)));
    assert_eq!(draft.base.revision, 2);
}

#[test]
fn blank_workflow_request_trims_the_name_and_sends_a_loadable_empty_graph() {
    let request = new_workflow_request("  Echo  ");
    assert_eq!(request.name, "Echo");
    assert_eq!(request.definition, json!({"nodes": [], "connections": []}));
}
