//! Document editor: revision state, draft commands, server reconciliation, nodes and the parameter inspector.
use super::{Intent, Intents};
use crate::{
    document::Draft,
    theme,
    widgets::{self, Tone},
    workbench::{DraftGate, Workbench, draft_gate},
};
use eframe::egui::{self, RichText};
use serde_json::Value;

/// Everything the editor shows, copied out of the draft so the frame can mutate the workbench.
struct DraftView {
    id: String,
    name: String,
    revision: u64,
    dirty: bool,
    uncertain: bool,
    conflict: bool,
    gate: DraftGate,
    remote: Option<RemoteView>,
    nodes: Vec<Value>,
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
            remote: draft.remote.as_ref().map(|remote| RemoteView {
                revision: remote.revision,
                nodes: remote.definition["nodes"].clone(),
            }),
            nodes: draft.definition["nodes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(draft) = workbench.session.draft() else {
        widgets::empty_state(
            ui,
            "Choose a workflow",
            "Select a workflow to inspect its nodes and edit parameters, or create a new one.",
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
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        toolbar(ui, workbench, &view, intents);
        if let Some(remote) = &view.remote {
            reconciliation(ui, workbench, remote);
        }
        ui.add_space(theme::SPACE_LG);
        widgets::section(ui, "Nodes");
        if view.nodes.is_empty() {
            widgets::caption(
                ui,
                "This workflow has no nodes yet. Node editing comes in a later release.",
            );
        }
        for node in &view.nodes {
            node_card(ui, workbench, node);
        }
        inspector(ui, workbench);
    });
}

fn toolbar(ui: &mut egui::Ui, workbench: &mut Workbench, view: &DraftView, intents: &mut Intents) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Undo").clicked() {
            if let Some(draft) = workbench.session.draft_mut() {
                let _ = draft.undo();
            }
            workbench.parameter.close();
        }
        if ui.button("Redo").clicked() {
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
        "Reapply replays your parameter edits onto the server version. It overwrites only the parameters you edited, including ones changed on the server. Other server changes are kept.",
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("Reapply my parameter edits").clicked() {
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

fn node_card(ui: &mut egui::Ui, workbench: &mut Workbench, node: &Value) {
    let node_id = node["id"].as_str().unwrap_or_default();
    let title = node["name"].as_str().unwrap_or(node_id);
    let action = node["action_key"].as_str().unwrap_or_default();
    theme::card_block(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(title).strong().size(16.0));
            widgets::caption(ui, action);
        });
        let Some(parameters) = node["parameters"].as_object() else {
            widgets::caption(ui, "This node has no configurable parameters.");
            return;
        };
        for (name, value) in parameters {
            parameter_row(ui, workbench, node_id, name, value);
        }
    });
    ui.add_space(theme::SPACE_SM);
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

/// The parameter currently open. Applying it changes the draft locally; saving is a separate step.
fn inspector(ui: &mut egui::Ui, workbench: &mut Workbench) {
    if !workbench.parameter.is_open() {
        return;
    }
    ui.add_space(theme::SPACE_LG);
    theme::card_block(ui, |ui| {
        widgets::section(ui, &workbench.parameter.parameter);
        widgets::caption(
            ui,
            format!(
                "Node {} · edit the JSON value, then apply it to your draft.",
                workbench.parameter.node
            ),
        );
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
