//! Graph primitives behind `Draft`: requested edits, the changes they record, and how a change moves
//! the definition forward or backward. Everything here is a pure function over the definition JSON.

use serde_json::{Value, json};

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
pub(super) enum Change {
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
    /// Whether a replay error only means the remote already holds this change's outcome. Creating
    /// an existing connection or removing an absent node satisfies the edit; anything else is a
    /// conflict the user has to review.
    pub(super) fn already_in_effect(&self, error: &EditError) -> bool {
        matches!(
            (self, error),
            (Self::ConnectionAdded { .. }, EditError::ConnectionExists)
                | (
                    Self::ConnectionRemoved { .. },
                    EditError::ConnectionNotFound
                )
                | (Self::NodeRemoved { .. }, EditError::NodeNotFound)
        )
    }

    /// The request that reproduces this change on any revision, used when replaying after a conflict.
    pub(super) fn intent(&self) -> Edit {
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

/// Applies the requested edit to the in-memory definition without touching the server.
pub(super) fn capture(definition: &Value, edit: Edit) -> Result<Change, EditError> {
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

/// Moves the definition forward or backward through one change.
pub(super) fn replay(
    definition: &mut Value,
    change: &Change,
    forward: bool,
) -> Result<(), EditError> {
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

/// Connections may be absent in older documents; reading treats that as empty.
pub(super) fn connections_of(definition: &Value) -> &[Value] {
    match definition["connections"].as_array() {
        Some(connections) => connections,
        None => &[],
    }
}

/// Compares graphs with an absent `connections` key equal to an empty list. Undoing the first
/// connection leaves an empty list behind, which is not a change from the base.
pub(super) fn with_connections(definition: &Value) -> Value {
    let mut normalized = definition.clone();
    if let Some(object) = normalized.as_object_mut() {
        object.entry("connections").or_insert_with(|| json!([]));
    }
    normalized
}

/// A node id derived from the action key, unique within the graph: `http_request`, `http_request_2`.
pub(super) fn next_node_id(definition: &Value, action_key: &str) -> String {
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

fn connections_mut(definition: &mut Value) -> Result<&mut Vec<Value>, EditError> {
    definition
        .as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?
        .entry("connections")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or(EditError::UnsupportedDocument)
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

/// The display name of a node, falling back to its id when the node or its name is missing.
pub(super) fn node_name(definition: &Value, id: &str) -> String {
    find_node(definition, id)
        .ok()
        .and_then(|node| node["name"].as_str())
        .unwrap_or(id)
        .to_owned()
}

/// Connections touching a node, as (from, to) pairs in graph order.
pub(super) fn links_of(definition: &Value, id: &str) -> Vec<(String, String)> {
    connections_of(definition)
        .iter()
        .filter(|connection| touches(connection, id))
        .map(endpoints)
        .collect()
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
