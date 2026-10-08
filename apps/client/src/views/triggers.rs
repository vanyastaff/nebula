//! What starts each workflow: the trigger bindings in its definition. A webhook trigger is
//! registered with the server, which answers with a public address and a signing secret that are
//! shown once.

use super::{Intent, Intents, states};
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::{Page, Workbench},
};
use eframe::egui::{self, RichText};
use nebula_api_contract::v1::workflow::WorkflowDocumentResponse;
use serde_json::{Value, json};

/// The webhook provider every engine build ships.
pub(crate) const WEBHOOK_PROVIDER: &str = "generic";

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.triggers.documents.wants_read() {
        intents.push(Intent::LoadTriggers);
    }
    states::page_header(
        ui,
        "Triggers",
        "What starts each workflow. A webhook trigger gets a public address and a signing secret.",
        |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh"))
                .on_hover_text("Read every workflow's triggers again")
                .clicked()
            {
                intents.push(Intent::LoadTriggers);
            }
        },
    );
    registered(ui, workbench);
    let documents = match states::show(ui, &workbench.triggers.documents, "triggers") {
        states::Shown::Ready(documents) => documents.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadTriggers);
            return;
        },
        states::Shown::Waiting => return,
    };
    if documents.is_empty() {
        if states::empty(
            ui,
            "No workflows",
            "Triggers belong to workflows. Create a workflow first.",
            Some("Open workflows"),
        ) {
            workbench.go(Page::Workflows);
        }
        return;
    }
    // Changes wait until the page shows the server's documents again, so a change whose answer was
    // lost is seen before it is repeated, and each edit starts from the stored revision.
    let locked = busy || !workbench.triggers.documents.settled();
    for document in &documents {
        workflow(ui, workbench, intents, document, locked);
        ui.add_space(theme::SPACE_SM);
    }
}

/// The address and secret of a webhook just registered. The secret is shown here once.
fn registered(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let Some(webhook) = &workbench.triggers.registered else {
        return;
    };
    let workflow = workbench
        .triggers
        .documents
        .value()
        .and_then(|documents| {
            documents
                .iter()
                .find(|document| document.workflow.id == webhook.workflow)
        })
        .map_or(webhook.workflow.as_str(), |document| {
            document.workflow.name.as_str()
        });
    let mut done = false;
    theme::card_block(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::badge(ui, "Webhook registered", Tone::Success);
            widgets::caption(ui, format!("{workflow} · trigger {}", webhook.trigger));
        });
        ui.add_space(theme::SPACE_SM);
        widgets::copyable(ui, "Address", &webhook.url);
        widgets::copyable(ui, "Signing secret", webhook.secret.as_str());
        widgets::banner(
            ui,
            Tone::Warning,
            "Copy the signing secret now. It is not stored in this app and is not shown again.",
        );
        ui.add_space(theme::SPACE_SM);
        done = ui.button("Done").clicked();
    });
    ui.add_space(theme::SPACE_LG);
    if done {
        workbench.triggers.registered = None;
    }
}

fn workflow(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    document: &WorkflowDocumentResponse,
    busy: bool,
) {
    let bindings: Vec<Value> = document.definition["trigger_bindings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let id = &document.workflow.id;
    theme::card_block(ui, |ui| {
        let mut open = false;
        widgets::row_with_actions(
            ui,
            |ui| {
                open = ui
                    .link(RichText::new(&document.workflow.name).strong())
                    .on_hover_text("Open this workflow in the editor")
                    .clicked();
            },
            |ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Add webhook trigger"))
                    .on_hover_text("Bind a webhook that starts this workflow")
                    .clicked()
                {
                    let mut next = bindings.clone();
                    next.push(json!({
                        "id": free_id(&bindings),
                        "plugin_key": "core",
                        "action_key": "webhook",
                        "config": {"provider": WEBHOOK_PROVIDER}
                    }));
                    intents.push(Intent::SaveTriggers(id.clone(), Value::Array(next)));
                }
            },
        );
        if open && workbench.select_workflow(id) {
            intents.push(Intent::LoadWorkflow(id.clone()));
        }
        if bindings.is_empty() {
            widgets::caption(
                ui,
                "No trigger: it starts only when someone runs it or a client starts it through the API.",
            );
            return;
        }
        for binding in &bindings {
            ui.add_space(theme::SPACE_XS);
            binding_row(ui, intents, id, binding, &bindings, busy);
        }
    });
}

fn binding_row(
    ui: &mut egui::Ui,
    intents: &mut Intents,
    workflow: &str,
    binding: &Value,
    bindings: &[Value],
    busy: bool,
) {
    let trigger = binding["id"].as_str().unwrap_or_default();
    let action = format!(
        "{}.{}",
        binding["plugin_key"].as_str().unwrap_or("core"),
        binding["action_key"].as_str().unwrap_or_default()
    );
    let webhook = binding["action_key"] == "webhook";
    widgets::row_with_actions(
        ui,
        |ui| {
            widgets::badge(
                ui,
                if webhook { "Webhook" } else { "Trigger" },
                Tone::Accent,
            );
            ui.label(RichText::new(trigger).monospace());
            widgets::caption(ui, &action);
            if let Some(provider) = binding["config"]["provider"].as_str() {
                widgets::caption(ui, format!("provider {provider}"));
            }
        },
        |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Remove"))
                .on_hover_text("Remove this trigger from the workflow")
                .clicked()
            {
                let next: Vec<Value> = bindings
                    .iter()
                    .filter(|other| other["id"] != binding["id"])
                    .cloned()
                    .collect();
                intents.push(Intent::SaveTriggers(
                    workflow.to_owned(),
                    Value::Array(next),
                ));
            }
            if webhook
                && ui
                    .add_enabled(!busy, egui::Button::new("Register"))
                    .on_hover_text("Get the webhook's address and signing secret")
                    .clicked()
            {
                intents.push(Intent::RegisterWebhook(
                    workflow.to_owned(),
                    trigger.to_owned(),
                ));
            }
        },
    );
}

/// A trigger id not taken yet: `webhook`, then `webhook_2`, and so on.
fn free_id(bindings: &[Value]) -> String {
    let taken = |candidate: &str| bindings.iter().any(|binding| binding["id"] == candidate);
    let mut suffix = 1;
    loop {
        let candidate = if suffix == 1 {
            "webhook".to_owned()
        } else {
            format!("webhook_{suffix}")
        };
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_trigger_takes_the_first_free_id() {
        assert_eq!(free_id(&[]), "webhook");
        assert_eq!(
            free_id(&[json!({"id": "webhook"}), json!({"id": "webhook_2"})]),
            "webhook_3"
        );
    }
}
