//! Workflow document editing: the draft, its undo history, and what a server revision means for it.
//! Presentation never edits the definition directly; every change goes through `Draft::apply`.

mod graph;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) use graph::{Edit, EditError, parameters_match};

use graph::{
    Change, capture, connections_of, links_of, next_node_id, node_name, replay, with_connections,
};
use nebula_api_contract::v1::workflow::{
    CreateWorkflowRequest, UpdateWorkflowDocumentRequest, UpdateWorkflowRequest,
    WorkflowDocumentResponse,
};
use serde_json::{Value, json};

/// Plugin for nodes added from the action catalog. The catalog does not report plugins yet.
const CATALOG_PLUGIN_KEY: &str = "core";

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

    /// Connections touching a node, as (from, to) pairs.
    pub(crate) fn links(&self, id: &str) -> Vec<(String, String)> {
        links_of(&self.definition, id)
    }

    /// Records one graph edit. A rejected edit leaves the document and history untouched.
    #[tracing::instrument(name = "client.document.apply", skip_all)]
    pub(crate) fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        let change = capture(&self.definition, edit)?;
        // Re-placing a node where it already is is not a change, so it leaves no undo entry.
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
            return Ok(());
        }
        replay(&mut self.definition, &change, true)?;
        self.undo.push(change);
        self.redo.clear();
        Ok(())
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

    pub(crate) fn set_literal(
        &mut self,
        node: &str,
        parameter: &str,
        value: Value,
    ) -> Result<(), EditError> {
        self.set_entry(
            node,
            parameter,
            Some(json!({"type": "literal", "value": value})),
        )
    }

    /// An expression the engine evaluates at run time, such as `{{ $input.name }}`.
    pub(crate) fn set_expression(
        &mut self,
        node: &str,
        parameter: &str,
        expression: &str,
    ) -> Result<(), EditError> {
        self.set_entry(
            node,
            parameter,
            Some(json!({"type": "expression", "expr": expression})),
        )
    }

    /// Removes the parameter, so the action's default applies.
    pub(crate) fn clear_parameter(&mut self, node: &str, parameter: &str) -> Result<(), EditError> {
        self.set_entry(node, parameter, None)
    }

    fn set_entry(
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
        // The graph, its connections and canvas placement are the editable parts. Every other server
        // field is preserved verbatim, because the server merges top-level keys of the patch.
        let mut patch = json!({
            "nodes": self.definition["nodes"],
            "connections": connections_of(&self.definition),
        });
        if let (Some(placement), Some(object)) =
            (self.definition.get("ui_metadata"), patch.as_object_mut())
        {
            object.insert("ui_metadata".into(), placement.clone());
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
                Err(error) if change.already_in_effect(&error) => {},
                Err(error) => return Err(error),
            }
        }
        next.start_key.clone_from(&self.start_key);
        next.execution_id.clone_from(&self.execution_id);
        *self = next;
        Ok(())
    }
}
