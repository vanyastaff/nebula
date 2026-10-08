//! Workflow graph editing: requested edits, reversible changes, undo and redo, and replay after the
//! server revision changed. Independent of presentation; every mutation goes through `Draft::apply`.

use nebula_api_contract::v1::workflow::{
    CreateWorkflowRequest, UpdateWorkflowDocumentRequest, UpdateWorkflowRequest,
    WorkflowDocumentResponse,
};
use serde_json::{Value, json};

/// Plugin for nodes added from the action catalog. The catalog does not report plugins yet.
const CATALOG_PLUGIN_KEY: &str = "core";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum EditError {
    #[error("This server does not expose an editable workflow document.")]
    UnsupportedDocument,
    #[error("The selected node or literal parameter no longer exists.")]
    MissingParameter,
    #[error("Enter a valid JSON value for the parameter.")]
    InvalidValue,
    #[error("The selected node no longer exists.")]
    NodeNotFound,
    #[error("A node with this id already exists.")]
    NodeExists,
    #[error("Node names cannot be empty.")]
    EmptyName,
    #[error("These nodes are already connected.")]
    ConnectionExists,
    #[error("These nodes are not connected.")]
    ConnectionNotFound,
}

/// A graph edit as the user requested it. Applying it records what undo needs.
#[derive(Clone)]
pub(crate) enum Edit {
    SetParameter {
        node: String,
        parameter: String,
        value: Value,
    },
    RenameNode {
        node: String,
        name: String,
    },
    InsertNode {
        node: Value,
    },
    RemoveNode {
        node: String,
    },
    Connect {
        from: String,
        to: String,
    },
    Disconnect {
        from: String,
        to: String,
    },
}

/// An applied edit with the state needed to reverse and redo it. `index` positions keep undo exact.
#[derive(Clone)]
enum Change {
    Parameter {
        node: String,
        parameter: String,
        before: Value,
        after: Value,
    },
    Name {
        node: String,
        before: String,
        after: String,
    },
    NodeInserted {
        node: Value,
        index: usize,
    },
    NodeRemoved {
        node: Value,
        index: usize,
        /// Connections that touched the node, with their positions before removal.
        connections: Vec<(usize, Value)>,
    },
    ConnectionAdded {
        connection: Value,
        index: usize,
    },
    ConnectionRemoved {
        connection: Value,
        index: usize,
    },
}

impl Change {
    /// The request that reproduces this change on any revision, used when replaying after a conflict.
    fn intent(&self) -> Edit {
        match self {
            Self::Parameter {
                node,
                parameter,
                after,
                ..
            } => Edit::SetParameter {
                node: node.clone(),
                parameter: parameter.clone(),
                value: after.clone(),
            },
            Self::Name { node, after, .. } => Edit::RenameNode {
                node: node.clone(),
                name: after.clone(),
            },
            Self::NodeInserted { node, .. } => Edit::InsertNode { node: node.clone() },
            Self::NodeRemoved { node, .. } => Edit::RemoveNode {
                node: node["id"].as_str().unwrap_or_default().to_owned(),
            },
            Self::ConnectionAdded { connection, .. } => {
                let (from, to) = endpoints(connection);
                Edit::Connect { from, to }
            },
            Self::ConnectionRemoved { connection, .. } => {
                let (from, to) = endpoints(connection);
                Edit::Disconnect { from, to }
            },
        }
    }
}

pub(crate) struct Draft {
    pub(crate) base: WorkflowDocumentResponse,
    pub(crate) definition: Value,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// A read never replaces an unsaved draft. Conflict recovery is explicit.
    pub(crate) remote: Option<WorkflowDocumentResponse>,
    pub(crate) uncertain_save: bool,
    pub(crate) save_conflict: bool,
    /// Set before dispatch; retained through disconnect until a receipt is known.
    pub(crate) start_key: Option<String>,
    pub(crate) execution_id: Option<String>,
}

