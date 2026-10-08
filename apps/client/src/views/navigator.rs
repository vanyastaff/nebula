//! Workflow list of the open workspace, shown in the sidebar: a header with creation, a filter, one row
//! per workflow, paging when the workspace has more than one page, and the count with a refresh.
use super::{Intent, Intents};
use crate::{theme, widgets, workbench::Workbench};
use eframe::egui::{self, Align, Layout, RichText};

const ROW_HEIGHT: f32 = 32.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if !workbench.workspace_open() {
        return;
    }
    let busy = workbench.session.busy();
    header(ui, workbench, busy);
    if workbench.navigator.creating {
        create_form(ui, workbench, intents, busy);
    }
    if workbench.navigator.workflows.is_empty() {
        if !workbench.navigator.creating {
            empty(ui, workbench);
        }
        return;
    }
    ui.add(widgets::field(&mut workbench.navigator.filter).hint_text("Search this page"));
    ui.add_space(theme::SPACE_XS);
    rows(ui, workbench, intents, busy);
    paging(ui, workbench, intents, busy);
    ui.add_space(theme::SPACE_SM);
    ui.horizontal_wrapped(|ui| {
        widgets::caption(
            ui,
            format!("{} in this workspace", workbench.navigator.total),
        );
        if ui
            .add_enabled(!busy, egui::Button::new("Refresh").small())
            .on_hover_text("Read the list again from the server")
            .clicked()
        {
            intents.push(Intent::ListWorkflows(workbench.navigator.page));
        }
    });
    ui.add_space(theme::SPACE_SM);
    widgets::caption(
        ui,
        "Unsaved drafts stay in this app until you close it, even after you sign out.",
    );
}

fn header(ui: &mut egui::Ui, workbench: &mut Workbench, busy: bool) {
    ui.horizontal(|ui| {
        widgets::section(ui, "Workflows");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("+ New"))
                .on_hover_text("Create a workflow")
                .clicked()
            {
                if workbench.navigator.creating {
                    workbench.navigator.creating = false;
                } else {
                    workbench.navigator.start_creating();
                }
            }
        });
    });
}

/// Inline creation: a name, then Create or Enter.
fn create_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let field = ui.add(
        widgets::field(&mut workbench.navigator.new_name).hint_text("Name of the new workflow"),
    );
    if std::mem::take(&mut workbench.navigator.focus_name) {
        field.request_focus();
    }
    let ready = !busy && !workbench.navigator.new_name.trim().is_empty();
    let submitted = widgets::submitted(ui, &field);
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
    ui.add_space(theme::SPACE_SM);
}

fn empty(ui: &mut egui::Ui, workbench: &mut Workbench) {
    ui.add_space(theme::SPACE_MD);
    widgets::caption(ui, "This workspace has no workflows yet.");
    if ui
        .add(widgets::primary_button("Create a workflow"))
        .clicked()
    {
        workbench.navigator.start_creating();
    }
}

/// One row per workflow: name on the left, a marker on the right when it has unsaved local edits.
fn rows(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let visible: Vec<(String, String)> = workbench
        .navigator
        .visible()
        .into_iter()
        .map(|workflow| (workflow.id.clone(), workflow.name.clone()))
        .collect();
    if visible.is_empty() {
        widgets::caption(ui, "No workflow on this page matches the search.");
    }
    for (id, name) in visible {
        let selected = workbench
            .session
            .selected
            .as_ref()
            .is_some_and(|key| key.workflow == id);
        let marker = if workbench.has_unsaved(&id) {
            RichText::new("Unsaved")
                .color(theme::WARNING)
                .size(theme::SIZE_SMALL)
        } else {
            RichText::new("")
        };
        let row = egui::Button::selectable(selected, name.as_str())
            .right_text(marker)
            .truncate()
            .min_size(egui::vec2(ui.available_width(), ROW_HEIGHT));
        if ui.add_enabled(!busy, row).on_hover_text(&name).clicked()
            && workbench.select_workflow(&id)
        {
            intents.push(Intent::LoadWorkflow(id));
        }
    }
}

fn paging(ui: &mut egui::Ui, workbench: &Workbench, intents: &mut Intents, busy: bool) {
    let navigator = &workbench.navigator;
    if !navigator.has_previous_page() && !navigator.has_next_page() {
        return;
    }
    let page = navigator.page;
    ui.add_space(theme::SPACE_XS);
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
