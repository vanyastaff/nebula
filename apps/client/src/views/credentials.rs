//! Credentials of the workspace: the secrets workflows use to reach other services. Each shows its
//! type, lifecycle and last test; a new one is created from its type's own form. Stored values are
//! never shown again.

use super::{Intent, Intents, form, states};
use crate::{
    clock,
    document::{expression, literal},
    schema::Form,
    theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, Align, Layout, RichText};
use nebula_api_contract::v1::credential::{
    CredentialLifecycleState, CredentialSummary, CredentialTypeInfo, TestCredentialResponse,
};

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.credentials.list.wants_read() {
        intents.push(Intent::LoadCredentials);
    }
    if workbench.credentials.types.wants_read() {
        intents.push(Intent::LoadCredentialTypes);
    }
    states::page_header(
        ui,
        "Credentials",
        "Secrets your workflows use to reach other services. Stored values are never shown again.",
        |ui| {
            // Creating waits until the list shows the server's credentials, so one whose
            // creation answer was lost is seen before another is made.
            let settled = workbench.credentials.list.settled();
            if workbench.credentials.draft.is_none()
                && ui
                    .add_enabled(!busy && settled, widgets::primary_button("New credential"))
                    .on_disabled_hover_text("Waiting for the credential list to be read")
                    .clicked()
            {
                workbench.start_credential(Some(String::new()));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh"))
                .on_hover_text("Read the credentials again")
                .clicked()
            {
                intents.push(Intent::LoadCredentials);
            }
        },
    );
    let types = workbench
        .credentials
        .types
        .value()
        .cloned()
        .unwrap_or_default();
    if workbench.credentials.draft.is_some() {
        theme::card_block(ui, |ui| {
            new_credential(ui, workbench, intents, &types, busy);
        });
        ui.add_space(theme::SPACE_LG);
    }
    let credentials = match states::show(ui, &workbench.credentials.list, "credentials") {
        states::Shown::Ready(list) => list.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadCredentials);
            return;
        },
        states::Shown::Waiting => return,
    };
    if credentials.is_empty() {
        if workbench.credentials.draft.is_none()
            && states::empty(
                ui,
                "No credentials yet",
                "Add an API key, a password or an OAuth2 grant once, then use it from any workflow.",
                Some("New credential"),
            )
        {
            workbench.start_credential(Some(String::new()));
        }
        return;
    }
    for credential in &credentials {
        row(ui, workbench, intents, credential, &types, busy);
        ui.add_space(theme::SPACE_XS);
    }
}

fn row(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    credential: &CredentialSummary,
    types: &[CredentialTypeInfo],
    busy: bool,
) {
    let kind = types
        .iter()
        .find(|kind| kind.key == credential.credential_key);
    let testable = kind.is_none_or(|kind| kind.capabilities.testable);
    let now = clock::now_millis();
    theme::card_block(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::mark(
                ui,
                &widgets::initial(&credential.name),
                theme::node_accent(&credential.credential_key),
                32.0,
            );
            ui.vertical(|ui| {
                ui.label(RichText::new(&credential.name).strong());
                let type_name = kind.map_or(credential.credential_key.as_str(), |kind| &kind.name);
                let mut facts = vec![
                    type_name.to_owned(),
                    format!("version {}", credential.version),
                ];
                if let Some(expires) = credential
                    .expires_at
                    .as_deref()
                    .and_then(clock::parse_rfc3339)
                {
                    facts.push(if expires <= now {
                        "expired".to_owned()
                    } else {
                        format!(
                            "expires {}",
                            readable_date(credential.expires_at.as_deref())
                        )
                    });
                }
                widgets::caption(ui, facts.join(" · "));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let confirming =
                    workbench.credentials.confirm_delete.as_deref() == Some(credential.id.as_str());
                if confirming {
                    if ui.button("Keep").clicked() {
                        workbench.credentials.confirm_delete = None;
                    }
                    if ui
                        .add_enabled(!busy, widgets::danger_button("Delete"))
                        .clicked()
                    {
                        intents.push(Intent::DeleteCredential(credential.id.clone()));
                    }
                    widgets::caption(ui, "Delete it? Workflows using it stop working.");
                } else {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Delete"))
                        .on_hover_text("Delete this credential")
                        .clicked()
                    {
                        workbench.credentials.confirm_delete = Some(credential.id.clone());
                    }
                    if testable
                        && ui
                            .add_enabled(!busy, egui::Button::new("Test"))
                            .on_hover_text("Ask the provider whether it accepts this credential")
                            .clicked()
                    {
                        intents.push(Intent::TestCredential(credential.id.clone()));
                    }
                    if let Some(test) = workbench.credentials.tests.get(&credential.id) {
                        match test {
                            TestCredentialResponse::Success { .. } => {
                                widgets::badge(ui, "Test passed", Tone::Success);
                            },
                            TestCredentialResponse::Failed { .. } => {
                                widgets::badge(ui, "Test failed", Tone::Danger);
                            },
                        }
                    }
                    let (label, tone) = lifecycle(&credential.lifecycle);
                    widgets::badge(ui, label, tone);
                }
            });
        });
    });
}

/// `2026-11-08T12:00:00Z` as `2026-11-08`.
fn readable_date(rfc3339: Option<&str>) -> String {
    rfc3339
        .and_then(|text| text.get(..10))
        .unwrap_or_default()
        .to_owned()
}

