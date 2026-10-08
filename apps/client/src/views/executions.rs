//! Every run of the workspace, newest first, filtered by status and workflow. Choosing one shows its
//! detail: status and timing, a timeline of its nodes, and each node's attempts, output and failure.
//! A run that is still going updates live.

use super::{Intent, Intents, states, status};
use crate::{
    clock,
    effects::ended,
    theme,
    widgets::{self, Tone},
    workbench::{
        Page, Workbench,
        pages::{STATUS_FILTERS, StatusFilter},
    },
};
use eframe::egui::{self, RichText};
use nebula_api_contract::v1::execution::{
    ExecutionDetailResponse, ExecutionNode, ExecutionNodeOutput, ExecutionSummary,
};

/// Panels at least this wide show the detail beside the list.
const SIDE_BY_SIDE_MIN: f32 = 860.0;
/// Height of one node's row in the timeline.
const TIMELINE_ROW: f32 = 26.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let busy = workbench.session.busy();
    if workbench.executions.list.wants_read() {
        intents.push(Intent::LoadExecutions { more: false });
    }
    if workbench.executions.workflows.wants_read() {
        intents.push(Intent::LoadWorkflowChoices);
    }
    states::page_header(
        ui,
        "Executions",
        "Runs of every workflow in this workspace, newest first.",
        |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh"))
                .on_hover_text("Read the history again")
                .clicked()
            {
                intents.push(Intent::LoadExecutions { more: false });
            }
        },
    );
    filters(ui, workbench);
    ui.add_space(theme::SPACE_MD);
    if ui.available_width() >= SIDE_BY_SIDE_MIN {
        ui.columns(2, |columns| {
            list(&mut columns[0], workbench, intents, busy);
            detail(&mut columns[1], workbench, intents, busy);
        });
    } else {
        list(ui, workbench, intents, busy);
        ui.add_space(theme::SPACE_LG);
        detail(ui, workbench, intents, busy);
    }
}

/// Status chips and the workflow choice. Changing either reads the history again.
fn filters(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for status in STATUS_FILTERS {
            let filter = StatusFilter::of(status);
            let on = workbench.executions.statuses.contains(&filter);
            if ui
                .selectable_label(on, status::label(status))
                .on_hover_text(if on {
                    "Stop filtering by this status"
                } else {
                    "Show only runs with this status"
                })
                .clicked()
            {
                if on {
                    workbench.executions.statuses.remove(&filter);
                } else {
                    workbench.executions.statuses.insert(filter);
                }
                changed = true;
            }
        }
    });
    ui.add_space(theme::SPACE_XS);
    // On its own line, no wider than the window, so the chips above can wrap freely.
    let chosen = workbench.executions.workflow.clone();
    let label = chosen.as_deref().map_or_else(
        || "All workflows".to_owned(),
        |id| workflow_name(workbench, id),
    );
    let filter = egui::ComboBox::from_id_salt("executions-workflow")
        .selected_text(label)
        .width(ui.available_width().min(260.0))
        .truncate()
        .show_ui(ui, |ui| {
            let mut choice = chosen.clone();
            ui.selectable_value(&mut choice, None, "All workflows");
            // Every workflow of the workspace, not the one page the Workflows list holds.
            match workbench.executions.workflows.value() {
                Some(workflows) => {
                    for workflow in workflows {
                        ui.selectable_value(&mut choice, Some(workflow.id.clone()), &workflow.name);
                    }
                },
                None => {
                    ui.add_enabled(false, egui::Label::new("Reading workflows…"));
                },
            }
            if choice != chosen {
                workbench.executions.workflow = choice;
                changed = true;
            }
        });
    widgets::named(ui, &filter.response, "Show runs of workflow");
    if changed {
        workbench.executions.list.invalidate();
        workbench.executions.next_cursor = None;
    }
}

fn workflow_name(workbench: &Workbench, id: &str) -> String {
    workbench
        .executions
        .workflows
        .value()
        .into_iter()
        .flatten()
        .chain(&workbench.navigator.workflows)
        .find(|workflow| workflow.id == id)
        .map_or_else(|| short(id), |workflow| workflow.name.clone())
}

/// `exe_01M4EKKAN9DB52KCRFRSR5DNZH` as `exe_…R5DNZH`: enough to tell runs apart in a list.
fn short(id: &str) -> String {
    match id.split_once('_') {
        Some((prefix, rest)) if rest.len() > 8 => {
            format!("{prefix}_…{}", &rest[rest.len() - 6..])
        },
        _ => id.to_owned(),
    }
}