/// Whether the server stores the parameters we sent. Activation canonicalizes node fields with
/// defaults, such as `retry_policy`, so whole-node equality would reject a correct write.
pub(crate) fn parameters_match(local: &Value, stored: &Value) -> bool {
    let (Some(local), Some(stored)) = (local.as_array(), stored.as_array()) else {
        return false;
    };
    local.len() == stored.len()
        && local.iter().all(|node| {
            stored
                .iter()
                .find(|candidate| candidate["id"] == node["id"])
                .is_some_and(|candidate| candidate["parameters"] == node["parameters"])
        })
}

/// A new workflow starts with an empty graph. The server assigns identity, version and timestamps.
pub(crate) fn new_workflow_request(name: &str) -> CreateWorkflowRequest {
    CreateWorkflowRequest {
        name: name.trim().into(),
        description: None,
        definition: json!({"nodes": [], "connections": []}),
    }
}

/// Applies the requested edit to the in-memory document without touching the server.
fn capture(definition: &Value, edit: Edit) -> Result<Change, EditError> {
    Ok(match edit {
        Edit::SetParameter {
            node,
            parameter,
            value,
        } => {
            let before = literal(definition, &node, &parameter)?.clone();
            Change::Parameter {
                node,
                parameter,
                before,
                after: value,
            }
        },
        Edit::RenameNode { node, name } => {
            let name = name.trim().to_owned();
            if name.is_empty() {
                return Err(EditError::EmptyName);
            }
            let before = find_node(definition, &node)?["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Change::Name {
                node,
                before,
                after: name,
            }
        },
        Edit::InsertNode { node } => {
            let nodes = nodes_of(definition)?;
            if node_position(nodes, node_id(&node)?).is_some() {
                return Err(EditError::NodeExists);
            }
            Change::NodeInserted {
                index: nodes.len(),
                node,
            }
        },
        Edit::RemoveNode { node: id } => {
            let nodes = nodes_of(definition)?;
            let index = node_position(nodes, &id).ok_or(EditError::NodeNotFound)?;
            let node = nodes.get(index).cloned().ok_or(EditError::NodeNotFound)?;
            let connections = connections_of(definition)
                .iter()
                .enumerate()
                .filter(|(_, connection)| touches(connection, &id))
                .map(|(position, connection)| (position, connection.clone()))
                .collect();
            Change::NodeRemoved {
                node,
                index,
                connections,
            }
        },
        Edit::Connect { from, to } => {
            require_node(definition, &from)?;
            require_node(definition, &to)?;
            if connection_between(definition, &from, &to).is_some() {
                return Err(EditError::ConnectionExists);
            }
            Change::ConnectionAdded {
                connection: json!({"from_node": from, "to_node": to}),
                index: connections_of(definition).len(),
            }
        },
        Edit::Disconnect { from, to } => {
            let index =
                connection_between(definition, &from, &to).ok_or(EditError::ConnectionNotFound)?;
            let connection = connections_of(definition)
                .get(index)
                .cloned()
                .ok_or(EditError::ConnectionNotFound)?;
            Change::ConnectionRemoved { connection, index }
        },
    })
}

