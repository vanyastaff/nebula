//! The signed-in user's settings: their profile, the server or demo they are connected to, and their
//! personal access tokens.

use super::{Intent, Intents, states};
use crate::{
    api::{Backend, FULL_ACCESS, TOKEN_SCOPES},
    clock, theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, RichText};

/// Token lifetimes on offer, in days.
const LIFETIMES: [u32; 4] = [7, 30, 90, 365];

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.settings.profile.wants_read() {
        intents.push(Intent::LoadProfile);
    }
    if workbench.settings.tokens.wants_read() {
        intents.push(Intent::LoadTokens);
    }
    states::page_header(
        ui,
        "Settings",
        "Your profile, this connection and your personal access tokens.",
        |ui| {
            if ui
                .button("Keyboard shortcuts")
                .on_hover_text("Show every shortcut (?)")
                .clicked()
            {
                workbench.shortcuts_open = true;
            }
        },
    );
    theme::card_block(ui, |ui| profile(ui, workbench, intents, busy));
    ui.add_space(theme::SPACE_MD);
    theme::card_block(ui, |ui| connection(ui, workbench));
    ui.add_space(theme::SPACE_MD);
    theme::card_block(ui, |ui| tokens(ui, workbench, intents, busy));
}

fn profile(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "Profile");
    let profile = match states::show(ui, &workbench.settings.profile, "profile") {
        states::Shown::Ready(profile) => profile.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadProfile);
            return;
        },
        states::Shown::Waiting => return,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&profile.email).strong());
        if profile.email_verified {
            widgets::badge(ui, "Email verified", Tone::Success);
        } else {
            widgets::badge(ui, "Email not verified", Tone::Warning);
        }
        if profile.mfa_enabled {
            widgets::badge(ui, "Two-factor on", Tone::Success);
        } else {
            widgets::badge(ui, "Two-factor off", Tone::Neutral);
        }
    });
    widgets::copyable(ui, "User identity", &profile.user_id);
    ui.add_space(theme::SPACE_SM);
    let name = widgets::labeled_field(
        ui,
        "Display name",
        &mut workbench.settings.display_name,
        false,
    );
    let typed = workbench.settings.display_name.trim();
    let ready = !busy && !typed.is_empty() && typed != profile.display_name;
    ui.add_space(theme::SPACE_SM);
    let save = ui
        .add_enabled(ready, egui::Button::new("Save name"))
        .clicked();
    if save || (ready && widgets::submitted(ui, &name)) {
        intents.push(Intent::SaveProfile);
    }
}

fn connection(ui: &mut egui::Ui, workbench: &mut Workbench) {
    widgets::section(ui, "Connection");
    let (server, demo) = match &workbench.backend {
        Some(Backend::Server(connection)) => (connection.endpoint().to_owned(), false),
        Some(Backend::Demo(_)) => ("Demo workspace, simulated in this app".to_owned(), true),
        None => (String::new(), false),
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&server).monospace());
        if demo {
            widgets::badge(ui, "Demo", Tone::Accent);
        }
    });
    if let Some(context) = &workbench.session.context {
        widgets::caption(
            ui,
            format!(
                "Workspace {} / {}",
                context.organization, context.workspace_selector
            ),
        );
    }
    if demo {
        widgets::caption(
            ui,
            "Runs play on a simulated executor. Signing out discards the demo's changes.",
        );
    }
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        if !demo && ui.button("Switch workspace").clicked() {
            workbench.workspace_form_open = true;
        }
        if ui.add(widgets::danger_button("Sign out")).clicked() {
            workbench.disconnect();
        }
    });
}

