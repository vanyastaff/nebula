//! Workflow document editing: the draft, its undo history, and what a server revision means for it.
//! Presentation never edits the definition directly; every change goes through `Draft::apply`.

mod graph;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) use graph::{Edit, EditError, Link, parameters_match};

use graph::{
    Change, capture, connections_of, links_of, next_node_id, node_name, replay, with_connections,
};
use nebula_api_contract::v1::workflow::{
    CreateWorkflowRequest, UpdateWorkflowDocumentRequest, UpdateWorkflowRequest,
    WorkflowDocumentResponse,
};
use serde_json::{Value, json};

/// Plugin assumed for a bare action key, such as one typed by hand without a plugin prefix.
const DEFAULT_PLUGIN_KEY: &str = "core";

/// Splits a catalog key such as `core.json_transform` into the plugin and the action a node stores
/// separately. A key without a plugin prefix belongs to the default plugin.
pub(crate) fn split_catalog_key(key: &str) -> (&str, &str) {
    key.split_once('.')
        .filter(|(plugin, action)| !plugin.is_empty() && !action.is_empty())
        .unwrap_or((DEFAULT_PLUGIN_KEY, key))
}

/// The catalog key of a node, `plugin.action`, which `GET /actions/{key}` resolves.
pub(crate) fn catalog_key(node: &Value) -> String {
    let action = node["action_key"].as_str().unwrap_or_default();
    let plugin = node["plugin_key"].as_str().unwrap_or(DEFAULT_PLUGIN_KEY);
    // As the workflow compiler qualifies it: an action already written with its plugin's prefix
    // keeps it.
    if action
        .strip_prefix(plugin)
        .is_some_and(|rest| rest.starts_with('.'))
    {
        return action.to_owned();
    }
    format!("{plugin}.{action}")
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
    /// Revision this app saw the server publish. The API does not report publication, so it is unknown
    /// until a publish succeeds here, and stale once a later save moves the revision on.
    pub(crate) published_revision: Option<u64>,
    /// Typing session that made the newest undo entry. Further keystrokes of that session extend the
    /// entry, so one undo takes back the whole text.
    typing: Option<u64>,
}

/// A literal parameter entry, the value itself.
pub(crate) fn literal(value: Value) -> Value {
    json!({"type": "literal", "value": value})
}

/// An expression parameter entry, evaluated when the node runs, such as `{{ $input.name }}`.
pub(crate) fn expression(text: &str) -> Value {
    json!({"type": "expression", "expr": text})
}

