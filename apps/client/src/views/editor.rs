//! Document editor, laid out like other node editors: an action bar with the revision state and the
//! commands that start network work, the graph canvas under it with its controls floating on top, and a
//! side panel that either adds nodes or inspects the selected one. Graph and parameter edits stay local
//! until the user saves.
use super::{Intent, Intents, canvas, inspector};
use crate::{
    document::Draft,
    theme,
    widgets::{self, Tone},
    workbench::{CATALOG_UNAVAILABLE, Catalog, DraftGate, Workbench, draft_gate},
};
use eframe::egui::{self, Align, Layout, RichText};
use serde_json::Value;

/// Canvas height when the page scrolls (narrow layouts), where it cannot take the remaining height.
const STACKED_CANVAS_HEIGHT: f32 = 360.0;
/// Least canvas height on wide layouts, so a short window still shows a usable graph.
const MIN_CANVAS_HEIGHT: f32 = 240.0;
/// Narrower pages put the action bar's commands on a second row instead of over the title.
const SINGLE_ROW_BAR_MIN: f32 = 720.0;

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
    /// This app saw the server publish exactly the revision being edited.
    published: bool,
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
            published: draft.published_revision == Some(draft.base.revision),
            remote: draft.remote.as_ref().map(|remote| RemoteView {
                revision: remote.revision,
                nodes: remote.definition["nodes"].clone(),
            }),
        }
    }
}

/// True when the side panel has something to show: the palette while it is open, else the selected node.
pub(crate) fn has_side_panel(workbench: &Workbench) -> bool {
    workbench.session.draft().is_some()
        && (workbench.add_node.open || workbench.selected_node.is_some())
}

/// `stacked` is set on narrow layouts, where the page scrolls and the canvas has a fixed height.
pub(crate) fn show(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    stacked: bool,
) {
    let Some(draft) = workbench.session.draft() else {
        no_workflow(ui, workbench);
        return;
    };
    let busy = workbench.session.busy();
    let view = DraftView::of(draft);
    shortcuts(ui, workbench, &view, intents);
    action_bar(ui, workbench, &view, intents, stacked);
    ui.add_enabled_ui(!busy, |ui| {
        if let Some(remote) = &view.remote {
            reconciliation(ui, workbench, remote);
        }
        ui.add_space(theme::SPACE_SM);
        let height = if stacked {
            STACKED_CANVAS_HEIGHT
        } else {
            ui.available_height().max(MIN_CANVAS_HEIGHT)
        };
        graph(ui, workbench, &view, intents, height);
    });
}

/// Keyboard shortcuts of the editor. Saving and running work anywhere; graph edits wait while a text
/// field has the keyboard, so the field keeps its own undo and delete.
fn shortcuts(ui: &egui::Ui, workbench: &mut Workbench, view: &DraftView, intents: &mut Intents) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let pressed = |modifiers: Modifiers, key: Key| {
        ui.ctx()
            .input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
    };
    let idle = !workbench.session.busy();
    if pressed(Modifiers::COMMAND, Key::S) && idle && view.gate.can_save {
        intents.push(Intent::SaveDraft);
    }
    if pressed(Modifiers::COMMAND, Key::Enter) && idle && (view.gate.can_run || view.pending_run) {
        intents.push(Intent::RunDraft);
    }
    if ui.ctx().text_edit_focused() || !idle {
        return;
    }
    // Shortcut matching ignores an extra Shift, so redo is checked before undo.
    let shift = Modifiers::COMMAND | Modifiers::SHIFT;
    if pressed(shift, Key::Z) || pressed(Modifiers::COMMAND, Key::Y) {
        redo(workbench);
    } else if pressed(Modifiers::COMMAND, Key::Z) {
        undo(workbench);
    }
    if pressed(Modifiers::NONE, Key::Delete) {
        inspector::remove_selected(workbench);
    }
    if pressed(Modifiers::NONE, Key::Escape) {
        if workbench.add_node.open {
            workbench.add_node.close();
        } else {
            workbench.selected_node = None;
            workbench.parameter.close();
        }
    }
}

