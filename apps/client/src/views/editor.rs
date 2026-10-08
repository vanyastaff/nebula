//! Document editor: revision state, the toolbar that starts network work, server reconciliation, and the
//! graph canvas with its inspector. Graph and parameter edits stay local until the user saves.
use super::{Intent, Intents, canvas, inspector};
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
    /// A run was accepted but its receipt is unknown, so the same start must be reconciled.
    pending_run: bool,
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
            pending_run: draft.start_key.is_some(),
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
        run_button(ui, &view, intents);
        add_node_form(ui, workbench);
        ui.add_space(theme::SPACE_LG);
        inspector::show(ui, workbench);
    });
}

fn toolbar(ui: &mut egui::Ui, workbench: &mut Workbench, view: &DraftView, intents: &mut Intents) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(view.can_undo, egui::Button::new("Undo"))
            .clicked()
            && let Some(draft) = workbench.session.draft_mut()
        {
            let result = draft.undo();
            workbench.parameter.close();
            workbench.feedback.report(result, "Undid the last edit.");
        }
        if ui
            .add_enabled(view.can_redo, egui::Button::new("Redo"))
            .clicked()
            && let Some(draft) = workbench.session.draft_mut()
        {
            let result = draft.redo();
            workbench.parameter.close();
            workbench.feedback.report(result, "Redid the edit.");
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
        if ui.button("Reapply my edits").clicked()
            && let Some(draft) = workbench.session.draft_mut()
        {
            let result = draft.reapply();
            workbench.parameter.close();
            workbench
                .feedback
                .report(result, "Draft rebased. Review and save changes.");
        }
        if ui
            .add(widgets::danger_button(
                "Discard draft and use server version",
            ))
            .clicked()
            && let Some(draft) = workbench.session.draft_mut()
            && let Some(remote) = draft.remote.take()
        {
            draft.saved(remote);
            workbench.parameter.close();
        }
    });
}

/// The primary run action under the canvas, as in node editors. It runs the server's current publication,
/// so it stays disabled while the draft has unsaved or unreviewed changes.
fn run_button(ui: &mut egui::Ui, view: &DraftView, intents: &mut Intents) {
    ui.add_space(theme::SPACE_SM);
    ui.vertical_centered(|ui| {
        let (label, enabled) = if view.pending_run {
            ("Reconcile pending run", true)
        } else {
            ("Execute workflow", view.gate.can_run)
        };
        if ui
            .add_enabled(enabled, widgets::primary_button(label))
            .clicked()
        {
            intents.push(Intent::RunDraft);
        }
    });
}

fn add_node_form(ui: &mut egui::Ui, workbench: &mut Workbench) {
    // A "+" on the canvas asks for the form to open once, so the user sees where the new node goes.
    let open = std::mem::take(&mut workbench.add_node.open_requested);
    egui::CollapsingHeader::new("Add node")
        .open(open.then_some(true))
        .show(ui, |ui| {
            if let Some(from) = workbench.add_node.connect_from.clone() {
                let source = workbench
                    .session
                    .draft()
                    .map_or_else(|| from.clone(), |draft| draft.node_name(&from));
                ui.horizontal_wrapped(|ui| {
                    widgets::caption(ui, format!("The new node connects after {source}."));
                    if ui.button("Clear").clicked() {
                        workbench.add_node.connect_from = None;
                    }
                });
            }
            add_node_fields(ui, workbench);
        });
}

fn add_node_fields(ui: &mut egui::Ui, workbench: &mut Workbench) {
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
        add_node(workbench, &key);
    }
}

fn add_node(workbench: &mut Workbench, action_key: &str) {
    let typed = workbench.add_node.name.trim().to_owned();
    let name = if typed.is_empty() {
        action_key.to_owned()
    } else {
        typed
    };
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.add_node(action_key, &name) {
        Ok(id) => {
            let linked = workbench
                .add_node
                .connect_from
                .take()
                .map(|from| draft.connect(&from, &id));
            workbench.selected_node = Some(id);
            workbench.rename.clone_from(&name);
            workbench.parameter.close();
            workbench.add_node = AddNodeForm::default();
            match linked {
                Some(Err(error)) => workbench.feedback.error(error.to_string()),
                _ => workbench.feedback.info(format!("Added {name}.")),
            }
        },
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}
