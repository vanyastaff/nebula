//! Sign-in, then workspace selection. Shown as one centered card while no workspace is open.
use super::{Intent, Intents};
use crate::{theme, widgets, workbench::Workbench};
use eframe::egui;

const FORM_WIDTH: f32 = 440.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    ui.add_space(theme::SPACE_XL);
    let inset = ((ui.available_width() - FORM_WIDTH) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(inset);
        ui.vertical(|ui| {
            ui.set_width(FORM_WIDTH.min(ui.available_width()));
            theme::card().show(ui, |ui| {
                if workbench.is_signed_in() {
                    workspace_form(ui, workbench, intents);
                } else {
                    sign_in_form(ui, workbench, intents);
                }
            });
        });
    });
}

fn sign_in_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::title(ui, "Sign in to Nebula");
    widgets::caption(
        ui,
        "Use an existing server. Passwords and tokens stay in memory and are cleared after sign-in.",
    );
    ui.add_space(theme::SPACE_MD);
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        widgets::labeled_field(ui, "Server address", &mut workbench.form.endpoint, false);
        ui.checkbox(&mut workbench.form.use_token, "Use a personal access token");
        if workbench.form.use_token {
            widgets::labeled_field(ui, "Personal access token", &mut workbench.form.token, true);
        } else {
            widgets::labeled_field(ui, "Email", &mut workbench.form.email, false);
            widgets::labeled_field(ui, "Password", &mut workbench.form.password, true);
            widgets::labeled_field(
                ui,
                "Authenticator code (optional)",
                &mut workbench.form.totp,
                true,
            );
        }
        ui.add_space(theme::SPACE_SM);
        if ui
            .add_sized(
                [ui.available_width(), 40.0],
                widgets::primary_button("Sign in"),
            )
            .clicked()
        {
            intents.push(Intent::SignIn);
        }
    });
}

fn workspace_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::title(ui, "Open a workspace");
    widgets::caption(
        ui,
        "Enter the organization and workspace slug or ID provided by your server.",
    );
    ui.add_space(theme::SPACE_MD);
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        widgets::labeled_field(ui, "Organization", &mut workbench.form.organization, false);
        widgets::labeled_field(ui, "Workspace", &mut workbench.form.workspace, false);
        ui.add_space(theme::SPACE_SM);
        // An empty slug would make every workspace request fail before it reaches the server.
        let ready = !workbench.form.organization.trim().is_empty()
            && !workbench.form.workspace.trim().is_empty();
        let open = widgets::primary_button("Open workspace")
            .min_size(egui::vec2(ui.available_width(), 40.0));
        if ui.add_enabled(ready, open).clicked() {
            intents.push(Intent::OpenWorkspace);
        }
        if workbench.workspace_open()
            && ui
                .add_sized(
                    [ui.available_width(), 34.0],
                    egui::Button::new("Back to workflows"),
                )
                .clicked()
        {
            workbench.workspace_form_open = false;
        }
    });
}