/// The page before a workflow is open: what to do next, with the shortest way to start.
fn no_workflow(ui: &mut egui::Ui, workbench: &mut Workbench) {
    // About a third of the way down a full page; a scrolling page has no height to share, so it caps.
    ui.add_space((ui.available_height() * 0.3).clamp(theme::SPACE_XL, 240.0));
    ui.vertical_centered(|ui| {
        widgets::section(ui, "No workflow open");
        widgets::caption(ui, "Pick a workflow from the list, or start a new one.");
        ui.add_space(theme::SPACE_SM);
        if ui.add(widgets::primary_button("New workflow")).clicked() {
            workbench.navigator.start_creating();
            workbench.sidebar_open = true;
        }
    });
}

/// The canvas with its controls floating over the corners: adding a node at the top right, where the
/// layout never places a node, zoom at the bottom left, and running the workflow at the bottom centre.
fn graph(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    view: &DraftView,
    intents: &mut Intents,
    height: f32,
) {
    let Some(frame) = canvas::show(ui, workbench, height) else {
        return;
    };
    let inset = frame.viewport.shrink(theme::SPACE_MD);
    let overlay = |ui: &mut egui::Ui, layout: Layout| {
        ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(layout))
    };
    if overlay(ui, Layout::top_down(Align::Max))
        .button("+ Add node")
        .on_hover_text("Add a node from the action catalog")
        .clicked()
    {
        workbench.selected_node = None;
        workbench.add_node.open_after(None);
    }
    canvas::zoom_controls(
        &mut overlay(ui, Layout::bottom_up(Align::Min)),
        workbench,
        &frame,
    );
    run_button(
        &mut overlay(ui, Layout::bottom_up(Align::Center)),
        view,
        intents,
    );
}

/// Commands of the action bar, in reading order.
#[derive(Clone, Copy)]
enum Command {
    Undo,
    Redo,
    Reload,
    Publish,
    Save,
}

const COMMANDS: [Command; 5] = [
    Command::Undo,
    Command::Redo,
    Command::Reload,
    Command::Publish,
    Command::Save,
];

/// Name and state on the left, commands on the right. Narrow layouts put the commands on their own row.
fn action_bar(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    view: &DraftView,
    intents: &mut Intents,
    stacked: bool,
) {
    let busy = workbench.session.busy();
    // Side panels can leave the page too narrow for one row; the commands then get their own.
    if stacked || ui.available_width() < SINGLE_ROW_BAR_MIN {
        ui.horizontal_wrapped(|ui| identity(ui, view, busy));
        ui.horizontal_wrapped(|ui| {
            for item in COMMANDS {
                command(ui, workbench, view, intents, item);
            }
        });
    } else {
        ui.horizontal(|ui| {
            identity(ui, view, busy);
            // A right-to-left layout puts the first item at the right edge, so it gets the items reversed.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for item in COMMANDS.iter().rev() {
                    command(ui, workbench, view, intents, *item);
                }
            });
        });
    }
}

fn identity(ui: &mut egui::Ui, view: &DraftView, busy: bool) {
    ui.label(RichText::new(&view.name).size(18.0).strong());
    // While a write is in flight its outcome is still open; only a settled failure is unknown.
    let (state, tone) = if view.uncertain && busy {
        ("Saving…", Tone::Accent)
    } else if view.uncertain {
        ("Save outcome unknown", Tone::Danger)
    } else if view.dirty {
        ("Unsaved changes", Tone::Warning)
    } else if view.published {
        ("Published", Tone::Success)
    } else {
        ("Saved", Tone::Success)
    };
    widgets::badge(ui, state, tone);
    if view.conflict {
        widgets::badge(ui, "Server has a newer version", Tone::Danger);
    }
    widgets::caption(ui, format!("Revision {}", view.revision));
}

