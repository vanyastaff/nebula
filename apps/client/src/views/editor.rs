//! Document editor: revision state, draft commands, server reconciliation, the graph canvas and the
//! node inspector. Graph and parameter edits are local; only the toolbar starts network work.
use super::{Intent, Intents, canvas};
use crate::{
    document::Draft,
    theme,
    widgets::{self, Tone},
    workbench::{AddNodeForm, DraftGate, Workbench, draft_gate},
};
use eframe::egui;
use serde_json::Value;

/// Everything the header and toolbar show, copied out of the draft so the frame can mutate the workbench.
struct DraftView {
    id: String,
    name: String,
    revision: u64,
    dirty: bool,
    uncertain: bool,
    conflict: bool,
    gate: DraftGate,
    can_undo: bool,
    can_redo: bool,
    remote: Option<RemoteView>,
}

struct RemoteView {
    revision: u64,
    nodes: Value,
}

impl DraftView {
    fn of(draft: &Draft) -> Self {
        Self {
            id: draft.base.workflow.id.clone(),
            name: draft.base.workflow.name.clone(),
            revision: draft.base.revision,
            dirty: draft.dirty(),
            uncertain: draft.uncertain_save,
            conflict: draft.save_conflict,
            gate: draft_gate(draft),
            can_undo: draft.can_undo(),
            can_redo: draft.can_redo(),
            remote: draft.remote.as_ref().map(|remote| RemoteView {
                revision: remote.revision,
                nodes: remote.definition["nodes"].clone(),
            }),
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(draft) = workbench.session.draft() else {
        widgets::empty_state(
            ui,
            "Choose a workflow",
            "Select a workflow to edit its graph and parameters, or create a new one.",
        );
        return;
    };
    let busy = workbench.session.busy();
    let view = DraftView::of(draft);
    widgets::title(ui, &view.name);
    ui.horizontal_wrapped(|ui| {
        widgets::badge(ui, format!("Revision {}", view.revision), Tone::Neutral);
        if view.dirty {
            widgets::badge(ui, "Unsaved changes", Tone::Warning);
        } else {
            widgets::badge(ui, "Saved", Tone::Success);
        }
        // While a write is in flight its outcome is still open; only a settled failure is unknown.
        if view.uncertain && busy {
            widgets::badge(ui, "Write in progress", Tone::Accent);
        } else if view.uncertain {
            widgets::badge(ui, "Save outcome unknown", Tone::Danger);
        }
        if view.conflict {
            widgets::badge(ui, "Server has a newer version", Tone::Danger);
        }
    });
    ui.add_space(theme::SPACE_MD);
    ui.add_enabled_ui(!busy, |ui| {
        toolbar(ui, workbench, &view, intents);
        if let Some(remote) = &view.remote {
            reconciliation(ui, workbench, remote);
        }
        ui.add_space(theme::SPACE_LG);
        widgets::section(ui, "Graph");
        canvas::show(ui, workbench);
        add_node_form(ui, workbench);
        ui.add_space(theme::SPACE_LG);
        node_inspector(ui, workbench);
    });
}

fn toolbar(ui: &mut egui::Ui, workbench: &mut Workbench, view: &DraftView, intents: &mut Intents) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(view.can_undo, egui::Button::new("Undo"))
            .clicked()
        {
            if let Some(draft) = workbench.session.draft_mut() {
                let _ = draft.undo();
            }
            workbench.parameter.close();
        }
        if ui
            .add_enabled(view.can_redo, egui::Button::new("Redo"))
            .clicked()
        {
            if let Some(draft) = workbench.session.draft_mut() {
                let _ = draft.redo();
            }
            workbench.parameter.close();
        }
        ui.add_space(theme::SPACE_SM);
        if ui
            .add_enabled(view.gate.can_save, widgets::primary_button("Save changes"))
            .clicked()
        {
            intents.push(Intent::SaveDraft);
        }
        if ui
            .add_enabled(view.gate.can_publish, egui::Button::new("Publish"))
            .clicked()
        {
            intents.push(Intent::PublishDraft);
        }
        if ui.button("Read server version").clicked() {
            intents.push(Intent::LoadWorkflow(view.id.clone()));
        }
    });
}