/// A credential's lifecycle in words, coloured by whether it needs attention.
const fn lifecycle(state: &CredentialLifecycleState) -> (&'static str, Tone) {
    match state {
        CredentialLifecycleState::Ready => ("Ready", Tone::Success),
        CredentialLifecycleState::RefreshDeferred { .. } => ("Refresh deferred", Tone::Warning),
        CredentialLifecycleState::RefreshBlocked => ("Refresh blocked", Tone::Danger),
        CredentialLifecycleState::ReauthRequired => ("Reauthorize", Tone::Warning),
        CredentialLifecycleState::OperationInFlight { .. } => ("Updating", Tone::Accent),
        CredentialLifecycleState::ReconciliationRequired { .. } => {
            ("Needs reconciliation", Tone::Danger)
        },
    }
}

/// Creation: first the type, then a name and the type's own form.
fn new_credential(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    types: &[CredentialTypeInfo],
    busy: bool,
) {
    ui.horizontal(|ui| {
        widgets::section(ui, "New credential");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                workbench.start_credential(None);
            }
        });
    });
    let chosen = workbench
        .credentials
        .draft
        .as_ref()
        .map(|draft| draft.kind.clone())
        .unwrap_or_default();
    let Some(kind) = types.iter().find(|kind| kind.key == chosen) else {
        choose_type(ui, workbench, types);
        return;
    };
    ui.horizontal_wrapped(|ui| {
        widgets::badge(ui, &kind.name, Tone::Accent);
        if ui.small_button("Change type").clicked() {
            workbench.start_credential(Some(String::new()));
        }
    });
    widgets::caption(ui, &kind.description);
    ui.add_space(theme::SPACE_SM);
    let schema = Form::from_json_schema(&kind.schema);
    let Some(draft) = workbench.credentials.draft.as_mut() else {
        return;
    };
    let name = widgets::labeled_field(ui, "Name", &mut draft.name, false);
    // The name takes the cursor once, when the type's form first shows.
    let focused_once = egui::Id::new(("credential-name-focused", &kind.key));
    if !ui.data(|data| data.get_temp::<bool>(focused_once).unwrap_or(false)) {
        ui.data_mut(|data| data.insert_temp(focused_once, true));
        name.request_focus();
    }
    ui.add_space(theme::SPACE_MD);
    let edits = form::show(
        ui,
        &schema,
        &draft.entries,
        egui::Id::new(("credential-form", &kind.key)),
    );
    // A credential form edits a local draft; its typing needs no undo history.
    form::take_typing_session(ui.ctx());
    for edit in edits {
        match edit {
            form::FormEdit::Literal(field, value) => {
                draft.entries.insert(field, literal(value));
            },
            form::FormEdit::Expression(field, text) => {
                draft.entries.insert(field, expression(&text));
            },
            form::FormEdit::Clear(field) => {
                draft.entries.remove(&field);
            },
        }
    }
    if let Some(docs) = &kind.documentation_url {
        widgets::caption(ui, format!("Provider documentation: {docs}"));
    }
    ui.add_space(theme::SPACE_SM);
    let ready = !busy && !draft.name.trim().is_empty();
    if ui
        .add_enabled(ready, widgets::primary_button("Save credential"))
        .on_disabled_hover_text("Name the credential first")
        .clicked()
    {
        intents.push(Intent::CreateCredential);
    }
}

/// The credential types as cards; choosing one opens its form.
fn choose_type(ui: &mut egui::Ui, workbench: &mut Workbench, types: &[CredentialTypeInfo]) {
    match &workbench.credentials.types {
        crate::workbench::Remote::Failed(reason) => {
            widgets::banner(
                ui,
                Tone::Danger,
                &format!("The credential types could not be read. {reason}"),
            );
            return;
        },
        remote if remote.value().is_none() => {
            states::loading(ui, "Reading credential types…");
            return;
        },
        _ => {},
    }
    widgets::caption(ui, "Choose what kind of secret this is.");
    ui.add_space(theme::SPACE_SM);
    let mut picked = None;
    for kind in types {
        let card = egui::Button::new(RichText::new(&kind.name).strong())
            .right_text(RichText::new(auth_pattern(&kind.auth_pattern)).color(theme::TEXT_MUTED))
            .min_size(egui::vec2(ui.available_width(), 40.0));
        if ui.add(card).on_hover_text(&kind.description).clicked() {
            picked = Some(kind.key.clone());
        }
    }
    if let Some(key) = picked {
        workbench.start_credential(Some(key));
    }
}

/// How a credential type authenticates, in words; the server names it by its `AuthPattern`.
fn auth_pattern(pattern: &str) -> &str {
    match pattern {
        "NoAuth" => "No authentication",
        "SecretToken" => "API key or token",
        "IdentityPassword" => "Username and password",
        "OAuth2" => "OAuth 2.0 sign-in",
        "KeyPair" => "Key pair",
        "Certificate" => "Client certificate",
        "RequestSigning" => "Signed requests",
        "ConnectionUri" => "Connection URI",
        "InstanceIdentity" => "Cloud instance identity",
        "SharedSecret" => "Pre-shared key",
        "Custom" => "Plugin-defined",
        other => other,
    }
}