fn command(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    view: &DraftView,
    intents: &mut Intents,
    command: Command,
) {
    let busy = workbench.session.busy();
    let gate = view.gate;
    match command {
        Command::Undo => {
            if ui
                .add_enabled(view.can_undo && !busy, egui::Button::new("Undo"))
                .on_hover_text("Undo the last edit (Ctrl+Z)")
                .clicked()
            {
                undo(workbench);
            }
        },
        Command::Redo => {
            if ui
                .add_enabled(view.can_redo && !busy, egui::Button::new("Redo"))
                .on_hover_text("Redo the edit (Ctrl+Shift+Z)")
                .clicked()
            {
                redo(workbench);
            }
        },
        Command::Reload => {
            if ui
                .add_enabled(!busy, egui::Button::new("Reload"))
                .on_hover_text("Read the server's version of this workflow")
                .clicked()
            {
                intents.push(Intent::LoadWorkflow(view.id.clone()));
            }
        },
        // Save and publish are never both possible, so at most one of them is the primary action.
        Command::Publish => {
            // Once this revision is known to be published, publishing again is allowed but not urged.
            let button = if gate.can_publish && !view.published {
                widgets::primary_button("Publish")
            } else {
                egui::Button::new("Publish")
            };
            if ui
                .add_enabled(gate.can_publish && !busy, button)
                .on_hover_text("Make this revision the one that runs")
                .on_disabled_hover_text("Save your changes before publishing them")
                .clicked()
            {
                intents.push(Intent::PublishDraft);
            }
        },
        Command::Save => {
            let button = if gate.can_save {
                widgets::primary_button("Save")
            } else {
                egui::Button::new("Save")
            };
            if ui
                .add_enabled(gate.can_save && !busy, button)
                .on_hover_text("Save your edits to the server (Ctrl+S)")
                .on_disabled_hover_text("Nothing to save")
                .clicked()
            {
                intents.push(Intent::SaveDraft);
            }
        },
    }
}

pub(crate) fn undo(workbench: &mut Workbench) {
    if let Some(draft) = workbench.session.draft_mut() {
        let result = draft.undo();
        workbench.parameter.close();
        workbench.feedback.report(result, "Undid the last edit.");
    }
}