/// Moves the document forward or backward through one change.
fn replay(definition: &mut Value, change: &Change, forward: bool) -> Result<(), EditError> {
    match change {
        Change::Parameter {
            node,
            parameter,
            before,
            after,
        } => {
            let value = if forward { after } else { before };
            *literal_mut(definition, node, parameter)? = value.clone();
        },
        Change::Name {
            node,
            before,
            after,
        } => {
            let name = if forward { after } else { before };
            set_field(find_node_mut(definition, node)?, "name", json!(name))?;
        },
        Change::NodeInserted { node, index } => {
            let nodes = nodes_mut(definition)?;
            if forward {
                nodes.insert((*index).min(nodes.len()), node.clone());
            } else {
                let position =
                    node_position(nodes, node_id(node)?).ok_or(EditError::NodeNotFound)?;
                nodes.remove(position);
            }
        },
        Change::NodeRemoved {
            node,
            index,
            connections,
        } => {
            if forward {
                let id = node_id(node)?;
                let nodes = nodes_mut(definition)?;
                let position = node_position(nodes, id).ok_or(EditError::NodeNotFound)?;
                nodes.remove(position);
                connections_mut(definition)?.retain(|connection| !touches(connection, id));
            } else {
                let nodes = nodes_mut(definition)?;
                nodes.insert((*index).min(nodes.len()), node.clone());
                let stored = connections_mut(definition)?;
                for (position, connection) in connections {
                    stored.insert((*position).min(stored.len()), connection.clone());
                }
            }
        },
        Change::ConnectionAdded { connection, index } => {
            let stored = connections_mut(definition)?;
            if forward {
                stored.insert((*index).min(stored.len()), connection.clone());
            } else {
                let position =
                    position_of(stored, connection).ok_or(EditError::ConnectionNotFound)?;
                stored.remove(position);
            }
        },
        Change::ConnectionRemoved { connection, index } => {
            let stored = connections_mut(definition)?;
            if forward {
                let position =
                    position_of(stored, connection).ok_or(EditError::ConnectionNotFound)?;
                stored.remove(position);
            } else {
                stored.insert((*index).min(stored.len()), connection.clone());
            }
        },
    }
    Ok(())
}

fn nodes_of(definition: &Value) -> Result<&[Value], EditError> {
    definition["nodes"]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(EditError::UnsupportedDocument)
}

fn nodes_mut(definition: &mut Value) -> Result<&mut Vec<Value>, EditError> {
    definition
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or(EditError::UnsupportedDocument)
}

/// Connections may be absent in older documents; reading treats that as empty.
fn connections_of(definition: &Value) -> &[Value] {
    match definition["connections"].as_array() {
        Some(connections) => connections,
        None => &[],
    }
}

fn connections_mut(definition: &mut Value) -> Result<&mut Vec<Value>, EditError> {
    definition
        .as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?
        .entry("connections")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or(EditError::UnsupportedDocument)
}

fn with_connections(definition: &Value) -> Value {
    let mut normalized = definition.clone();
    if let Some(object) = normalized.as_object_mut() {
        object.entry("connections").or_insert_with(|| json!([]));
    }
    normalized
}

fn node_id(node: &Value) -> Result<&str, EditError> {
    node["id"].as_str().ok_or(EditError::UnsupportedDocument)
}

fn node_position(nodes: &[Value], id: &str) -> Option<usize> {
    nodes
        .iter()
        .position(|node| node["id"].as_str() == Some(id))
}

fn find_node<'a>(definition: &'a Value, id: &str) -> Result<&'a Value, EditError> {
    let nodes = nodes_of(definition)?;
    nodes
        .get(node_position(nodes, id).ok_or(EditError::NodeNotFound)?)
        .ok_or(EditError::NodeNotFound)
}

fn find_node_mut<'a>(definition: &'a mut Value, id: &str) -> Result<&'a mut Value, EditError> {
    let nodes = nodes_mut(definition)?;
    let position = node_position(nodes, id).ok_or(EditError::NodeNotFound)?;
    nodes.get_mut(position).ok_or(EditError::NodeNotFound)
}

fn require_node(definition: &Value, id: &str) -> Result<(), EditError> {
    find_node(definition, id).map(|_| ())
}

fn touches(connection: &Value, id: &str) -> bool {
    connection["from_node"].as_str() == Some(id) || connection["to_node"].as_str() == Some(id)
}

fn connection_between(definition: &Value, from: &str, to: &str) -> Option<usize> {
    connections_of(definition).iter().position(|connection| {
        connection["from_node"].as_str() == Some(from) && connection["to_node"].as_str() == Some(to)
    })
}

fn position_of(connections: &[Value], connection: &Value) -> Option<usize> {
    connections.iter().position(|stored| stored == connection)
}