/// Shown when the server holds a different revision than the draft is based on.
fn reconciliation(ui: &mut egui::Ui, workbench: &mut Workbench, remote: &RemoteView) {
    ui.add_space(theme::SPACE_MD);
    widgets::banner(
        ui,
        Tone::Warning,
        &format!(
            "The server is on revision {}. Review it before saving.",
            remote.revision
        ),
    );
    egui::CollapsingHeader::new("Server nodes for comparison").show(ui, |ui| {
        let pretty = serde_json::to_string_pretty(&remote.nodes).unwrap_or_default();
        ui.monospace(pretty);
    });
    widgets::caption(
        ui,
        "Reapply replays your graph and parameter edits onto the server version. An edit that no longer applies, such as a removed node, stops the replay and keeps your draft.",
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("Reapply my edits").clicked() {
            let result = workbench.session.draft_mut().map(Draft::reapply);
            workbench.parameter.close();
            match result {
                Some(Ok(())) => workbench
                    .feedback
                    .info("Draft rebased. Review and save changes."),
                Some(Err(error)) => workbench.feedback.error(error.to_string()),
                None => {},
            }
        }
        if ui
            .add(widgets::danger_button(
                "Discard draft and use server version",
            ))
            .clicked()
        {
            if let Some(draft) = workbench.session.draft_mut()
                && let Some(remote) = draft.remote.take()
            {
                draft.saved(remote);
            }
            workbench.parameter.close();
        }
    });
}

fn add_node_form(ui: &mut egui::Ui, workbench: &mut Workbench) {
    egui::CollapsingHeader::new("Add node").show(ui, |ui| {
        widgets::labeled_field(
            ui,
            "Action key (for example json_transform)",
            &mut workbench.add_node.action_key,
            false,
        );
        widgets::labeled_field(
            ui,
            "Display name (optional)",
            &mut workbench.add_node.name,
            false,
        );
        widgets::caption(
            ui,
            "The new node starts without parameters. The server checks required inputs when you publish.",
        );
        let key = workbench.add_node.action_key.trim().to_owned();
        if ui
            .add_enabled(!key.is_empty(), widgets::primary_button("Add node"))
            .clicked()
        {
            let typed = workbench.add_node.name.trim().to_owned();
            let name = if typed.is_empty() { key.clone() } else { typed };
            let Some(draft) = workbench.session.draft_mut() else {
                return;
            };
            match draft.add_node(&key, &name) {
                Ok(id) => {
                    workbench.selected_node = Some(id);
                    workbench.rename.clone_from(&name);
                    workbench.parameter.close();
                    workbench.add_node = AddNodeForm::default();
                    workbench.feedback.info(format!("Added {name}."));
                },
                Err(error) => workbench.feedback.error(error.to_string()),
            }
        }
    });
}

/// Inspector for the selected node: rename, connections, parameters and removal.
fn node_inspector(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let Some(node_id) = workbench.selected_node.clone() else {
        widgets::caption(
            ui,
            "Select a node on the graph to rename it, edit its parameters or change its connections.",
        );
        return;
    };
    let Some(draft) = workbench.session.draft() else {
        return;
    };
    let node = draft.definition["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["id"].as_str() == Some(node_id.as_str()))
        })
        .cloned();
    let links: Vec<(String, String)> = draft
        .connections()
        .iter()
        .filter(|connection| {
            connection["from_node"].as_str() == Some(node_id.as_str())
                || connection["to_node"].as_str() == Some(node_id.as_str())
        })
        .map(|connection| {
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
        })
        .collect();
    let Some(node) = node else {
        // The node was removed by a replay or a server read; nothing to inspect.
        workbench.selected_node = None;
        return;
    };
    let action = node["action_key"].as_str().unwrap_or_default().to_owned();
    let name = node["name"].as_str().unwrap_or(&node_id).to_owned();

    theme::card_block(ui, |ui| {
        widgets::section(ui, &name);
        widgets::caption(ui, format!("{action} · id {node_id}"));
        ui.add_space(theme::SPACE_SM);
        widgets::labeled_field(ui, "Name", &mut workbench.rename, false);
        ui.horizontal_wrapped(|ui| {
            let renamed = workbench.rename.trim().to_owned();
            let changed = !renamed.is_empty() && renamed != name;
            if ui
                .add_enabled(changed, egui::Button::new("Rename"))
                .clicked()
            {
                rename(workbench, &node_id, &renamed);
            }
            if ui.add(widgets::danger_button("Remove node")).clicked() {
                remove(workbench, &node_id, &name);
            }
        });

        ui.add_space(theme::SPACE_SM);
        widgets::section(ui, "Connections");
        if links.is_empty() {
            widgets::caption(
                ui,
                "Not connected. Drag from an output port to another node's input port.",
            );
        }
        for (from, to) in &links {
            ui.horizontal_wrapped(|ui| {
                widgets::caption(ui, format!("{from} → {to}"));
                if ui.button("Disconnect").clicked() {
                    disconnect(workbench, from, to);
                }
            });
        }

        ui.add_space(theme::SPACE_SM);
        widgets::section(ui, "Parameters");
        match node["parameters"].as_object() {
            Some(parameters) if !parameters.is_empty() => {
                for (parameter, value) in parameters {
                    parameter_row(ui, workbench, &node_id, parameter, value);
                }
            },
            _ => widgets::caption(ui, "This node has no configurable parameters."),
        }
        parameter_editor(ui, workbench);
    });
}