fn list(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let rows = match states::show(ui, &workbench.executions.list, "executions") {
        states::Shown::Ready(rows) => rows.clone(),
        states::Shown::Retry => {
            intents.push(Intent::LoadExecutions { more: false });
            return;
        },
        states::Shown::Waiting => return,
    };
    if rows.is_empty() {
        let filtered =
            !workbench.executions.statuses.is_empty() || workbench.executions.workflow.is_some();
        if filtered {
            if states::empty(
                ui,
                "No runs match",
                "No run of this workspace has the chosen status or workflow.",
                Some("Clear filters"),
            ) {
                workbench.executions.statuses.clear();
                workbench.executions.workflow = None;
                workbench.executions.list.invalidate();
            }
        } else if states::empty(
            ui,
            "No runs yet",
            "Publish a workflow and run it; its runs appear here as they happen.",
            Some("Open workflows"),
        ) {
            workbench.go(Page::Workflows);
        }
        return;
    }
    let now = clock::now_millis();
    let mut opened = None;
    for row in &rows {
        let selected = workbench.executions.selected.as_deref() == Some(row.id.as_str());
        let name = workflow_name(workbench, &row.workflow_id);
        let created = clock::parse_rfc3339(&row.created_at).unwrap_or(now);
        let frame = if selected {
            theme::card().stroke(egui::Stroke::new(1.0, theme::ACCENT))
        } else {
            theme::card()
        };
        let response = frame
            .inner_margin(egui::Margin::same(theme::SPACE_SM as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::badge(ui, status::label(row.status), status::tone(row.status));
                    // One wrapping column after the badge, so a narrow window never widens the row.
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).strong());
                        let mut facts = vec![short(&row.id), clock::ago(now, created)];
                        facts.extend(took(row, now));
                        widgets::caption(ui, facts.join(" · "));
                    });
                });
            })
            .response;
        let label = format!(
            "Show the {} run of {name}",
            status::label(row.status).to_lowercase()
        );
        if widgets::card_clicked(&response, &label, !busy) {
            opened = Some(row.id.clone());
        }
        ui.add_space(theme::SPACE_XS);
    }
    if workbench.executions.next_cursor.is_some() {
        ui.add_space(theme::SPACE_SM);
        // A failed page leaves the rows above; the same button reads that page again.
        if let Some(reason) = &workbench.executions.more_error {
            widgets::banner(
                ui,
                Tone::Danger,
                &format!("Older runs could not be read. {reason}"),
            );
        }
        let more = workbench.executions.appending;
        let label = if more {
            "Reading…"
        } else if workbench.executions.more_error.is_some() {
            "Try again"
        } else {
            "Show older runs"
        };
        if ui
            .add_enabled(!busy && !more, egui::Button::new(label))
            .clicked()
        {
            intents.push(Intent::LoadExecutions { more: true });
        }
    }
    if let Some(id) = opened {
        intents.push(Intent::OpenExecution(id));
    }
}

/// How long a run took, or has been going, from its start.
fn took(run: &ExecutionSummary, now: i64) -> Option<String> {
    let started = clock::parse_rfc3339(run.started_at.as_deref()?)?;
    let finished = run
        .finished_at
        .as_deref()
        .and_then(clock::parse_rfc3339)
        .unwrap_or(now);
    Some(clock::duration(finished - started))
}

fn detail(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents, busy: bool) {
    let Some(selected) = workbench.executions.selected.clone() else {
        theme::card_block(ui, |ui| {
            states::empty(
                ui,
                "Choose a run",
                "Its status, timeline and each node's output show here.",
                None,
            );
        });
        return;
    };
    if workbench.executions.detail.wants_read() {
        intents.push(Intent::OpenExecution(selected.clone()));
    }
    theme::card_block(ui, |ui| {
        let detail = match states::show(ui, &workbench.executions.detail, "run") {
            states::Shown::Ready(detail) if detail.execution.id == selected => detail.clone(),
            states::Shown::Retry => {
                intents.push(Intent::OpenExecution(selected.clone()));
                return;
            },
            _ => return,
        };
        let live = !ended(detail.execution.status);
        summary(ui, workbench, intents, &detail, live, busy);
        ui.add_space(theme::SPACE_MD);
        timeline(ui, workbench, &detail);
        if let Some(node) = workbench.executions.node.clone()
            && let Some(evidence) = detail.nodes.get(&node)
        {
            ui.add_space(theme::SPACE_MD);
            node_detail(ui, &node, evidence);
        }
        if let Some(input) = &detail.input {
            ui.add_space(theme::SPACE_MD);
            egui::CollapsingHeader::new("Run input")
                .id_salt("run-input")
                .show(ui, |ui| {
                    json_block(ui, "Run input", input, "run-input-json");
                });
        }
    });
}

