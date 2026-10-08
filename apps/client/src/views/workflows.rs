//! The workflows of the open workspace as a page: create one, search the listed page, open one in the
//! editor, and page through the rest.

use super::{Intent, Intents, states};
use crate::{
    clock, theme,
    widgets::{self, Tone},
    workbench::{Remote, Workbench},
};
use eframe::egui::{self, RichText};

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.navigator.read.wants_read() {
        intents.push(Intent::ListWorkflows(workbench.navigator.page.max(1)));
    }
    let total = workbench.navigator.total;
    let summary = match total {
        0 => "Workflows stored in this workspace.".to_owned(),
        1 => "1 workflow in this workspace.".to_owned(),
        count => format!("{count} workflows in this workspace."),
    };
    states::page_header(ui, "Workflows", &summary, |ui| {
        if ui
            .add_enabled(!busy, widgets::primary_button("New workflow"))
            .on_hover_text("Create a workflow with an empty graph")
            .clicked()
        {
            workbench.navigator.start_creating();
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Refresh"))
            .on_hover_text("Read the list again")
            .clicked()
        {
            intents.push(Intent::ListWorkflows(workbench.navigator.page.max(1)));
        }
    });
    if workbench.navigator.creating {
        theme::card_block(ui, |ui| create_form(ui, workbench, intents, busy));
        ui.add_space(theme::SPACE_MD);
    }
    match &workbench.navigator.read {
        Remote::Idle | Remote::Loading => {
            states::loading(ui, "Reading workflows…");
            return;
        },
        Remote::Failed(reason) => {
            if states::failed(ui, "The workflows could not be read.", reason) {
                intents.push(Intent::ListWorkflows(workbench.navigator.page.max(1)));
            }
            return;
        },
        Remote::Ready(()) | Remote::Reloading(()) | Remote::Stale(()) => {},
    }
    if workbench.navigator.workflows.is_empty() {
        if !workbench.navigator.creating
            && states::empty(
                ui,
                "No workflows yet",
                "A workflow is a graph of actions. Create one, then add its nodes on the canvas.",
                Some("Create a workflow"),
            )
        {
            workbench.navigator.start_creating();
        }
        return;
    }
    let search = ui.add(
        widgets::field(&mut workbench.navigator.filter)
            .hint_text("Search this page by name")
            .desired_width(f32::INFINITY),
    );
    widgets::named(ui, &search, "Search workflows on this page");
    ui.add_space(theme::SPACE_SM);
    rows(ui, workbench, intents, busy);
    paging(ui, workbench, intents, busy);
    ui.add_space(theme::SPACE_MD);
    widgets::caption(
        ui,
        "Unsaved drafts stay in this app until you close it, even after you sign out.",
    );
}

/// Inline creation: a name, then Create or Enter.
fn create_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "New workflow");
    let field = widgets::labeled_field(ui, "Name", &mut workbench.navigator.new_name, false);
    if std::mem::take(&mut workbench.navigator.focus_name) {
        field.request_focus();
    }
    let ready = !busy && !workbench.navigator.new_name.trim().is_empty();
    let submitted = widgets::submitted(ui, &field);
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        let create = ui
            .add_enabled(ready, widgets::primary_button("Create"))
            .clicked();
        if create || (submitted && ready) {
            intents.push(Intent::CreateWorkflow);
        }
        if ui.button("Cancel").clicked() {
            workbench.navigator.creating = false;
            workbench.navigator.new_name.clear();
        }
    });
}

/// One card per workflow, which opens it; its description, an unsaved marker and when it last
/// changed read on it.
fn rows(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let now = clock::now_millis();
    let visible: Vec<(String, String, Option<String>, i64)> = workbench
        .navigator
        .visible()
        .into_iter()
        .map(|workflow| {
            (
                workflow.id.clone(),
                workflow.name.clone(),
                workflow.description.clone(),
                workflow.updated_at,
            )
        })
        .collect();
    if visible.is_empty() {
        states::empty(
            ui,
            "No match",
            "No workflow on this page has that in its name.",
            None,
        );
        return;
    }
    let mut opened = None;
    for (id, name, description, updated) in visible {
        let unsaved = workbench.has_unsaved(&id);
        let card = theme::card()
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Coloured by name, which stays put, unlike ids a fresh workspace hands out.
                    widgets::mark(
                        ui,
                        &widgets::initial(&name),
                        theme::node_accent(&name),
                        32.0,
                    );
                    // Everything after the mark wraps in one column, so no part of the card asks
                    // for width the window does not have.
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&name).strong().size(theme::SIZE_LABEL));
                            if unsaved {
                                widgets::badge(ui, "Unsaved", Tone::Warning);
                            }
                        });
                        if let Some(description) = &description {
                            widgets::caption(ui, description.as_str());
                        }
                        widgets::caption(
                            ui,
                            format!("Updated {}", clock::ago(now, updated * 1000)),
                        );
                    });
                });
            })
            .response;
        if widgets::card_clicked(&card, &format!("Open {name}"), !busy) {
            opened = Some(id.clone());
        }
        ui.add_space(theme::SPACE_XS);
    }
    if let Some(id) = opened
        && workbench.select_workflow(&id)
    {
        intents.push(Intent::LoadWorkflow(id));
    }
}

fn paging(ui: &mut egui::Ui, workbench: &Workbench, intents: &mut Intents, busy: bool) {
    let navigator = &workbench.navigator;
    if !navigator.has_previous_page() && !navigator.has_next_page() {
        return;
    }
    let page = navigator.page;
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !busy && navigator.has_previous_page(),
                egui::Button::new("Previous"),
            )
            .clicked()
        {
            intents.push(Intent::ListWorkflows(page - 1));
        }
        widgets::caption(ui, format!("Page {page}"));
        if ui
            .add_enabled(
                !busy && navigator.has_next_page(),
                egui::Button::new("Next"),
            )
            .clicked()
        {
            intents.push(Intent::ListWorkflows(page + 1));
        }
    });
}
