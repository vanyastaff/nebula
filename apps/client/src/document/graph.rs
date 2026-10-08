//! Graph primitives behind `Draft`: requested edits, the changes they record, and how a change moves
//! the definition forward or backward. Everything here is a pure function over the definition JSON.

use serde_json::{Value, json};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum EditError {
    #[error("This server does not expose an editable workflow document.")]
    UnsupportedDocument,
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

/// One connection of the graph. Links between the same nodes on different ports are distinct edges,
/// such as the main and the error route out of one node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) from: String,
    pub(crate) to: String,
    /// Source output port; absent means `out`, as the engine routes it.
    pub(crate) from_port: Option<String>,
    /// Target input port; absent means the node's default input.
    pub(crate) to_port: Option<String>,
}

impl Link {
    /// A link between the default ports, as the canvas draws it.
    pub(crate) fn between(from: &str, to: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port: None,
            to_port: None,
        }
    }

    fn of(connection: &Value) -> Self {
        let text = |key: &str| connection[key].as_str().map(str::to_owned);
        Self {
            from: text("from_node").unwrap_or_default(),
            to: text("to_node").unwrap_or_default(),
            from_port: text("from_port"),
            to_port: text("to_port"),
        }
    }

    /// The output port the engine routes this link from.
    pub(crate) fn source_port(&self) -> &str {
        self.from_port.as_deref().unwrap_or("out")
    }

    fn same_edge(&self, other: &Self) -> bool {
        self.from == other.from
            && self.to == other.to
            && self.source_port() == other.source_port()
            && self.to_port == other.to_port
    }

    fn to_value(&self) -> Value {
        let mut connection = json!({"from_node": self.from, "to_node": self.to});
        if let Some(object) = connection.as_object_mut() {
            if let Some(port) = &self.from_port {
                object.insert("from_port".into(), json!(port));
            }
            if let Some(port) = &self.to_port {
                object.insert("to_port".into(), json!(port));
            }
        }
        connection
    }
}

/// A graph edit as the user requested it. Applying it records what undo needs.
#[derive(Clone)]
pub(crate) enum Edit {
    /// Sets the whole parameter entry, a `{"type": ...}` value such as a literal or an expression, or
    /// removes it with `None` so the action's default applies.
    SetParameter {
        node: String,
        parameter: String,
        entry: Option<Value>,
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
        link: Link,
    },
    Disconnect {
        link: Link,
    },
    /// Routes an existing link from another output port of its source, such as `true` of an `if`;
    /// `None` is the default `out`.
    Reroute {
        link: Link,
        from_port: Option<String>,
    },
    /// Places a node on the editor canvas. Positions live in `ui_metadata`, not in the graph itself.
    MoveNode {
        node: String,
        x: f64,
        y: f64,
    },
}

/// An applied edit with the state needed to reverse and redo it. `index` positions keep undo exact.
#[derive(Clone)]
pub(super) enum Change {
    /// Whole parameter entries; `None` is an absent parameter.
    Parameter {
        node: String,
        parameter: String,
        before: Option<Value>,
        after: Option<Value>,
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
        /// The node's saved canvas position, removed with it so a later node with the same id does
        /// not inherit it.
        placed: Option<Value>,
    },
    ConnectionAdded {
        connection: Value,
        index: usize,
    },
    ConnectionRemoved {
        connection: Value,
        index: usize,
    },
    /// The same link from another source port; it keeps its place among the connections.
    ConnectionRerouted {
        before: Value,
        after: Value,
    },
    /// `before` is the position entry the node had, or `None` when it had none.
    Position {
        node: String,
        before: Option<Value>,
        after: Value,
    },
}

