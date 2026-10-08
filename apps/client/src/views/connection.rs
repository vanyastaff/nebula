//! Signed-out welcome with the sign-in card, then the workspace choice. Both are centered pages.
use super::{Intent, Intents};
use crate::{
    theme, widgets,
    workbench::{SignInMode, Workbench},
};
use eframe::egui::{self, RichText};

/// Width of the single-card page: the workspace choice, and sign-in on narrow windows.
const CARD_WIDTH: f32 = 440.0;

/// Rough height of the welcome and workspace pages, to centre them vertically on tall windows. It is a
/// little generous, which lifts the page slightly above the true centre, where it reads as centred.
const PAGE_HEIGHT: f32 = 520.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    // Centred on tall windows instead of hugging the top; a scrolling page has no height to share.
    let spare = ui.available_height() - PAGE_HEIGHT;
    ui.add_space(if spare.is_finite() {
        (spare / 2.0).max(theme::SPACE_XL)
    } else {
        theme::SPACE_XL
    });
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
        "The workbench edits the workflows stored on a Nebula server. Sign in and choose a workspace, or explore the demo workspace without a server.",
    );
    ui.add_space(theme::SPACE_LG);
    feature(
        ui,
        theme::ACCENT,
        "Canvas",
        "Drag nodes, connect their ports and add actions.",
    );
    feature(
        ui,
        theme::WARNING,
        "Drafts",
        "Save changes, review a newer server version and reapply your edits.",
    );
    feature(
        ui,
        theme::SUCCESS,
        "Runs",
        "Publish a version, execute it and read each node's output.",
    );
}

/// One feature with a badge in the shape the canvas gives nodes, so the page previews the product.
fn feature(ui: &mut egui::Ui, color: egui::Color32, heading: &str, body: &str) {
    ui.horizontal_top(|ui| {
        widgets::mark(ui, &widgets::initial(heading), color, 32.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::SPACE_XS;
            ui.label(RichText::new(heading).strong());
            widgets::caption(ui, body);
        });
    });
    ui.add_space(theme::SPACE_SM);
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
        let entered = match workbench.form.mode {
            SignInMode::Password => password_fields(ui, workbench),
            SignInMode::Token => {
                let token = widgets::labeled_field(
                    ui,
                    "Personal access token",
                    &mut workbench.form.token,
                    true,
                );
                focus_once(ui, &token, "sign-in-token", workbench.form.token.is_empty());
                widgets::submitted(ui, &token)
            },
        };
        ui.add_space(theme::SPACE_SM);
        let sign_in =
            widgets::primary_button("Sign in").min_size(egui::vec2(ui.available_width(), 40.0));
        let ready = workbench.form.can_sign_in();
        if ui.add_enabled(ready, sign_in).clicked() || (entered && ready) {
            intents.push(Intent::SignIn);
        }
        ui.add_space(theme::SPACE_MD);
        widgets::divider_label(ui, "or");
        ui.add_space(theme::SPACE_SM);
        if ui
            .add(egui::Button::new("Explore the demo workspace").min_size(egui::vec2(
                ui.available_width(),
                36.0,
            )))
            .on_hover_text("Sample workflows, runs, credentials and team, simulated in this app")
            .clicked()
        {
            intents.push(Intent::OpenDemo);
        }
        widgets::caption(
            ui,
            "No server needed: the demo runs workflows on a simulated executor and resets when you sign out.",
        );
    });
}

/// Puts the cursor in `field` the first time its stage is shown, when it still needs input. Once per
/// stage, so clicking elsewhere afterwards is not undone.
fn focus_once(ui: &egui::Ui, field: &egui::Response, stage: &str, needed: bool) {
    let id = egui::Id::new(("focus-once", stage));
    let first = ui.ctx().data_mut(|data| {
        let seen = data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, true);
        !seen
    });
    if first && needed {
        field.request_focus();
    }
}

/// Email and password, then the code field once the server has asked for a second factor. Returns true
/// when Enter was pressed in one of them.
fn password_fields(ui: &mut egui::Ui, workbench: &mut Workbench) -> bool {
    let form = &mut workbench.form;
    let email = widgets::labeled_field(ui, "Email", &mut form.email, false);
    focus_once(ui, &email, "sign-in-email", form.email.is_empty());
    let password = widgets::labeled_field(ui, "Password", &mut form.password, true);
    // A remembered email leaves the password as the first field to fill.
    focus_once(
        ui,
        &password,
        "sign-in-password",
        !form.email.is_empty() && form.password.is_empty(),
    );
    let mut entered = widgets::submitted(ui, &email) || widgets::submitted(ui, &password);
    if form.mfa_required {
        let code = widgets::labeled_field(ui, "Authenticator code", &mut form.totp, true);
        focus_once(ui, &code, "sign-in-code", form.totp.is_empty());
        entered |= widgets::submitted(ui, &code);
        widgets::caption(
            ui,
            "This account requires a code from its authenticator app.",
        );
    }
    entered
}

/// Workspaces this app opened before, one click each. The server has no endpoint that lists them.
fn recent_workspaces(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let recent = workbench.recent_here();
    if recent.is_empty() {
        return;
    }
    widgets::caption(ui, "Recent");
    let mut chosen = None;
    for recent in &recent {
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
    if let Some(chosen) = chosen {
        workbench.form.organization = chosen.organization;
        workbench.form.workspace = chosen.workspace;
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
        let organization =
            widgets::labeled_field(ui, "Organization", &mut workbench.form.organization, false);
        focus_once(
            ui,
            &organization,
            "workspace-organization",
            workbench.recent_here().is_empty() && workbench.form.organization.is_empty(),
        );
        let workspace =
            widgets::labeled_field(ui, "Workspace", &mut workbench.form.workspace, false);
        let entered = widgets::submitted(ui, &organization) || widgets::submitted(ui, &workspace);
        ui.add_space(theme::SPACE_SM);
        let open = widgets::primary_button("Open workspace")
            .min_size(egui::vec2(ui.available_width(), 40.0));
        let ready = workbench.form.can_open_workspace();
        if ui.add_enabled(ready, open).clicked() || (entered && ready) {
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
