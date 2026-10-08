//! Runs of the open workflow, read from what the server persisted: recent runs, and the nodes of the
//! chosen one with their outputs. The chosen run's states stream in while it runs, through the
//! app's execution watch. Starting a run lives on the canvas.
use super::{Intent, Intents, status as words};
use crate::{theme, widgets, workbench::Workbench};
use eframe::egui::{self, Align, Layout, RichText};
use nebula_api_contract::v1::execution::ExecutionNodeOutput;

/// Panels at least this wide show the chosen run beside the list instead of under it.
const SIDE_BY_SIDE_MIN: f32 = 720.0;
/// Longest node output shown inline; the rest is on the executions page.
const OUTPUT_PREVIEW: usize = 160;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    if workbench.session.draft().is_none() {
        return;
    }
    let busy = workbench.session.busy();
    // The list, with the chosen run, loads once per opened workflow and again after a start, without
    // a click. The app marks it requested when the read starts (`Workbench::recent_runs_requested`),
    // so an intent dropped behind another request is asked again next frame.
    if !workbench.history_requested && !busy {
        intents.push(Intent::RefreshRuns);
    }
    ui.horizontal(|ui| {
        widgets::section(ui, "Runs");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh").small())
                .on_hover_text("Read recent runs and the chosen run again")
                .clicked()
            {
                intents.push(Intent::RefreshRuns);
            }
        });
    });
    if ui.available_width() >= SIDE_BY_SIDE_MIN {
        ui.columns(2, |columns| {
            history(&mut columns[0], workbench, intents, busy);
            status(&mut columns[1], workbench);
        });
    } else {
        history(ui, workbench, intents, busy);
        ui.add_space(theme::SPACE_SM);
        status(ui, workbench);
    }
}

/// `2026-10-08T12:34:56.123456Z` as `2026-10-08 12:34:56`, the precision a person reads.
fn readable_time(rfc3339: &str) -> String {
    rfc3339.get(..19).unwrap_or(rfc3339).replace('T', " ")
}

fn history(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    if let Some(reason) = &workbench.history_error {
        let shown = if workbench.history.is_some() {
            "The list below is from an earlier read"
        } else {
            "Use Refresh to try again"
        };
        widgets::caption(
            ui,
            format!("Recent runs could not be read: {reason} {shown}."),
        );
    }
    let Some(runs) = &workbench.history else {
        if workbench.history_error.is_none() {
            widgets::caption(ui, "Reading recent runs…");
        }
        return;
    };
    if runs.items.is_empty() {
        widgets::caption(
            ui,
            "No runs yet. Publish the workflow, then use Execute workflow.",
        );
        return;
    }
    let current = workbench
        .session
        .draft()
        .and_then(|draft| draft.execution_id.clone());
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("history")
        .max_height(220.0)
        .show(ui, |ui| {
            for execution in &runs.items {
                let selected = current.as_deref() == Some(execution.id.as_str());
                let (color, _) = words::tone(execution.status).colors();
                // Status and time are what a person scans for; the id is in the run's details.
                let row = egui::Button::selectable(
                    selected,
                    RichText::new(words::label(execution.status)).color(color),
                )
                .right_text(
                    RichText::new(readable_time(&execution.created_at)).color(theme::TEXT_MUTED),
                )
                .min_size(egui::vec2(ui.available_width(), 30.0));
                if ui
                    .add_enabled(!busy, row)
                    .on_hover_text(&execution.id)
                    .clicked()
                {
                    chosen = Some(execution.id.clone());
                }
            }
        });
    if let Some(id) = chosen {
        if let Some(draft) = workbench.session.draft_mut() {
            draft.execution_id = Some(id.clone());
        }
        // The previous run's nodes must not stand in for this one while it is read.
        if workbench
            .status
            .as_ref()
            .is_some_and(|status| status.execution.id != id)
        {
            workbench.status = None;
        }
        intents.push(Intent::LoadExecution(id));
    }
}

fn status(ui: &mut egui::Ui, workbench: &Workbench) {
    let chosen = workbench
        .session
        .draft()
        .and_then(|draft| draft.execution_id.as_deref());
    let shown = workbench
        .status
        .as_ref()
        .filter(|status| Some(status.execution.id.as_str()) == chosen);
    let Some(status) = shown else {
        widgets::caption(ui, "Choose a run to see what each node produced.");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        widgets::badge(
            ui,
            words::label(status.execution.status),
            words::tone(status.execution.status),
        );
        widgets::caption(ui, readable_time(&status.execution.created_at));
    });
    ui.add_space(theme::SPACE_XS);
    for (name, node) in &status.nodes {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            widgets::badge(
                ui,
                words::node_label(node.status),
                words::node_tone(node.status),
            );
        });
        if let Some(ExecutionNodeOutput::Inline { value }) = &node.output {
            let text = value.to_string();
            let preview: String = text.chars().take(OUTPUT_PREVIEW).collect();
            let shown = if preview.len() < text.len() {
                format!("{preview}…")
            } else {
                preview
            };
            ui.label(RichText::new(shown).monospace().color(theme::TEXT_MUTED));
        }
        // Why a node failed is the first thing a person looks for, so it is shown where the node is.
        if let Some(error) = &node.error {
            let reason = error.message.as_deref().unwrap_or(&error.category);
            ui.label(RichText::new(format!("{}: {reason}", error.code)).color(theme::DANGER));
        }
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