fn endpoints(connection: &Value) -> (String, String) {
    (
        connection["from_node"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        connection["to_node"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    )
}

fn set_field(node: &mut Value, key: &str, value: Value) -> Result<(), EditError> {
    node.as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?
        .insert(key.into(), value);
    Ok(())
}

/// The literal value of a parameter. Expressions and templates are not editable yet.
fn literal<'a>(definition: &'a Value, node: &str, parameter: &str) -> Result<&'a Value, EditError> {
    let value = find_node(definition, node).map_err(|_| EditError::MissingParameter)?["parameters"]
        .get(parameter)
        .ok_or(EditError::MissingParameter)?;
    if value["type"].as_str() != Some("literal") {
        return Err(EditError::MissingParameter);
    }
    value.get("value").ok_or(EditError::MissingParameter)
}

fn literal_mut<'a>(
    definition: &'a mut Value,
    node: &str,
    parameter: &str,
) -> Result<&'a mut Value, EditError> {
    let value = find_node_mut(definition, node)
        .map_err(|_| EditError::MissingParameter)?
        .get_mut("parameters")
        .and_then(|parameters| parameters.get_mut(parameter))
        .ok_or(EditError::MissingParameter)?;
    if value["type"].as_str() != Some("literal") {
        return Err(EditError::MissingParameter);
    }
    value.get_mut("value").ok_or(EditError::MissingParameter)
}

/// A new node id derived from the action key, unique within the graph: `http_request`, `http_request_2`.
fn next_node_id(definition: &Value, action_key: &str) -> String {
    let stem: String = action_key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let taken: Vec<&str> = nodes_of(definition)
        .unwrap_or_default()
        .iter()
        .filter_map(|node| node["id"].as_str())
        .collect();
    let mut suffix = 1;
    loop {
        let candidate = if suffix == 1 {
            stem.clone()
        } else {
            format!("{stem}_{suffix}")
        };
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
        suffix += 1;
    }
}

impl Draft {
    pub(crate) fn new(base: WorkflowDocumentResponse) -> Result<Self, EditError> {
        if base.revision == 0
            || !base.definition["nodes"].is_array()
            || base.definition["id"].as_str() != Some(&base.workflow.id)
        {
            return Err(EditError::UnsupportedDocument);
        }
        Ok(Self {
            definition: base.definition.clone(),
            base,
            undo: Vec::new(),
            redo: Vec::new(),
            remote: None,
            uncertain_save: false,
            save_conflict: false,
            start_key: None,
            execution_id: None,
        })
    }

    /// Compares the graph with the base. An absent `connections` key equals an empty list, because
    /// undoing the first connection leaves an empty list behind.
    pub(crate) fn dirty(&self) -> bool {
        with_connections(&self.definition) != with_connections(&self.base.definition)
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn connections(&self) -> &[Value] {
        connections_of(&self.definition)
    }

    /// Records one graph edit. A rejected edit leaves the document and history untouched.
    #[tracing::instrument(name = "client.document.apply", skip_all)]
    pub(crate) fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        let change = capture(&self.definition, edit)?;
        if let Change::Parameter { before, after, .. } = &change
            && before == after
        {
            return Ok(());
        }
        replay(&mut self.definition, &change, true)?;
        self.undo.push(change);
        self.redo.clear();
        Ok(())
    }

    pub(crate) fn edit(
        &mut self,
        node: &str,
        parameter: &str,
        text: &str,
    ) -> Result<(), EditError> {
        let value: Value = serde_json::from_str(text).map_err(|_| EditError::InvalidValue)?;
        self.apply(Edit::SetParameter {
            node: node.into(),
            parameter: parameter.into(),
            value,
        })
    }

    /// Adds a node with no parameters. Required inputs are validated by the server at publication.
    pub(crate) fn add_node(
        &mut self,
        action_key: &str,
        action_name: &str,
    ) -> Result<String, EditError> {
        let id = next_node_id(&self.definition, action_key);
        self.apply(Edit::InsertNode {
            node: json!({
                "id": id,
                "name": action_name,
                "plugin_key": CATALOG_PLUGIN_KEY,
                "action_key": action_key,
                "parameters": {},
            }),
        })?;
        Ok(id)
    }

    pub(crate) fn rename_node(&mut self, node: &str, name: &str) -> Result<(), EditError> {
        self.apply(Edit::RenameNode {
            node: node.into(),
            name: name.into(),
        })
    }