impl Change {
    /// Whether a replay error only means the remote `definition` already holds this change's
    /// outcome. Creating an existing connection, removing an absent node, or inserting a node the
    /// remote holds as the same action, name and parameters (a save whose answer was lost) satisfies
    /// the edit; anything else is a conflict the user has to review.
    pub(super) fn already_in_effect(&self, error: &EditError, definition: &Value) -> bool {
        match (self, error) {
            (Self::ConnectionAdded { .. }, EditError::ConnectionExists)
            | (Self::ConnectionRemoved { .. }, EditError::ConnectionNotFound)
            | (Self::NodeRemoved { .. }, EditError::NodeNotFound) => true,
            (Self::ConnectionRerouted { after, .. }, EditError::ConnectionNotFound) => {
                position_of(connections_of(definition), after).is_some()
            },
            (Self::NodeInserted { node, .. }, EditError::NodeExists) => {
                find_node(definition, node["id"].as_str().unwrap_or_default())
                    .is_ok_and(|stored| same_node(node, stored))
            },
            _ => false,
        }
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
                entry: after.clone(),
            },
            Self::Name { node, after, .. } => Edit::RenameNode {
                node: node.clone(),
                name: after.clone(),
            },
            Self::NodeInserted { node, .. } => Edit::InsertNode { node: node.clone() },
            Self::NodeRemoved { node, .. } => Edit::RemoveNode {
                node: node["id"].as_str().unwrap_or_default().to_owned(),
            },
            Self::ConnectionAdded { connection, .. } => Edit::Connect {
                link: Link::of(connection),
            },
            Self::ConnectionRemoved { connection, .. } => Edit::Disconnect {
                link: Link::of(connection),
            },
            Self::ConnectionRerouted { before, after } => Edit::Reroute {
                link: Link::of(before),
                from_port: Link::of(after).from_port,
            },
            Self::Position { node, after, .. } => Edit::MoveNode {
                node: node.clone(),
                x: after["x"].as_f64().unwrap_or_default(),
                y: after["y"].as_f64().unwrap_or_default(),
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

/// The stored node is the one the draft inserted: same action, name and parameters. Fields the
/// server fills in on its own, such as `enabled`, do not count.
fn same_node(intended: &Value, stored: &Value) -> bool {
    ["plugin_key", "action_key", "name", "parameters"]
        .iter()
        .all(|key| intended.get(key) == stored.get(key))
}

/// Applies the requested edit to the in-memory definition without touching the server.
pub(super) fn capture(definition: &Value, edit: Edit) -> Result<Change, EditError> {
    Ok(match edit {
        Edit::SetParameter {
            node,
            parameter,
            entry,
        } => {
            let before = find_node(definition, &node)?["parameters"]
                .get(&parameter)
                .cloned();
            Change::Parameter {
                node,
                parameter,
                before,
                after: entry,
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
                placed: saved_position(definition, &id),
                node,
                index,
                connections,
            }
        },
        Edit::Connect { link } => {
            require_node(definition, &link.from)?;
            require_node(definition, &link.to)?;
            if link_position(definition, &link).is_some() {
                return Err(EditError::ConnectionExists);
            }
            Change::ConnectionAdded {
                connection: link.to_value(),
                index: connections_of(definition).len(),
            }
        },
        Edit::Disconnect { link } => {
            let index = link_position(definition, &link).ok_or(EditError::ConnectionNotFound)?;
            let connection = connections_of(definition)
                .get(index)
                .cloned()
                .ok_or(EditError::ConnectionNotFound)?;
            Change::ConnectionRemoved { connection, index }
        },
        Edit::Reroute { link, from_port } => {
            let index = link_position(definition, &link).ok_or(EditError::ConnectionNotFound)?;
            let before = connections_of(definition)
                .get(index)
                .cloned()
                .ok_or(EditError::ConnectionNotFound)?;
            let rerouted = Link {
                from_port: from_port.filter(|port| port != "out"),
                ..link
            };
            if link_position(definition, &rerouted).is_some() {
                return Err(EditError::ConnectionExists);
            }
            // Fields the connection carries beyond its ends stay as they were.
            let mut after = before.clone();
            if let Some(object) = after.as_object_mut() {
                match &rerouted.from_port {
                    Some(port) => object.insert("from_port".into(), json!(port)),
                    None => object.remove("from_port"),
                };
            }
            Change::ConnectionRerouted { before, after }
        },
        Edit::MoveNode { node, x, y } => {
            require_node(definition, &node)?;
            Change::Position {
                before: saved_position(definition, &node),
                after: json!({"x": x, "y": y}),
                node,
            }
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
            let entry = if forward { after } else { before };
            set_parameter_entry(definition, node, parameter, entry.clone())?;
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
            placed,
        } => {
            let id = node_id(node)?;
            let nodes = nodes_mut(definition)?;
            if forward {
                let position = node_position(nodes, id).ok_or(EditError::NodeNotFound)?;
                nodes.remove(position);
                connections_mut(definition)?.retain(|connection| !touches(connection, id));
                set_saved_position(definition, id, None)?;
            } else {
                nodes.insert((*index).min(nodes.len()), node.clone());
                let stored = connections_mut(definition)?;
                for (position, connection) in connections {
                    stored.insert((*position).min(stored.len()), connection.clone());
                }
                if placed.is_some() {
                    set_saved_position(definition, id, placed.as_ref())?;
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
        Change::ConnectionRerouted { before, after } => {
            let (from, to) = if forward {
                (before, after)
            } else {
                (after, before)
            };
            let stored = connections_mut(definition)?;
            let position = position_of(stored, from).ok_or(EditError::ConnectionNotFound)?;
            if let Some(slot) = stored.get_mut(position) {
                slot.clone_from(to);
            }
        },
        Change::Position {
            node,
            before,
            after,
        } => {
            let target = if forward {
                Some(after)
            } else {
                before.as_ref()
            };
            set_saved_position(definition, node, target)?;
        },
    }
    Ok(())
}

/// The saved canvas position entry for a node, if the node has been placed by hand.
pub(super) fn saved_position(definition: &Value, node: &str) -> Option<Value> {
    definition
        .get("ui_metadata")
        .and_then(|metadata| metadata.get("node_positions"))
        .and_then(|positions| positions.get(node))
        .cloned()
}

/// Writes or clears one node's saved position. Clearing removes empty containers again, so an undone
/// first placement leaves the definition exactly as it was.
fn set_saved_position(
    definition: &mut Value,
    node: &str,
    position: Option<&Value>,
) -> Result<(), EditError> {
    let root = definition
        .as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?;
    match position {
        Some(position) => {
            let metadata = root
                .entry("ui_metadata")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or(EditError::UnsupportedDocument)?;
            metadata
                .entry("node_positions")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or(EditError::UnsupportedDocument)?
                .insert(node.into(), position.clone());
        },
        None => {
            if let Some(metadata) = root.get_mut("ui_metadata").and_then(Value::as_object_mut) {
                if let Some(positions) = metadata
                    .get_mut("node_positions")
                    .and_then(Value::as_object_mut)
                {
                    positions.remove(node);
                    if positions.is_empty() {
                        metadata.remove("node_positions");
                    }
                }
                if metadata.is_empty() {
                    root.remove("ui_metadata");
                }
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

/// Connections touching a node, in graph order.
pub(super) fn links_of(definition: &Value, id: &str) -> Vec<Link> {
    connections_of(definition)
        .iter()
        .filter(|connection| touches(connection, id))
        .map(Link::of)
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

fn link_position(definition: &Value, link: &Link) -> Option<usize> {
    connections_of(definition)
        .iter()
        .position(|connection| Link::of(connection).same_edge(link))
}

fn position_of(connections: &[Value], connection: &Value) -> Option<usize> {
    connections.iter().position(|stored| stored == connection)
}

fn set_field(node: &mut Value, key: &str, value: Value) -> Result<(), EditError> {
    node.as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?
        .insert(key.into(), value);
    Ok(())
}

/// Writes or removes one parameter entry. A node without a `parameters` object gets one.
fn set_parameter_entry(
    definition: &mut Value,
    node: &str,
    parameter: &str,
    entry: Option<Value>,
) -> Result<(), EditError> {
    let node = find_node_mut(definition, node)?
        .as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?;
    let parameters = node
        .entry("parameters")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(EditError::UnsupportedDocument)?;
    match entry {
        Some(entry) => {
            parameters.insert(parameter.to_owned(), entry);
        },
        None => {
            parameters.remove(parameter);
        },
    }
    Ok(())
}