fn rename(workbench: &mut Workbench, node: &str, name: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.rename_node(node, name) {
        Ok(()) => workbench.feedback.info("Node renamed in your draft."),
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}

fn remove(workbench: &mut Workbench, node: &str, name: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.remove_node(node) {
        Ok(()) => {
            workbench.selected_node = None;
            workbench.parameter.close();
            workbench
                .feedback
                .info(format!("Removed {name} and its connections."));
        },
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}

fn disconnect(workbench: &mut Workbench, from: &str, to: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.disconnect(from, to) {
        Ok(()) => workbench
            .feedback
            .info(format!("Disconnected {from} from {to}.")),
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}

fn parameter_row(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    node_id: &str,
    name: &str,
    value: &Value,
) {
    ui.horizontal_wrapped(|ui| {
        if value["type"].as_str() == Some("literal") {
            let selected =
                workbench.parameter.node == node_id && workbench.parameter.parameter == name;
            let entry = egui::Button::new(name)
                .selected(selected)
                .min_size(egui::vec2(140.0, 32.0));
            if ui.add(entry).clicked() {
                let text = serde_json::to_string_pretty(&value["value"]).unwrap_or_default();
                workbench.parameter.open(node_id, name, text);
            }
            let preview: String = value["value"].to_string().chars().take(64).collect();
            widgets::caption(ui, preview);
        } else {
            ui.add_enabled(
                false,
                egui::Button::new(name).min_size(egui::vec2(140.0, 32.0)),
            );
            widgets::caption(
                ui,
                format!(
                    "{} parameter · read-only in this release",
                    value["type"].as_str().unwrap_or("unknown")
                ),
            );
        }
    });
}

/// The open parameter. Applying it changes the draft locally; saving is a separate step.
fn parameter_editor(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let owns_selection =
        workbench.selected_node.as_deref() == Some(workbench.parameter.node.as_str());
    if !workbench.parameter.is_open() || !owns_selection {
        return;
    }
    ui.add_space(theme::SPACE_MD);
    widgets::section(ui, &workbench.parameter.parameter);
    widgets::caption(ui, "Edit the JSON value, then apply it to your draft.");
    ui.add_space(theme::SPACE_XS);
    ui.add(
        egui::TextEdit::multiline(&mut workbench.parameter.text)
            .code_editor()
            .desired_rows(8)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        if ui
            .add(widgets::primary_button("Apply parameter edit"))
            .clicked()
        {
            apply_parameter(workbench);
        }
        if ui.button("Close").clicked() {
            workbench.parameter.close();
        }
    });
}

fn apply_parameter(workbench: &mut Workbench) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.edit(
        &workbench.parameter.node,
        &workbench.parameter.parameter,
        &workbench.parameter.text,
    ) {
        Ok(()) => workbench.feedback.info("Parameter edited in your draft."),
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}