fn summary(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    intents: &mut Intents,
    detail: &ExecutionDetailResponse,
    live: bool,
    busy: bool,
) {
    let run = &detail.execution;
    let now = clock::now_millis();
    ui.horizontal_wrapped(|ui| {
        widgets::badge(ui, status::label(run.status), status::tone(run.status));
        if live {
            widgets::badge(ui, "Live", Tone::Accent);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        let name = workflow_name(workbench, &run.workflow_id);
        if ui
            .link(RichText::new(&name).strong())
            .on_hover_text("Open this workflow in the editor")
            .clicked()
            && workbench.select_workflow(&run.workflow_id)
        {
            intents.push(Intent::LoadWorkflow(run.workflow_id.clone()));
        }
    });
    ui.label(RichText::new(&run.id).monospace().color(theme::TEXT_MUTED));
    ui.add_space(theme::SPACE_SM);
    egui::Grid::new("run-facts")
        .num_columns(2)
        .spacing([theme::SPACE_LG, theme::SPACE_XS])
        .show(ui, |ui| {
            let created = clock::parse_rfc3339(&run.created_at).unwrap_or(now);
            fact(
                ui,
                "Created",
                &format!(
                    "{} ({})",
                    readable(&run.created_at),
                    clock::ago(now, created)
                ),
            );
            if let Some(took) = took(run, now) {
                fact(ui, if live { "Running for" } else { "Took" }, &took);
            }
            fact(ui, "Nodes", &detail.nodes.len().to_string());
            fact(ui, "Output", &bytes(detail.total_output_bytes));
            if detail.total_retries > 0 {
                fact(ui, "Retries", &detail.total_retries.to_string());
            }
        });
    ui.add_space(theme::SPACE_SM);
    ui.horizontal_wrapped(|ui| {
        if live
            && ui
                .add_enabled(!busy, widgets::danger_button("Cancel run"))
                .on_hover_text("Ask the runtime to stop this run")
                .clicked()
        {
            intents.push(Intent::CancelExecution(run.id.clone()));
        }
        if !live
            && ui
                .add_enabled(!busy, egui::Button::new("Run again"))
                .on_hover_text("Start the workflow's published version again")
                .clicked()
        {
            intents.push(Intent::RerunWorkflow(run.workflow_id.clone()));
        }
    });
}

fn fact(ui: &mut egui::Ui, name: &str, value: &str) {
    widgets::caption(ui, name);
    ui.label(value);
    ui.end_row();
}

/// `2026-10-08T20:34:40.939721Z` as `2026-10-08 20:34:40`.
fn readable(rfc3339: &str) -> String {
    rfc3339.get(..19).unwrap_or(rfc3339).replace('T', " ")
}

fn bytes(count: u64) -> String {
    match count {
        0..=1_023 => format!("{count} B"),
        1_024..=1_048_575 => format!("{:.1} KiB", count as f64 / 1024.0),
        _ => format!("{:.1} MiB", count as f64 / 1_048_576.0),
    }
}

/// Each node as a bar on the run's time axis, in the order they were scheduled. A bar runs from the
/// node's start (or scheduling) to its finish, or to now while it is going. Choosing a row shows the
/// node's evidence.
fn timeline(ui: &mut egui::Ui, workbench: &mut Workbench, detail: &ExecutionDetailResponse) {
    widgets::section(ui, "Timeline");
    if detail.nodes.is_empty() {
        widgets::caption(ui, "No node has been scheduled yet.");
        return;
    }
    let now = clock::now_millis();
    let start = clock::parse_rfc3339(&detail.execution.created_at).unwrap_or(now);
    let end = detail
        .execution
        .finished_at
        .as_deref()
        .and_then(clock::parse_rfc3339)
        .unwrap_or(now)
        .max(start + 1);
    let mut nodes: Vec<(&String, &ExecutionNode)> = detail.nodes.iter().collect();
    nodes.sort_by_key(|(_, node)| node.scheduled_at.clone());
    let label_width = (ui.available_width() * 0.32).clamp(90.0, 180.0);
    for (key, node) in nodes {
        let selected = workbench.executions.node.as_deref() == Some(key.as_str());
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TIMELINE_ROW),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                selected,
                format!("{key}: {}", status::node_label(node.status)),
            )
        });
        let painter = ui.painter_at(rect);
        if selected || response.hovered() {
            painter.rect_filled(rect, theme::RADIUS_SM, theme::FIELD);
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                theme::RADIUS_SM,
                egui::Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        painter.text(
            rect.left_center() + egui::vec2(theme::SPACE_SM, 0.0),
            egui::Align2::LEFT_CENTER,
            key,
            egui::FontId::proportional(theme::SIZE_BODY),
            theme::TEXT,
        );
        let track = egui::Rect::from_min_max(
            egui::pos2(rect.left() + label_width, rect.top() + 7.0),
            egui::pos2(rect.right() - theme::SPACE_SM, rect.bottom() - 7.0),
        );
        painter.rect_filled(track, theme::RADIUS_SM, theme::SIDEBAR);
        let from = node
            .started_at
            .as_deref()
            .or(node.scheduled_at.as_deref())
            .and_then(clock::parse_rfc3339)
            .unwrap_or(start);
        let to = node
            .finished_at
            .as_deref()
            .and_then(clock::parse_rfc3339)
            .unwrap_or(now)
            .max(from);
        let x = |instant: i64| {
            let share = ((instant - start) as f32 / (end - start) as f32).clamp(0.0, 1.0);
            track.width().mul_add(share, track.left())
        };
        let bar = egui::Rect::from_min_max(
            egui::pos2(x(from), track.top()),
            egui::pos2(x(to).max(x(from) + 3.0), track.bottom()),
        );
        let (color, _) = status::node_tone(node.status).colors();
        painter.rect_filled(bar, theme::RADIUS_SM, color);
        if response.clicked() {
            workbench.executions.node = if selected { None } else { Some(key.clone()) };
        }
        response.on_hover_text(format!(
            "{} · {}",
            status::node_label(node.status),
            clock::duration(to - from)
        ));
    }
    widgets::caption(ui, "Choose a node to see its attempts and output.");
}

