//! Signed-out welcome with the sign-in card, then the workspace choice. Both are centered pages.
use super::{Intent, Intents};
use crate::{
    theme, widgets,
    workbench::{SignInMode, Workbench},
};
use eframe::egui::{self, RichText};

/// Width of the single-card page: the workspace choice, and sign-in on narrow windows.
const CARD_WIDTH: f32 = 440.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    ui.add_space(theme::SPACE_XL);
    if workbench.is_signed_in() {
        widgets::page_column(ui, CARD_WIDTH, |ui| {
            theme::card_block(ui, |ui| workspace_form(ui, workbench, intents));
        });
    } else {
        widgets::page_column(ui, theme::PAGE_MAX_WIDTH, |ui| {
            welcome(ui, workbench, intents);
        });
    }
}

/// The introduction sits beside the sign-in card on wide windows and above it on narrow ones.
fn welcome(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if ui.available_width() >= theme::WIDE_LAYOUT_MIN {
        // A wider gap than the default keeps the introduction from running into the card.
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_XL;
            ui.columns(2, |columns| {
                introduction(&mut columns[0]);
                theme::card_block(&mut columns[1], |ui| sign_in_form(ui, workbench, intents));
            });
        });
    } else {
        introduction(ui);
        ui.add_space(theme::SPACE_XL);
        theme::card_block(ui, |ui| sign_in_form(ui, workbench, intents));
    }
}

fn introduction(ui: &mut egui::Ui) {
    widgets::headline(ui, "Design and run workflows on your Nebula server");
    ui.add_space(theme::SPACE_SM);
    widgets::caption(
        ui,
        "The workbench edits the workflows stored on a Nebula server. Sign in, choose a workspace, then work through its workflows.",
    );
    ui.add_space(theme::SPACE_LG);
    feature(
        ui,
        "Canvas",
        "Drag nodes, connect their ports and add actions.",
    );
    feature(
        ui,
        "Drafts",
        "Save changes, review a newer server version and reapply your edits.",
    );
    feature(
        ui,
        "Runs",
        "Publish a version, execute it and read each node's output.",
    );
}

fn feature(ui: &mut egui::Ui, heading: &str, body: &str) {
    ui.label(RichText::new(heading).strong());
    widgets::caption(ui, body);
    ui.add_space(theme::SPACE_MD);
}

fn sign_in_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::title(ui, "Sign in");
    widgets::caption(
        ui,
        "Use an account on this server or a personal access token. Secrets are cleared once sign-in completes.",
    );
    ui.add_space(theme::SPACE_MD);
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        // The address rarely changes, so it sits behind a collapsed header. While collapsed the address is
        // shown as a caption, which wraps; a header line would not, and would widen the card on phones.
        let server = egui::CollapsingHeader::new("Server")
            .id_salt("server-address")
            .show(ui, |ui| {
                widgets::labeled_field(ui, "Server address", &mut workbench.form.endpoint, false);
            });
        if server.body_returned.is_none() {
            let address = workbench.form.endpoint.trim();
            widgets::caption(
                ui,
                if address.is_empty() {
                    "No server address yet."
                } else {
                    address
                },
            );
        }
        ui.add_space(theme::SPACE_SM);
        let mut mode = workbench.form.mode;
        widgets::segmented(
            ui,
            &mut mode,
            &[
                (SignInMode::Password, "Password"),
                (SignInMode::Token, "Access token"),
            ],
        );
        workbench.form.set_mode(mode);
        ui.add_space(theme::SPACE_SM);
        match workbench.form.mode {
            SignInMode::Password => password_fields(ui, workbench),
            SignInMode::Token => {
                widgets::labeled_field(
                    ui,
                    "Personal access token",
                    &mut workbench.form.token,
                    true,
                );
            },
        }
        ui.add_space(theme::SPACE_SM);
        let sign_in =
            widgets::primary_button("Sign in").min_size(egui::vec2(ui.available_width(), 40.0));
        if ui
            .add_enabled(workbench.form.can_sign_in(), sign_in)
            .clicked()
        {
            intents.push(Intent::SignIn);
        }
    });
}

/// The code field appears only after the server has asked for a second factor.
fn password_fields(ui: &mut egui::Ui, workbench: &mut Workbench) {
    widgets::labeled_field(ui, "Email", &mut workbench.form.email, false);
    widgets::labeled_field(ui, "Password", &mut workbench.form.password, true);
    if workbench.form.mfa_required {
        widgets::labeled_field(ui, "Authenticator code", &mut workbench.form.totp, true);
        widgets::caption(
            ui,
            "This account requires a code from its authenticator app.",
        );
    }
}

/// Workspaces this app opened before, one click each. The server has no endpoint that lists them.
fn recent_workspaces(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if workbench.recent.is_empty() {
        return;
    }
    widgets::caption(ui, "Recent");
    let mut chosen = None;
    for recent in &workbench.recent {
        let row = egui::Button::new(
            RichText::new(format!("{} / {}", recent.organization, recent.workspace)).strong(),
        )
        .right_text(RichText::new("Open").color(theme::ACCENT))
        .truncate()
        .min_size(egui::vec2(ui.available_width(), 36.0));
        if ui.add(row).clicked() {
            chosen = Some(recent.clone());
        }
    }
    if let Some(recent) = chosen {
        workbench.form.organization = recent.organization;
        workbench.form.workspace = recent.workspace;
        intents.push(Intent::OpenWorkspace);
    }
    ui.add_space(theme::SPACE_SM);
    ui.separator();
}

fn workspace_form(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::title(ui, "Choose a workspace");
    if let Some(profile) = &workbench.profile {
        widgets::caption(ui, format!("Signed in as {}.", profile.email));
    }
    ui.add_space(theme::SPACE_MD);
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        recent_workspaces(ui, workbench, intents);
        widgets::caption(
            ui,
            "Enter the organization and workspace slug or ID provided by your server.",
        );
        widgets::labeled_field(ui, "Organization", &mut workbench.form.organization, false);
        widgets::labeled_field(ui, "Workspace", &mut workbench.form.workspace, false);
        ui.add_space(theme::SPACE_SM);
        let open = widgets::primary_button("Open workspace")
            .min_size(egui::vec2(ui.available_width(), 40.0));
        if ui
            .add_enabled(workbench.form.can_open_workspace(), open)
            .clicked()
        {
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
