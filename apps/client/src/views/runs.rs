//! Runs of the open workflow: start, reconcile a pending start, recent history and persisted status.
use super::{Intent, Intents};
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::{Workbench, draft_gate},
};
use eframe::egui;
use nebula_api_contract::v1::execution::{ExecutionNodeOutput, ExecutionStatus};

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    widgets::section(ui, "Runs");
    let Some(draft) = workbench.session.draft() else {
        widgets::caption(ui, "Open a workflow to run it and inspect its recent runs.");
        return;
    };
    let execution = draft.execution_id.clone();
    let can_run = draft_gate(draft).can_run;
    let pending = draft.start_key.is_some();
    widgets::caption(
        ui,
        "Runs the server's published version. Save and publish your changes first.",
    );
    ui.add_enabled_ui(!workbench.session.busy(), |ui| {
        ui.horizontal_wrapped(|ui| {
            let label = if pending {
                "Reconcile pending run"
            } else {
                "Run workflow"
            };
            if ui
                .add_enabled(can_run || pending, widgets::primary_button(label))
                .clicked()
            {
                intents.push(Intent::RunDraft);
            }
            if ui.button("Recent runs").clicked() {
                intents.push(Intent::LoadRecentRuns);
            }
            if let Some(id) = execution
                && ui.button("Refresh status").clicked()
            {
                intents.push(Intent::LoadExecution(id));
            }
        });
        history(ui, workbench, intents);
    });
    status(ui, workbench);
}

fn history(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(runs) = &workbench.history else {
        return;
    };
    let mut chosen = None;
    egui::CollapsingHeader::new("Recent runs")
        .default_open(workbench.status.is_none())
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("history")
                .max_height(120.0)
                .show(ui, |ui| {
                    for execution in &runs.items {
                        let started = execution
                            .created_at
                            .get(..19)
                            .unwrap_or(&execution.created_at)
                            .replace('T', " ");
                        if ui
                            .button(format!("{:?}    {started}", execution.status))
                            .clicked()
                        {
                            chosen = Some(execution.id.clone());
                        }
                    }
                });
        });
    if let Some(id) = chosen {
        if let Some(draft) = workbench.session.draft_mut() {
            draft.execution_id = Some(id.clone());
        }
        intents.push(Intent::LoadExecution(id));
    }
}

fn status(ui: &mut egui::Ui, workbench: &Workbench) {
    let Some(status) = &workbench.status else {
        return;
    };
    ui.add_space(theme::SPACE_SM);
    let tone = match status.execution.status {
        ExecutionStatus::Completed => Tone::Success,
        ExecutionStatus::Failed | ExecutionStatus::TimedOut => Tone::Danger,
        _ => Tone::Accent,
    };
    widgets::badge(ui, format!("{:?}", status.execution.status), tone);
    for (name, node) in &status.nodes {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(name).strong());
            widgets::caption(ui, format!("{:?}", node.status));
            if let Some(ExecutionNodeOutput::Inline { value }) = &node.output {
                ui.monospace(value.to_string());
            }
        });
    }
    ui.collapsing("Execution details", |ui| {
        widgets::caption(ui, &status.execution.id);
        widgets::caption(ui, format!("Server snapshot {}", status.snapshot_version));
        widgets::caption(ui, format!("Updated {}", status.execution.updated_at));
        widgets::caption(
            ui,
            format!(
                "Started {}",
                status
                    .execution
                    .started_at
                    .as_deref()
                    .unwrap_or("Not started")
            ),
        );
        widgets::caption(
            ui,
            format!(
                "Finished {}",
                status
                    .execution
                    .finished_at
                    .as_deref()
                    .unwrap_or("Not terminal")
            ),
        );
    });
}
