//! Workflow editing commands and draft recovery, independent of presentation.

use nebula_api_contract::v1::workflow::{
    CreateWorkflowRequest, UpdateWorkflowDocumentRequest, UpdateWorkflowRequest,
    WorkflowDocumentResponse,
};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum EditError {
    #[error("This server does not expose an editable workflow document.")]
    UnsupportedDocument,
    #[error("The selected node or literal parameter no longer exists.")]
    MissingParameter,
    #[error("Enter a valid JSON value for the parameter.")]
    InvalidValue,
}

#[derive(Clone)]
struct ParameterEdit {
    node: String,
    parameter: String,
    before: Value,
    after: Value,
}

pub(crate) struct Draft {
    pub(crate) base: WorkflowDocumentResponse,
    pub(crate) definition: Value,
    undo: Vec<ParameterEdit>,
    redo: Vec<ParameterEdit>,
    /// A read never replaces an unsaved draft. Conflict recovery is explicit.
    pub(crate) remote: Option<WorkflowDocumentResponse>,
    pub(crate) uncertain_save: bool,
    pub(crate) save_conflict: bool,
    /// Set before dispatch; retained through disconnect until a receipt is known.
    pub(crate) start_key: Option<String>,
    pub(crate) execution_id: Option<String>,
}

/// A new workflow starts with an empty graph. The server assigns identity, version and timestamps.
pub(crate) fn new_workflow_request(name: &str) -> CreateWorkflowRequest {
    CreateWorkflowRequest {
        name: name.trim().into(),
        description: None,
        definition: json!({"nodes": [], "connections": []}),
    }
}

fn parameter_mut<'a>(
    definition: &'a mut Value,
    node: &str,
    parameter: &str,
) -> Result<&'a mut Value, EditError> {
    let nodes = definition
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or(EditError::MissingParameter)?;
    let entry = nodes
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(node))
        .ok_or(EditError::MissingParameter)?;
    let value = entry
        .get_mut("parameters")
        .and_then(|parameters| parameters.get_mut(parameter))
        .ok_or(EditError::MissingParameter)?;
    if value["type"].as_str() != Some("literal") {
        return Err(EditError::MissingParameter);
    }
    value.get_mut("value").ok_or(EditError::MissingParameter)
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

    pub(crate) fn dirty(&self) -> bool {
        self.definition != self.base.definition
    }

    #[tracing::instrument(name = "client.document.edit", skip_all)]
    pub(crate) fn edit(
        &mut self,
        node: &str,
        parameter: &str,
        text: &str,
    ) -> Result<(), EditError> {
        let after: Value = serde_json::from_str(text).map_err(|_| EditError::InvalidValue)?;
        let slot = parameter_mut(&mut self.definition, node, parameter)?;
        if *slot != after {
            let before = std::mem::replace(slot, after.clone());
            self.undo.push(ParameterEdit {
                node: node.into(),
                parameter: parameter.into(),
                before,
                after,
            });
            self.redo.clear();
        }
        Ok(())
    }

    pub(crate) fn undo(&mut self) -> Result<(), EditError> {
        if let Some(edit) = self.undo.last() {
            *parameter_mut(&mut self.definition, &edit.node, &edit.parameter)? =
                edit.before.clone();
        }
        if let Some(edit) = self.undo.pop() {
            self.redo.push(edit);
        }
        Ok(())
    }

    pub(crate) fn redo(&mut self) -> Result<(), EditError> {
        if let Some(edit) = self.redo.last() {
            *parameter_mut(&mut self.definition, &edit.node, &edit.parameter)? = edit.after.clone();
        }
        if let Some(edit) = self.redo.pop() {
            self.undo.push(edit);
        }
        Ok(())
    }

    pub(crate) fn save_request(&self) -> UpdateWorkflowDocumentRequest {
        // Only the nodes field is edited. Preserve every other server field verbatim.
        UpdateWorkflowDocumentRequest {
            expected_revision: Some(self.base.revision),
            update: UpdateWorkflowRequest {
                name: None,
                description: None,
                definition: Some(json!({"nodes": self.definition["nodes"]})),
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

    /// Reapply only the operator's parameter commands to a freshly read snapshot.
    /// The presentation must make any overlapping overwrite explicit before calling.
    pub(crate) fn reapply(&mut self) -> Result<(), EditError> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let mut next = Self::new(remote.clone())?;
        for edit in &self.undo {
            let slot = parameter_mut(&mut next.definition, &edit.node, &edit.parameter)?;
            let before = std::mem::replace(slot, edit.after.clone());
            next.undo.push(ParameterEdit {
                before,
                ..edit.clone()
            });
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
    fn blank_workflow_request_trims_the_name_and_sends_a_loadable_empty_graph() {
        let request = new_workflow_request("  Echo  ");
        assert_eq!(request.name, "Echo");
        assert_eq!(request.definition, json!({"nodes": [], "connections": []}));
    }
    #[test]
    fn deleted_node_during_conflict_does_not_destroy_local_draft() {
        let mut draft = Draft::new(snapshot(1, 7)).unwrap();
        draft.edit("echo", "message", "9").unwrap();
        let mut remote = snapshot(2, 8);
        remote.definition["nodes"] = json!([]);
        draft.remote = Some(remote);
        assert_eq!(draft.reapply(), Err(EditError::MissingParameter));
        assert!(draft.dirty());
        assert_eq!(draft.base.revision, 1);
    }
}