fn tokens(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "Personal access tokens");
    widgets::caption(
        ui,
        "A token signs a script or tool in as you, with only the scopes you give it.",
    );
    ui.add_space(theme::SPACE_SM);
    if let Some((name, token)) = &workbench.settings.revealed {
        let mut done = false;
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::SUCCESS))
            .corner_radius(theme::RADIUS_MD)
            .inner_margin(egui::Margin::same(theme::SPACE_MD as i8))
            .show(ui, |ui| {
                widgets::badge(
                    ui,
                    format!("Token \u{201c}{name}\u{201d} created"),
                    Tone::Success,
                );
                widgets::copyable(ui, "Token", token.as_str());
                widgets::caption(ui, "Copy it now. It is not shown again.");
                done = ui.button("Done").clicked();
            });
        if done {
            workbench.settings.revealed = None;
        }
        ui.add_space(theme::SPACE_MD);
    }
    match states::show(ui, &workbench.settings.tokens, "tokens") {
        states::Shown::Ready(tokens) => {
            let tokens = tokens.clone();
            if tokens.is_empty() {
                widgets::caption(ui, "No tokens yet.");
            }
            let now = clock::now_millis();
            for token in &tokens {
                widgets::row_with_actions(
                    ui,
                    |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&token.name).strong());
                            let created = clock::parse_rfc3339(&token.created_at).unwrap_or(now);
                            let used = token
                                .last_used_at
                                .as_deref()
                                .and_then(clock::parse_rfc3339)
                                .map_or_else(
                                    || "never used".to_owned(),
                                    |at| format!("used {}", clock::ago(now, at)),
                                );
                            widgets::caption(
                                ui,
                                format!(
                                    "{} · created {} · {used}",
                                    token.scopes.join(", "),
                                    clock::ago(now, created)
                                ),
                            );
                        });
                    },
                    |ui| {
                        let confirming =
                            workbench.settings.confirm_revoke.as_deref() == Some(token.id.as_str());
                        if confirming {
                            if ui.button("Keep").clicked() {
                                workbench.settings.confirm_revoke = None;
                            }
                            if ui
                                .add_enabled(!busy, widgets::danger_button("Revoke"))
                                .clicked()
                            {
                                intents.push(Intent::RevokeToken(token.id.clone()));
                            }
                        } else if ui
                            .add_enabled(!busy, egui::Button::new("Revoke"))
                            .on_hover_text("Stop this token from signing in")
                            .clicked()
                        {
                            workbench.settings.confirm_revoke = Some(token.id.clone());
                        }
                    },
                );
                ui.separator();
            }
        },
        states::Shown::Retry => intents.push(Intent::LoadTokens),
        states::Shown::Waiting => {},
    }
    ui.add_space(theme::SPACE_SM);
    new_token(ui, workbench, intents, busy);
}

fn new_token(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::caption(ui, "New token");
    // Creating waits until the list shows the server's tokens, so a token whose creation answer
    // was lost is seen before another is made.
    let settled = workbench.settings.tokens.settled();
    let form = &mut workbench.settings.new_token;
    let name =
        ui.add(widgets::field(&mut form.name).hint_text("What will use it, such as CI deploy"));
    let mut full = form.scopes.contains(FULL_ACCESS);
    if ui
        .checkbox(&mut full, "Full access")
        .on_hover_text("Everything your account may do; it replaces every other scope")
        .changed()
    {
        form.scopes.clear();
        if full {
            form.scopes.insert(FULL_ACCESS.to_owned());
        }
    }
    ui.add_enabled_ui(!full, |ui| {
        for (group, scopes) in TOKEN_SCOPES {
            ui.horizontal_wrapped(|ui| {
                widgets::caption(ui, group);
                for scope in scopes {
                    let mut on = form.scopes.contains(*scope);
                    let action = scope.split_once(':').map_or(*scope, |(_, action)| action);
                    if ui.checkbox(&mut on, action).on_hover_text(*scope).changed() {
                        if on {
                            form.scopes.insert((*scope).to_owned());
                        } else {
                            form.scopes.remove(*scope);
                        }
                    }
                }
            });
        }
    });
    ui.horizontal(|ui| {
        widgets::caption(ui, "Expires after");
        egui::ComboBox::from_id_salt("token-lifetime")
            .selected_text(format!("{} days", form.ttl_days))
            .show_ui(ui, |ui| {
                for days in LIFETIMES {
                    ui.selectable_value(&mut form.ttl_days, days, format!("{days} days"));
                }
            });
    });
    let ready = !busy && settled && !form.name.trim().is_empty() && !form.scopes.is_empty();
    if ui
        .add_enabled(ready, widgets::primary_button("Create token"))
        .on_disabled_hover_text(if settled {
            "Name the token and give it at least one scope"
        } else {
            "Waiting for the token list to be read"
        })
        .clicked()
        || (ready && widgets::submitted(ui, &name))
    {
        intents.push(Intent::CreateToken);
    }
}
