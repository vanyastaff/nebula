//! Workflow list for the open workspace, with paging and creation.
use super::{Intent, Intents};
use crate::{theme, widgets, workbench::Workbench};
use eframe::egui::{self, RichText};

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::section(ui, "Workflows");
    widgets::caption(
        ui,
        format!("{} in this workspace", workbench.navigator.total),
    );
    if !workbench.workspace_open() {
        widgets::caption(ui, "Open a workspace to list its workflows.");
        return;
    }
    ui.add_space(theme::SPACE_XS);
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        paging(ui, workbench, intents);
        egui::CollapsingHeader::new("New workflow").show(ui, |ui| {
            widgets::labeled_field(
                ui,
                "Workflow name",
                &mut workbench.navigator.new_name,
                false,
            );
            let ready = !workbench.navigator.new_name.trim().is_empty();
            if ui
                .add_enabled(ready, widgets::primary_button("Create workflow"))
                .clicked()
            {
                intents.push(Intent::CreateWorkflow);
            }
        });
        ui.add_space(theme::SPACE_SM);
        for workflow in workbench.navigator.workflows.clone() {
            let selected = workbench
                .session
                .selected
                .as_ref()
                .is_some_and(|key| key.workflow == workflow.id);
            let entry =
                egui::Button::new(RichText::new(&workflow.name).strong()).selected(selected);
            if ui.add_sized([ui.available_width(), 36.0], entry).clicked()
                && workbench.select_workflow(&workflow.id)
            {
                intents.push(Intent::LoadWorkflow(workflow.id));
            }
        }
    });
    ui.separator();
    widgets::caption(
        ui,
        "Drafts stay here when you disconnect. Closing the app clears them.",
    );
}

fn paging(ui: &mut egui::Ui, workbench: &Workbench, intents: &mut Intents) {
    let page = workbench.navigator.page;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Refresh").clicked() {
            intents.push(Intent::ListWorkflows(page));
        }
        if ui
            .add_enabled(
                workbench.navigator.has_previous_page(),
                egui::Button::new("Previous"),
            )
            .clicked()
        {
            intents.push(Intent::ListWorkflows(page - 1));
        }
        if ui
            .add_enabled(
                workbench.navigator.has_next_page(),
                egui::Button::new("Next"),
            )
            .clicked()
        {
            intents.push(Intent::ListWorkflows(page + 1));
        }
    });
}