    /// Removes the node and every connection that touches it.
    pub(crate) fn remove_node(&mut self, node: &str) -> Result<(), EditError> {
        self.apply(Edit::RemoveNode { node: node.into() })
    }

    pub(crate) fn connect(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        self.apply(Edit::Connect {
            from: from.into(),
            to: to.into(),
        })
    }

    pub(crate) fn disconnect(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        self.apply(Edit::Disconnect {
            from: from.into(),
            to: to.into(),
        })
    }

    pub(crate) fn undo(&mut self) -> Result<(), EditError> {
        let Some(change) = self.undo.last().cloned() else {
            return Ok(());
        };
        replay(&mut self.definition, &change, false)?;
        self.undo.pop();
        self.redo.push(change);
        Ok(())
    }

    pub(crate) fn redo(&mut self) -> Result<(), EditError> {
        let Some(change) = self.redo.last().cloned() else {
            return Ok(());
        };
        replay(&mut self.definition, &change, true)?;
        self.redo.pop();
        self.undo.push(change);
        Ok(())
    }

    pub(crate) fn save_request(&self) -> UpdateWorkflowDocumentRequest {
        // Nodes and connections are the editable graph. Every other server field is preserved verbatim.
        UpdateWorkflowDocumentRequest {
            expected_revision: Some(self.base.revision),
            update: UpdateWorkflowRequest {
                name: None,
                description: None,
                definition: Some(json!({
                    "nodes": self.definition["nodes"],
                    "connections": connections_of(&self.definition),
                })),
            },
        }
    }

    pub(crate) fn saved(&mut self, document: WorkflowDocumentResponse) {
        self.definition = document.definition.clone();
        self.base = document;
        self.undo.clear();
        self.redo.clear();
        self.remote = None;
        self.uncertain_save = false;
        self.save_conflict = false;
    }

    /// Replays the recorded edits onto a freshly read snapshot. Any edit that no longer applies
    /// (a removed node, an existing connection) fails and leaves this draft untouched.
    pub(crate) fn reapply(&mut self) -> Result<(), EditError> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let mut next = Self::new(remote.clone())?;
        for change in &self.undo {
            next.apply(change.intent())?;
        }
        next.start_key.clone_from(&self.start_key);
        next.execution_id.clone_from(&self.execution_id);
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn snapshot(revision: u64, value: i64) -> WorkflowDocumentResponse {
        serde_json::from_value(json!({"id":"wf_test","name":"Echo","created_at":0,"updated_at":0,"revision":revision,"definition":{"id":"wf_test","nodes":[{"id":"echo","parameters":{"message":{"type":"literal","value":value}}}],"unknown_future_field":true}})).unwrap()
    }

    fn two_node_draft() -> Draft {
        let mut draft = Draft::new(snapshot(1, 7)).unwrap();
        draft.add_node("http.request", "HTTP request").unwrap();
        draft.connect("echo", "http_request").unwrap();
        draft
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
            draft.add_node("http.request", "HTTP").unwrap(),
            "http_request"
        );
        assert_eq!(
            draft.add_node("http.request", "HTTP").unwrap(),
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
    fn parameters_match_accepts_server_defaults_but_rejects_a_changed_parameter() {
        let local = json!([{"id": "t", "parameters": {"data": {"type": "literal", "value": {"value": 2}}}}]);
        let canonical = json!([{"id": "t", "enabled": true, "retry_policy": null, "parameters": {"data": {"type": "literal", "value": {"value": 2}}}}]);
        let changed = json!([{"id": "t", "parameters": {"data": {"type": "literal", "value": {"value": 3}}}}]);
        assert!(parameters_match(&local, &canonical));
        assert!(!parameters_match(&local, &changed));
        assert!(!parameters_match(&local, &json!([])));
    }

    #[test]
    fn blank_workflow_request_trims_the_name_and_sends_a_loadable_empty_graph() {
        let request = new_workflow_request("  Echo  ");
        assert_eq!(request.name, "Echo");
        assert_eq!(request.definition, json!({"nodes": [], "connections": []}));
    }
}
