//! Top bar and feedback strip shared by every layout.
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, Align, Layout, RichText};

/// Brand and location on the left, account on the right. Phones wrap the account below the brand, so
/// nothing scrolls sideways.
pub(crate) fn header(ui: &mut egui::Ui, workbench: &mut Workbench, wide: bool) {
    if wide {
        ui.horizontal(|ui| {
            location(ui, workbench, wide);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Nested so the account actions keep their reading order inside the right-aligned slot.
                ui.horizontal(|ui| account(ui, workbench));
            });
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            location(ui, workbench, wide);
            account(ui, workbench);
        });
    }
}

fn location(ui: &mut egui::Ui, workbench: &mut Workbench, wide: bool) {
    ui.label(
        RichText::new("Nebula")
            .size(22.0)
            .strong()
            .color(theme::ACCENT),
    );
    match &workbench.session.context {
        Some(context) => {
            let place = format!("{} / {}", context.organization, context.workspace_selector);
            ui.label(RichText::new(place).color(theme::TEXT).strong());
        },
        None => widgets::caption(ui, "Workflow workbench"),
    }
    // Phones have no room for the sidebar beside the page, so its list opens from here instead.
    if !wide && workbench.workspace_open() && !workbench.workspace_form_open {
        let open = egui::Button::new("Workflows").selected(workbench.sidebar_open);
        if ui.add(open).clicked() {
            workbench.sidebar_open = !workbench.sidebar_open;
        }
    }
}

fn account(ui: &mut egui::Ui, workbench: &mut Workbench) {
    if let Some(profile) = &workbench.profile {
        widgets::caption(ui, profile.email.as_str());
    }
    if workbench.workspace_open() && ui.button("Switch workspace").clicked() {
        workbench.workspace_form_open = !workbench.workspace_form_open;
    }
    if workbench.is_signed_in() && ui.button("Sign out").clicked() {
        workbench.disconnect();
    }
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