fn node_detail(ui: &mut egui::Ui, key: &str, node: &ExecutionNode) {
    ui.horizontal(|ui| {
        widgets::section(ui, key);
        widgets::badge(
            ui,
            status::node_label(node.status),
            status::node_tone(node.status),
        );
    });
    if let Some(next) = &node.next_attempt_at {
        widgets::caption(ui, format!("Continues at {}", readable(next)));
    }
    if let Some(error) = &node.error {
        let message = error.message.as_deref().unwrap_or(&error.category);
        widgets::banner(ui, Tone::Danger, &format!("{}: {message}", error.code));
        widgets::caption(
            ui,
            format!(
                "Category {}{}",
                error.category,
                if error.retryable { ", retryable" } else { "" }
            ),
        );
    }
    if !node.attempts.is_empty() {
        ui.add_space(theme::SPACE_SM);
        egui::Grid::new(("attempts", key))
            .num_columns(3)
            .striped(true)
            .spacing([theme::SPACE_LG, theme::SPACE_XS])
            .show(ui, |ui| {
                widgets::caption(ui, "Attempt");
                widgets::caption(ui, "Finished");
                widgets::caption(ui, "Output");
                ui.end_row();
                for attempt in &node.attempts {
                    ui.label(attempt.attempt_number.to_string());
                    ui.label(
                        attempt
                            .finished_at
                            .as_deref()
                            .map_or_else(|| "—".to_owned(), readable),
                    );
                    ui.label(bytes(attempt.output_bytes));
                    ui.end_row();
                }
            });
    }
    match &node.output {
        Some(ExecutionNodeOutput::Inline { value }) => {
            ui.add_space(theme::SPACE_SM);
            widgets::caption(ui, "Output");
            json_block(ui, &format!("Output of {key}"), value, ("node-output", key));
        },
        Some(ExecutionNodeOutput::External { size, mime }) => widgets::caption(
            ui,
            format!(
                "Stored outside the run record ({}{}).",
                size.map_or_else(|| "size unknown".to_owned(), bytes),
                mime.as_deref()
                    .map(|mime| format!(", {mime}"))
                    .unwrap_or_default()
            ),
        ),
        Some(ExecutionNodeOutput::Binary { size, mime }) => {
            widgets::caption(ui, format!("Binary output, {} of {mime}.", bytes(*size)));
        },
        Some(ExecutionNodeOutput::Collection { items }) => {
            widgets::caption(ui, format!("A collection of {} items.", items.len()));
        },
        Some(ExecutionNodeOutput::Deferred) => widgets::caption(ui, "The result arrives later."),
        Some(ExecutionNodeOutput::Empty) | None => {},
    }
}

/// Pretty JSON in a bounded, scrolling block that can be selected and copied but not edited.
/// `name` is what assistive technology calls it.
pub(crate) fn json_block(
    ui: &mut egui::Ui,
    name: &str,
    value: &serde_json::Value,
    salt: impl std::hash::Hash + std::fmt::Debug,
) {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    // A `&str` buffer is read-only, so the text is selectable without pretending to be editable.
    let mut text = pretty.as_str();
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .max_height(280.0)
        .show(ui, |ui| {
            let block = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(1),
            );
            widgets::named(ui, &block, name);
        });
}
