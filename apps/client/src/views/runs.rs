//! Runs of the open workflow, read from what the server persisted: recent runs, and the nodes of the
//! chosen one with their outputs. Starting a run lives on the canvas.
use super::{Intent, Intents};
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, Align, Layout, RichText};
use nebula_api_contract::v1::execution::{ExecutionNodeOutput, ExecutionStatus};

/// Panels at least this wide show the chosen run beside the list instead of under it.
const SIDE_BY_SIDE_MIN: f32 = 720.0;
/// Longest node output shown inline; the rest is behind the run details.
const OUTPUT_PREVIEW: usize = 160;
/// Seconds between reads of a run that has not ended.
const FOLLOW_SECONDS: f64 = 2.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(draft) = workbench.session.draft() else {
        return;
    };
    let execution = draft.execution_id.clone();
    let busy = workbench.session.busy();
    // The list loads once per opened workflow and again after a start, without a click. The app marks
    // it requested only when the request starts, so losing a frame to another request asks again.
    if !workbench.history_requested && !busy {
        intents.push(Intent::LoadRecentRuns);
    }
    follow(ui, workbench, execution.as_deref(), intents, busy);
    ui.horizontal(|ui| {
        widgets::section(ui, "Runs");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh").small())
                .on_hover_text("Read recent runs and the chosen run again")
                .clicked()
            {
                intents.push(Intent::LoadRecentRuns);
                if let Some(id) = execution {
                    intents.push(Intent::LoadExecution(id));
                }
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

/// Keeps the chosen run current: it is read once chosen, then every few seconds until it ends, and the
/// list is read again while its row still shows an older status. Reads stay spaced out even when they
/// fail, so a server in trouble is not asked every frame.
fn follow(
    ui: &egui::Ui,
    workbench: &Workbench,
    execution: Option<&str>,
    intents: &mut Intents,
    busy: bool,
) {
    let Some(id) = execution else {
        return;
    };
    let shown = workbench
        .status
        .as_ref()
        .filter(|status| status.execution.id == id);
    let ended = shown.is_some_and(|status| ended(status.execution.status));
    let row_behind = shown
        .zip(workbench.history.as_ref())
        .is_some_and(|(status, runs)| {
            runs.items
                .iter()
                .any(|run| run.id == id && run.status != status.execution.status)
        });
    if ended && !row_behind {
        return;
    }
    let context = ui.ctx();
    let now = context.input(|input| input.time);
    let clock = egui::Id::new(("run-follow", id));
    let due = context
        .data(|data| data.get_temp::<f64>(clock))
        .is_none_or(|last| now - last >= FOLLOW_SECONDS);
    if due && !busy {
        context.data_mut(|data| data.insert_temp(clock, now));
        intents.push(if ended {
            Intent::LoadRecentRuns
        } else {
            Intent::LoadExecution(id.to_owned())
        });
    }
    context.request_repaint_after(std::time::Duration::from_secs_f64(FOLLOW_SECONDS));
}

/// A run in one of these states will not change again.
const fn ended(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Completed
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled
            | ExecutionStatus::TimedOut
    )
}

fn status_tone(status: ExecutionStatus) -> Tone {
    match status {
        ExecutionStatus::Completed => Tone::Success,
        ExecutionStatus::Failed | ExecutionStatus::TimedOut => Tone::Danger,
        ExecutionStatus::Cancelled | ExecutionStatus::Cancelling => Tone::Warning,
        ExecutionStatus::Created | ExecutionStatus::Running | ExecutionStatus::Paused => {
            Tone::Accent
        },
    }
}

fn status_label(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Created => "Queued",
        ExecutionStatus::Running => "Running",
        ExecutionStatus::Paused => "Paused",
        ExecutionStatus::Cancelling => "Cancelling",
        ExecutionStatus::Completed => "Completed",
        ExecutionStatus::Failed => "Failed",
        ExecutionStatus::Cancelled => "Cancelled",
        ExecutionStatus::TimedOut => "Timed out",
    }
}

/// `2026-10-08T12:34:56.123456Z` as `2026-10-08 12:34:56`, the precision a person reads.
fn readable_time(rfc3339: &str) -> String {
    rfc3339.get(..19).unwrap_or(rfc3339).replace('T', " ")
}

fn history(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let Some(runs) = &workbench.history else {
        widgets::caption(ui, "Reading recent runs…");
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
                let (color, _) = status_tone(execution.status).colors();
                // Status and time are what a person scans for; the id is in the run's details.
                let row = egui::Button::selectable(
                    selected,
                    RichText::new(status_label(execution.status)).color(color),
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
        intents.push(Intent::LoadExecution(id));
    }
}

fn status(ui: &mut egui::Ui, workbench: &Workbench) {
    let Some(status) = &workbench.status else {
        widgets::caption(ui, "Choose a run to see what each node produced.");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        widgets::badge(
            ui,
            status_label(status.execution.status),
            status_tone(status.execution.status),
        );
        widgets::caption(ui, readable_time(&status.execution.created_at));
    });
    ui.add_space(theme::SPACE_XS);
    for (name, node) in &status.nodes {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            widgets::caption(ui, format!("{:?}", node.status));
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