pub(crate) fn redo(workbench: &mut Workbench) {
    if let Some(draft) = workbench.session.draft_mut() {
        let result = draft.redo();
        workbench.parameter.close();
        workbench.feedback.report(result, "Redid the edit.");
    }
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

/// The main action on the canvas, as in node editors. It runs the server's current publication, so it
/// stays disabled while the draft has unsaved or unreviewed changes, and says why on hover.
fn run_button(ui: &mut egui::Ui, view: &DraftView, intents: &mut Intents) {
    let (label, enabled) = if view.pending_run {
        ("Reconcile pending run", true)
    } else {
        ("Execute workflow", view.gate.can_run)
    };
    let blocker = if view.dirty {
        "Save and publish your changes first. Runs use the published version."
    } else {
        "Review the server's version first."
    };
    let button = widgets::primary_button(label).min_size(egui::vec2(180.0, 38.0));
    if ui
        .add_enabled(enabled, button)
        .on_hover_text("Run the published version of this workflow (Ctrl+Enter)")
        .on_disabled_hover_text(blocker)
        .clicked()
    {
        intents.push(Intent::RunDraft);
    }
}

/// The side panel: the add-node palette while it is open, otherwise the selected node.
pub(crate) fn side(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if workbench.add_node.open {
        palette(ui, workbench, intents);
    } else {
        inspector::show(ui, workbench);
    }
}

fn palette(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if widgets::panel_header(ui, "Add node") {
        workbench.add_node.close();
        return;
    }
    if let Some(from) = workbench.add_node.connect_from.clone() {
        let source = workbench
            .session
            .draft()
            .map_or_else(|| from.clone(), |draft| draft.node_name(&from));
        ui.horizontal_wrapped(|ui| {
            widgets::caption(ui, format!("Connects after {source}."));
            if ui.small_button("Don't connect").clicked() {
                workbench.add_node.connect_from = None;
            }
        });
    }
    ui.add_space(theme::SPACE_SM);
    if let Some((key, name)) = catalog(ui, workbench, intents) {
        add_node(workbench, &key, &name);
        return;
    }
    ui.add_space(theme::SPACE_MD);
    ui.separator();
    add_node_fields(ui, workbench);
}

/// The server's action catalog, asked for the first time the palette opens. Returns the key and name of
/// an action the user picked.
fn catalog(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
) -> Option<(String, String)> {
    let mut chosen = None;
    match &workbench.catalog {
        Catalog::NotRequested => {
            intents.push(Intent::LoadCatalog);
            ui.horizontal(|ui| {
                ui.spinner();
                widgets::caption(ui, "Loading the action catalog…");
            });
        },
        Catalog::Unavailable => {
            widgets::caption(ui, CATALOG_UNAVAILABLE);
            if ui.button("Check again").clicked() {
                intents.push(Intent::LoadCatalog);
            }
        },
        Catalog::Ready(actions) => {
            ui.add(widgets::field(&mut workbench.add_node.filter).hint_text("Search actions"));
            let filter = workbench.add_node.filter.trim().to_lowercase();
            let matches: Vec<_> = actions
                .iter()
                .filter(|action| {
                    filter.is_empty()
                        || action.name.to_lowercase().contains(&filter)
                        || action.key.to_lowercase().contains(&filter)
                })
                .collect();
            if matches.is_empty() {
                widgets::caption(ui, "No action matches the search.");
            }
            egui::ScrollArea::vertical()
                .id_salt("action-catalog")
                .max_height(320.0)
                .show(ui, |ui| {
                    for action in matches {
                        let row = egui::Button::new(RichText::new(&action.name).strong())
                            .right_text(RichText::new(&action.key).color(theme::TEXT_MUTED))
                            .frame_when_inactive(false)
                            .min_size(egui::vec2(ui.available_width(), 34.0));
                        if ui
                            .add(row)
                            .on_hover_text(format!("Add {} to the graph", action.name))
                            .clicked()
                        {
                            chosen = Some((action.key.clone(), action.name.clone()));
                        }
                    }
                });
        },
    }
    chosen
}

/// A hand-typed action, for keys the catalog does not list or servers without a catalog.
fn add_node_fields(ui: &mut egui::Ui, workbench: &mut Workbench) {
    // With a catalog the typed key is the alternative; without one the catalog message already says so.
    if matches!(workbench.catalog, Catalog::Ready(_)) {
        widgets::caption(ui, "Or add an action by its key.");
    }
    let key_field =
        widgets::labeled_field(ui, "Action key", &mut workbench.add_node.action_key, false);
    let name_field = widgets::labeled_field(
        ui,
        "Display name (optional)",
        &mut workbench.add_node.name,
        false,
    );
    let entered = widgets::submitted(ui, &key_field) || widgets::submitted(ui, &name_field);
    let key = workbench.add_node.action_key.trim().to_owned();
    let clicked = ui
        .add_enabled(!key.is_empty(), egui::Button::new("Add node"))
        .on_hover_text("New nodes start without parameters; the server checks inputs on publish")
        .clicked();
    if !key.is_empty() && (clicked || entered) {
        let typed = workbench.add_node.name.trim().to_owned();
        let name = if typed.is_empty() { key.clone() } else { typed };
        add_node(workbench, &key, &name);
    }
}

fn add_node(workbench: &mut Workbench, action_key: &str, name: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.add_node(action_key, name) {
        Ok(id) => {
            let linked = workbench
                .add_node
                .connect_from
                .take()
                .map(|from| draft.connect(&from, &id));
            workbench.selected_node = Some(id);
            name.clone_into(&mut workbench.rename);
            workbench.parameter.close();
            workbench.add_node.close();
            match linked {
                Some(Err(error)) => workbench.feedback.error(error.to_string()),
                _ => workbench.feedback.info(format!("Added {name}.")),
            }
        },
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}