/// A new workflow starts with an empty graph. The server assigns identity, version and timestamps.
pub(crate) fn new_workflow_request(name: &str) -> CreateWorkflowRequest {
    CreateWorkflowRequest {
        name: name.trim().into(),
        description: None,
        definition: json!({"nodes": [], "connections": []}),
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
            published_revision: None,
            typing: None,
        })
    }

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

    /// The display name of a node, falling back to its id.
    pub(crate) fn node_name(&self, id: &str) -> String {
        node_name(&self.definition, id)
    }

    /// Connections touching a node, with their ports.
    pub(crate) fn links(&self, id: &str) -> Vec<Link> {
        links_of(&self.definition, id)
    }

    /// Records one graph edit. A rejected edit leaves the document and history untouched.
    #[tracing::instrument(name = "client.document.apply", skip_all)]
    pub(crate) fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        let change = capture(&self.definition, edit)?;
        self.record(change)?;
        self.typing = None;
        Ok(())
    }

    /// Sets or removes a parameter entry as it is typed. Keystrokes of one typing `session` in the
    /// same parameter form a single undo step; a session that ends where it began leaves none.
    pub(crate) fn type_parameter(
        &mut self,
        node: &str,
        parameter: &str,
        entry: Option<Value>,
        session: u64,
    ) -> Result<(), EditError> {
        let change = capture(
            &self.definition,
            Edit::SetParameter {
                node: node.into(),
                parameter: parameter.into(),
                entry,
            },
        )?;
        if self.typing == Some(session)
            && let Some(Change::Parameter {
                node: last_node,
                parameter: last_parameter,
                before,
                after,
            }) = self.undo.last_mut()
            && last_node == node
            && last_parameter == parameter
            && let Change::Parameter { after: typed, .. } = &change
        {
            replay(&mut self.definition, &change, true)?;
            after.clone_from(typed);
            if before == after {
                self.undo.pop();
                self.typing = None;
            }
            return Ok(());
        }
        if self.record(change)? {
            self.typing = Some(session);
        }
        Ok(())
    }

    /// Applies a captured change and makes it the newest undo step. Returns false for a change that
    /// changes nothing, such as re-placing a node where it already is, which leaves no undo entry.
    fn record(&mut self, change: Change) -> Result<bool, EditError> {
        let unchanged = match &change {
            Change::Parameter { before, after, .. } => before == after,
            Change::Position {
                before: Some(before),
                after,
                ..
            } => before == after,
            _ => false,
        };
        if unchanged {
            return Ok(false);
        }
        replay(&mut self.definition, &change, true)?;
        self.undo.push(change);
        self.redo.clear();
        Ok(true)
    }

    /// Sets a literal from JSON text, as the raw parameter editor does.
    pub(crate) fn edit(
        &mut self,
        node: &str,
        parameter: &str,
        text: &str,
    ) -> Result<(), EditError> {
        let value: Value = serde_json::from_str(text).map_err(|_| EditError::InvalidValue)?;
        self.set_literal(node, parameter, value)
    }

    /// Replaces a whole stored entry from its JSON, for kinds the form does not edit (an
    /// expression, a template, a reference). The entry must still say which kind it is.
    pub(crate) fn edit_entry(
        &mut self,
        node: &str,
        parameter: &str,
        text: &str,
    ) -> Result<(), EditError> {
        let entry: Value = serde_json::from_str(text).map_err(|_| EditError::InvalidValue)?;
        if !entry.get("type").is_some_and(Value::is_string) {
            return Err(EditError::InvalidValue);
        }
        self.set_entry(node, parameter, Some(entry))
    }

    pub(crate) fn set_literal(
        &mut self,
        node: &str,
        parameter: &str,
        value: Value,
    ) -> Result<(), EditError> {
        self.set_entry(node, parameter, Some(literal(value)))
    }

    /// Sets the whole parameter entry, or removes it with `None` so the action's default applies.
    pub(crate) fn set_entry(
        &mut self,
        node: &str,
        parameter: &str,
        entry: Option<Value>,
    ) -> Result<(), EditError> {
        self.apply(Edit::SetParameter {
            node: node.into(),
            parameter: parameter.into(),
            entry,
        })
    }

    /// Adds a node with no parameters. `key` is a catalog key (`core.json_transform`) or a bare
    /// action of the default plugin. Required inputs are validated by the server at publication.
    pub(crate) fn add_node(&mut self, key: &str, action_name: &str) -> Result<String, EditError> {
        let (plugin, action) = split_catalog_key(key);
        let id = next_node_id(&self.definition, action);
        self.apply(Edit::InsertNode {
            node: json!({
                "id": id,
                "name": action_name,
                "plugin_key": plugin,
                "action_key": action,
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

    /// Connects the default output of `from` to the default input of `to`.
    pub(crate) fn connect(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        self.apply(Edit::Connect {
            link: Link::between(from, to),
        })
    }

    /// Removes exactly this link; other links between the same nodes on other ports stay.
    pub(crate) fn disconnect(&mut self, link: &Link) -> Result<(), EditError> {
        self.apply(Edit::Disconnect { link: link.clone() })
    }

    /// Places a node on the canvas at the given top-left corner, in canvas coordinates.
    pub(crate) fn move_node(&mut self, node: &str, x: f64, y: f64) -> Result<(), EditError> {
        self.apply(Edit::MoveNode {
            node: node.into(),
            x,
            y,
        })
    }

    /// The canvas position the user chose for a node, or `None` when the layout places it.
    pub(crate) fn placed_position(&self, node: &str) -> Option<(f64, f64)> {
        let position = graph::saved_position(&self.definition, node)?;
        Some((position["x"].as_f64()?, position["y"].as_f64()?))
    }

    pub(crate) fn undo(&mut self) -> Result<(), EditError> {
        let Some(change) = self.undo.last().cloned() else {
            return Ok(());
        };
        replay(&mut self.definition, &change, false)?;
        self.undo.pop();
        self.redo.push(change);
        self.typing = None;
        Ok(())
    }

    pub(crate) fn redo(&mut self) -> Result<(), EditError> {
        let Some(change) = self.redo.last().cloned() else {
            return Ok(());
        };
        replay(&mut self.definition, &change, true)?;
        self.redo.pop();
        self.undo.push(change);
        self.typing = None;
        Ok(())
    }

    pub(crate) fn save_request(&self) -> UpdateWorkflowDocumentRequest {
        // The graph, its connections and canvas placement are the editable parts. Every other server
        // field is preserved verbatim, because the server merges top-level keys of the patch.
        let mut patch = json!({
            "nodes": self.definition["nodes"],
            "connections": connections_of(&self.definition),
        });
        // A placement removed from the draft is sent as empty metadata: left out, the server's merge
        // would keep the old positions.
        let placement = match self.definition.get("ui_metadata") {
            Some(placement) => Some(placement.clone()),
            None if self.base.definition.get("ui_metadata").is_some() => Some(json!({})),
            None => None,
        };
        if let (Some(placement), Some(object)) = (placement, patch.as_object_mut()) {
            object.insert("ui_metadata".into(), placement);
        }
        UpdateWorkflowDocumentRequest {
            expected_revision: Some(self.base.revision),
            update: UpdateWorkflowRequest {
                name: None,
                description: None,
                definition: Some(patch),
            },
        }
    }

    pub(crate) fn saved(&mut self, document: WorkflowDocumentResponse) {
        self.definition = document.definition.clone();
        self.base = document;
        self.undo.clear();
        self.redo.clear();
        self.typing = None;
        self.remote = None;
        self.uncertain_save = false;
        self.save_conflict = false;
    }

    /// Replays the recorded edits onto a freshly read snapshot. An edit whose outcome the remote
    /// already has counts as applied. An edit that cannot be honored, such as a connection to a
    /// removed node, fails and leaves this draft untouched.
    pub(crate) fn reapply(&mut self) -> Result<(), EditError> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let mut next = Self::new(remote.clone())?;
        for change in &self.undo {
            match next.apply(change.intent()) {
                Ok(()) => {},
                Err(error) if change.already_in_effect(&error, &next.definition) => {},
                Err(error) => return Err(error),
            }
        }
        next.start_key.clone_from(&self.start_key);
        next.execution_id.clone_from(&self.execution_id);
        next.published_revision = self.published_revision;
        *self = next;
        Ok(())
    }
}
