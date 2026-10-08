//! Who belongs to the organization and who can work in this workspace, with their roles. The server
//! knows people by their user identity; names are not part of its API.

use super::{Intent, Intents, states};
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::{
        Workbench,
        pages::{ORG_ROLES, WORKSPACE_ROLES},
    },
};
use eframe::egui::{self, RichText};

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.team.organization.wants_read() {
        intents.push(Intent::LoadOrgMembers);
    }
    if workbench.team.workspace.wants_read() {
        intents.push(Intent::LoadWorkspaceMembers);
    }
    states::page_header(
        ui,
        "Team",
        "Members of the organization, and who can work in this workspace.",
        |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh"))
                .on_hover_text("Read both member lists again")
                .clicked()
            {
                intents.push(Intent::LoadOrgMembers);
                intents.push(Intent::LoadWorkspaceMembers);
            }
        },
    );
    theme::card_block(ui, |ui| workspace(ui, workbench, intents, busy));
    ui.add_space(theme::SPACE_MD);
    theme::card_block(ui, |ui| organization(ui, workbench, intents, busy));
    ui.add_space(theme::SPACE_MD);
    theme::card_block(ui, |ui| add_member(ui, workbench, intents, busy));
}

fn you(workbench: &Workbench, principal: &str) -> bool {
    workbench
        .profile
        .as_ref()
        .is_some_and(|profile| profile.user_id == principal)
}

fn identity(ui: &mut egui::Ui, you: bool, principal: &str) {
    ui.label(RichText::new(principal).monospace());
    if you {
        widgets::badge(ui, "You", Tone::Accent);
    }
}

/// Workspace access: each member's role can be changed in place.
fn workspace(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "This workspace");
    widgets::caption(
        ui,
        "Viewers read, runners also run, editors also change workflows, admins also manage access.",
    );
    ui.add_space(theme::SPACE_SM);
    let members = match states::show(ui, &workbench.team.workspace, "workspace members") {
        states::Shown::Ready(members) => members.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadWorkspaceMembers);
            return;
        },
        states::Shown::Waiting => return,
    };
    if members.is_empty() {
        widgets::caption(ui, "Nobody has explicit access to this workspace.");
    }
    for member in &members {
        let you = you(workbench, &member.principal_id);
        widgets::row_with_actions(
            ui,
            |ui| identity(ui, you, &member.principal_id),
            |ui| {
                remove_button(ui, workbench, intents, &member.principal_id, busy, true);
                let mut role = member.role.0.clone();
                egui::ComboBox::from_id_salt(("workspace-role", &member.principal_id))
                    .selected_text(&role)
                    .show_ui(ui, |ui| {
                        for candidate in WORKSPACE_ROLES {
                            ui.selectable_value(&mut role, candidate.to_owned(), candidate);
                        }
                    });
                if role != member.role.0 && !busy {
                    intents.push(Intent::SetWorkspaceMember(
                        member.principal_id.clone(),
                        role,
                    ));
                }
            },
        );
        ui.separator();
    }
}

fn organization(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "Organization");
    ui.add_space(theme::SPACE_SM);
    let members = match states::show(ui, &workbench.team.organization, "organization members") {
        states::Shown::Ready(members) => members.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadOrgMembers);
            return;
        },
        states::Shown::Waiting => return,
    };
    for member in &members {
        let you = you(workbench, &member.principal_id);
        widgets::row_with_actions(
            ui,
            |ui| identity(ui, you, &member.principal_id),
            |ui| {
                remove_button(ui, workbench, intents, &member.principal_id, busy, false);
                let tone = if member.role.0 == "owner" {
                    Tone::Accent
                } else {
                    Tone::Neutral
                };
                widgets::badge(ui, &member.role.0, tone);
            },
        );
        ui.separator();
    }
}

/// Remove, then a confirmation in its place. Removing yourself is left to someone else.
fn remove_button(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    principal: &str,
    busy: bool,
    workspace: bool,
) {
    if you(workbench, principal) {
        return;
    }
    let key = format!("{}:{principal}", if workspace { "ws" } else { "org" });
    if workbench.team.confirm_remove.as_deref() == Some(key.as_str()) {
        if ui.button("Keep").clicked() {
            workbench.team.confirm_remove = None;
        }
        if ui
            .add_enabled(!busy, widgets::danger_button("Remove"))
            .clicked()
        {
            intents.push(if workspace {
                Intent::RemoveWorkspaceMember(principal.to_owned())
            } else {
                Intent::RemoveOrgMember(principal.to_owned())
            });
        }
    } else if ui
        .add_enabled(!busy, egui::Button::new("Remove"))
        .on_hover_text(if workspace {
            "Take away access to this workspace"
        } else {
            "Remove from the organization"
        })
        .clicked()
    {
        workbench.team.confirm_remove = Some(key);
    }
}

fn add_member(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    widgets::section(ui, "Add a member");
    widgets::caption(
        ui,
        "Enter the person's user identity (usr_…), shown on their own Settings page.",
    );
    let team = &mut workbench.team;
    if team.new_role.is_empty() {
        "member".clone_into(&mut team.new_role);
    }
    let field = ui.add(widgets::field(&mut team.new_member).hint_text("usr_…"));
    ui.horizontal(|ui| {
        widgets::caption(ui, "Organization role");
        egui::ComboBox::from_id_salt("new-member-role")
            .selected_text(&team.new_role)
            .show_ui(ui, |ui| {
                for role in ORG_ROLES {
                    ui.selectable_value(&mut team.new_role, role.to_owned(), role);
                }
            });
    });
    let ready = !busy && !team.new_member.trim().is_empty();
    if ui
        .add_enabled(ready, widgets::primary_button("Add to organization"))
        .clicked()
        || (ready && widgets::submitted(ui, &field))
    {
        intents.push(Intent::AddOrgMember);
    }
    ui.add_space(theme::SPACE_SM);
    if team.new_workspace_role.is_empty() {
        "viewer".clone_into(&mut team.new_workspace_role);
    }
    ui.horizontal_wrapped(|ui| {
        widgets::caption(ui, "Workspace role");
        egui::ComboBox::from_id_salt("new-member-workspace-role")
            .selected_text(&team.new_workspace_role)
            .show_ui(ui, |ui| {
                for role in WORKSPACE_ROLES {
                    ui.selectable_value(&mut team.new_workspace_role, role.to_owned(), role);
                }
            });
        if ui
            .add_enabled(ready, egui::Button::new("Give workspace access"))
            .on_hover_text("Let this person work in this workspace with the chosen role")
            .clicked()
        {
            intents.push(Intent::SetWorkspaceMember(
                team.new_member.trim().to_owned(),
                team.new_workspace_role.clone(),
            ));
        }
    });
}
