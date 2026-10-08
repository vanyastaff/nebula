//! Header and feedback strip shared by every layout.
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, RichText};

pub(crate) fn header(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let workspace = workbench
        .session
        .context
        .as_ref()
        .map(|context| format!("{} / {}", context.organization, context.workspace_selector));
    // One wrapping row: brand, workspace and actions share the line on wide windows and wrap on
    // phones without horizontal scrolling.
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Nebula")
                .size(22.0)
                .strong()
                .color(theme::ACCENT),
        );
        widgets::caption(ui, "Workflow workbench");
        if let Some(workspace) = workspace {
            widgets::badge(ui, workspace, Tone::Neutral);
        }
        if workbench.is_signed_in() {
            if ui.button("Switch workspace").clicked() {
                workbench.workspace_form_open = !workbench.workspace_form_open;
            }
            if ui.button("Sign out").clicked() {
                workbench.disconnect();
            }
        }
    });
}

/// Latest outcome of the last action, plus a spinner while a request is in flight.
pub(crate) fn feedback(ui: &mut egui::Ui, workbench: &Workbench) {
    let tone = if workbench.feedback.failure {
        Tone::Danger
    } else {
        Tone::Neutral
    };
    ui.vertical(|ui| {
        if workbench.session.busy() {
            ui.spinner();
        }
        widgets::banner(ui, tone, &workbench.feedback.message);
    });
}
