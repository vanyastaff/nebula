//! Live Monitor page — workflow execution monitoring with killer features.
//!
//! Inspired by Temporal, Inngest, Prefect, Dagster:
//! - Replay/Retry controls for failed executions
//! - Live metrics (success rate, latency, throughput)
//! - Full-text search in executions and logs
//! - Execution pinning/bookmarks
//! - SLA threshold indicators
//! - Step-level retry and debugging
//! - Execution comparison/diff

use gpui::prelude::*;
use gpui::{FontWeight, div, px};

use gpui_component::Sizable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::data::{
    EventHistoryEntry, ExecutionDetails, LogEntry, LogLevel, MonitorExecution, MonitorStatus,
    NodeDetail, StepStatus, TraceStep, TraceStepType, event_history, execution_details,
    execution_log, heatmap_data, monitor_executions, monitor_summary, node_detail,
    related_executions, total_duration_ms, trace_steps, workers,
};
use crate::theme::{fonts, shadcn};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum TraceViewMode {
    #[default]
    Compact,
    Timeline,
    Json,
}

/// Center content tab: timeline (Gantt + node detail), logs, or events.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum CenterTab {
    #[default]
    Timeline,
    Logs,
    Events,
}

pub struct MonitorPage;

impl MonitorPage {
    pub fn new() -> Self {
        Self
    }
}

impl gpui::Render for MonitorPage {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let summary = monitor_summary();
        let executions = monitor_executions();
        let selected_state =
            window.use_keyed_state::<String>("monitor_selected", cx, |_, _| "ex_f3a".into());
        let selected = selected_state.read(cx).clone();

        let trace = trace_steps(&selected);
        let log_entries = execution_log(&selected);
        let details = execution_details(&selected);
        let total_ms = total_duration_ms(&selected);
        let selected_node =
            window.use_keyed_state::<Option<usize>>("monitor_selected_node", cx, |_, _| None);
        let status_filter =
            window
                .use_keyed_state::<Option<MonitorStatus>>("monitor_status_filter", cx, |_, _| None);
        let view_mode = window.use_keyed_state::<TraceViewMode>("monitor_view_mode", cx, |_, _| {
            TraceViewMode::Compact
        });
        let center_tab = window
            .use_keyed_state::<CenterTab>("monitor_center_tab", cx, |_, _| CenterTab::Timeline);
        let log_level_filter =
            window.use_keyed_state::<Option<LogLevel>>("monitor_log_filter", cx, |_, _| None);
        let pinned_executions =
            window.use_keyed_state::<Vec<String>>("monitor_pinned", cx, |_, _| vec![]);
        let search_query =
            window.use_keyed_state::<String>("monitor_search", cx, |_, _| String::new());

        let selected_exec = executions.iter().find(|e| e.id == selected);
        let events = event_history(&selected);

        v_flex()
            .size_full()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
            .bg(shadcn::background())
            // Top nav: breadcrumbs, LIVE, actions
            .child(monitor_top_nav())
            // Metrics bar
            .child(live_metrics_bar(&summary))
            // Main content
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(execution_list_panel(
                        &executions,
                        &selected,
                        &summary,
                        selected_state.clone(),
                        status_filter.clone(),
                        selected_node.clone(),
                        pinned_executions.clone(),
                        search_query.clone(),
                        cx,
                    ))
                    .child(execution_trace_panel(
                        &selected,
                        selected_exec,
                        &trace,
                        &log_entries,
                        &events,
                        total_ms,
                        selected_node.clone(),
                        view_mode.clone(),
                        center_tab.clone(),
                        log_level_filter.clone(),
                        cx,
                    ))
                    .child(right_panel(
                        &summary,
                        details.as_ref(),
                        selected_exec,
                        &selected,
                    )),
            )
            // Status bar
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(22.))
                    .px_4()
                    .items_center()
                    .gap_4()
                    .border_t_1()
                    .border_color(shadcn::border())
                    .bg(shadcn::card())
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(5.))
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(shadcn::success()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child("Database: healthy"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(5.))
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(shadcn::success()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format!(
                                        "Workers: {}/{}",
                                        summary.workers_active, summary.workers_total
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(5.))
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(shadcn::success()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format!("Cluster: {} node", summary.cluster_nodes)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(5.))
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(shadcn::success()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format!("Uptime: {}%", summary.uptime_pct)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .font_family(fonts::MONO)
                            .text_color(shadcn::muted_foreground())
                            .child("Nebula v4.1"),
                    ),
            )
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TOP NAV: breadcrumbs, LIVE, actions (match design)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn monitor_top_nav() -> impl gpui::IntoElement {
    h_flex()
        .flex_shrink_0()
        .h(px(38.))
        .px_4()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child("nebula"),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::border())
                        .child("›"),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child("production"),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::border())
                        .child("›"),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(fonts::MONO)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(shadcn::muted_foreground())
                        .child("Monitor"),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(shadcn::success_bg())
                .border_1()
                .border_color(shadcn::success_border())
                .child(
                    div()
                        .w(px(6.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(shadcn::success()),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(shadcn::success())
                        .child("LIVE"),
                ),
        )
        .child(div().flex_1())
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Button::new("pause").ghost().xsmall().child("⏸ Pause"))
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(shadcn::violet().opacity(0.4))
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::violet())
                        .child("↺ Replay"),
                )
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(shadcn::destructive().opacity(0.3))
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::destructive())
                        .child("✕ Cancel"),
                )
                .child(div().w(px(1.)).h(px(16.)).bg(shadcn::border()))
                .child(Button::new("settings").ghost().xsmall().child("⚙ Settings"))
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(shadcn::success_bg())
                        .border_1()
                        .border_color(shadcn::success_border())
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::success())
                        .child("⊕ New"),
                ),
        )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LIVE METRICS BAR
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn live_metrics_bar(summary: &crate::data::MonitorSummary) -> impl gpui::IntoElement {
    let (p50, _p90, p95, p99) = summary.percentiles_ms;

    h_flex()
        .flex_shrink_0()
        .h(px(60.))
        .px_4()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        // Success rate + sparkline, latency + percentiles, throughput, counts, infrastructure
        .child(
            h_flex()
                .gap_4()
                .items_center()
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("SUCCESS RATE"),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_family(fonts::MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(shadcn::success())
                                        .child(format!("{}%", summary.success_rate_pct)),
                                )
                                .child(sparkline_bar(
                                    &summary.success_sparkline,
                                    shadcn::success(),
                                    52.,
                                    18.,
                                )),
                        ),
                )
                .child(div().w(px(1.)).h(px(36.)).bg(shadcn::border()))
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("AVG LATENCY"),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_color(shadcn::flow())
                                .child(format!("{}ms", summary.avg_latency_ms)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(fonts::MONO)
                                        .text_color(shadcn::muted_foreground())
                                        .child(format!("P50 {}ms", p50)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(fonts::MONO)
                                        .text_color(shadcn::muted_foreground())
                                        .child(format!("P95 {}ms", p95)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(fonts::MONO)
                                        .text_color(shadcn::muted_foreground())
                                        .child(format!("P99 {}s", p99 / 1000)),
                                ),
                        ),
                )
                .child(div().w(px(1.)).h(px(36.)).bg(shadcn::border()))
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("THROUGHPUT"),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_color(shadcn::violet())
                                .child(format!("{}/min", summary.throughput_per_min)),
                        ),
                )
                .child(div().w(px(1.)).h(px(36.)).bg(shadcn::border()))
                .child(
                    h_flex().gap_1().children(
                        [
                            ("RUNNING", summary.running, shadcn::warning()),
                            ("QUEUED", summary.queued, shadcn::muted_foreground()),
                            ("DONE", summary.done_1m, shadcn::success()),
                            ("FAILED", summary.failed_1m, shadcn::destructive()),
                        ]
                        .iter()
                        .map(|(label, n, color)| {
                            v_flex()
                                .items_center()
                                .justify_center()
                                .px_3()
                                .py_1()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_family(fonts::MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(*color)
                                        .child(format!("{}", n)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(shadcn::muted_foreground())
                                        .child(*label),
                                )
                        }),
                    ),
                )
                .child(div().w(px(1.)).h(px(36.)).bg(shadcn::border()))
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("INFRASTRUCTURE"),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .child(
                                    v_flex()
                                        .gap(px(1.))
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_family(fonts::MONO)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(shadcn::success())
                                                .child(format!(
                                                    "{}/{}",
                                                    summary.workers_active, summary.workers_total
                                                )),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(shadcn::muted_foreground())
                                                .child("Workers"),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap(px(1.))
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_family(fonts::MONO)
                                                .text_color(shadcn::muted_foreground())
                                                .child(format!("{} node", summary.cluster_nodes)),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(shadcn::muted_foreground())
                                                .child("Cluster"),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap(px(1.))
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_family(fonts::MONO)
                                                .text_color(shadcn::success())
                                                .child(format!("{}%", summary.uptime_pct)),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(shadcn::muted_foreground())
                                                .child("Uptime"),
                                        ),
                                ),
                        ),
                ),
        )
}

/// Simple sparkline as a row of bars (no SVG path for compatibility).
fn sparkline_bar(
    data: &[u32],
    color: gpui::Rgba,
    total_w: f32,
    bar_h: f32,
) -> impl gpui::IntoElement {
    let max = data.iter().copied().max().unwrap_or(1).max(1) as f32;
    let bar_w = if data.is_empty() {
        0.
    } else {
        (total_w / data.len() as f32).max(2.)
    };
    h_flex()
        .gap(px(1.))
        .h(px(bar_h))
        .items_end()
        .children(data.iter().map(|&v| {
            let pct = (v as f32 / max).min(1.0).max(0.1);
            div()
                .w(px(bar_w))
                .h(px(bar_h * pct))
                .rounded(px(1.))
                .bg(color)
        }))
}

fn metric_pill(label: &'static str, value: &str, healthy: bool) -> impl gpui::IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(if healthy {
                    shadcn::success().opacity(0.12)
                } else {
                    shadcn::warning().opacity(0.12)
                })
                .text_xs()
                .font_family(fonts::MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if healthy {
                    shadcn::success()
                } else {
                    shadcn::warning()
                })
                .child(value.to_string()),
        )
}

fn metric_pill_count(label: &'static str, value: u32, color: gpui::Rgba) -> impl gpui::IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(color.opacity(0.12))
                .text_xs()
                .font_family(fonts::MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value.to_string()),
        )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LEFT PANEL: Execution List with Search & Pinning
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn execution_list_panel(
    executions: &[MonitorExecution],
    selected_id: &str,
    summary: &crate::data::MonitorSummary,
    selected_state: gpui::Entity<String>,
    status_filter: gpui::Entity<Option<MonitorStatus>>,
    selected_node: gpui::Entity<Option<usize>>,
    pinned: gpui::Entity<Vec<String>>,
    search_query: gpui::Entity<String>,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let filter = status_filter.read(cx);
    let query = search_query.read(cx).to_lowercase();
    let pinned_ids = pinned.read(cx).clone();

    let filtered: Vec<_> = executions
        .iter()
        .filter(|e| {
            let matches_filter = match filter {
                None => true,
                Some(s) => e.status == *s,
            };
            let matches_search = query.is_empty()
                || e.id.to_lowercase().contains(&query)
                || e.workflow.to_lowercase().contains(&query);
            matches_filter && matches_search
        })
        .collect();

    // Sort: pinned first, then by time
    let mut pinned_execs: Vec<&MonitorExecution> = vec![];
    let mut unpinned_execs: Vec<&MonitorExecution> = vec![];
    for e in &filtered {
        if pinned_ids.contains(&e.id) {
            pinned_execs.push(e);
        } else {
            unpinned_execs.push(e);
        }
    }

    v_flex()
        .w(px(300.))
        .h_full()
        .flex_shrink_0()
        .min_h(px(0.))
        .overflow_hidden()
        .border_r_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        // Header
        .child(
            h_flex()
                .flex_shrink_0()
                .h(px(48.))
                .px_4()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(shadcn::foreground())
                        .child("Executions"),
                )
                .child(
                    div()
                        .px_2()
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(shadcn::muted())
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child(format!("{} total", filtered.len())),
                ),
        )
        // Search bar (Killer Feature #2)
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(shadcn::border())
                .bg(shadcn::background())
                .child(search_input(search_query.clone(), cx)),
        )
        // Stats
        .child(
            h_flex()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .bg(shadcn::background())
                .child(mini_stat(summary.running, "running", shadcn::success()))
                .child(mini_stat(
                    summary.queued,
                    "queued",
                    shadcn::muted_foreground(),
                ))
                .child(mini_stat(summary.done_1m, "done", shadcn::flow()))
                .child(mini_stat(
                    summary.failed_1m,
                    "failed",
                    shadcn::destructive(),
                )),
        )
        // Filter tabs
        .child(
            h_flex()
                .flex_shrink_0()
                .h(px(36.))
                .px_3()
                .gap_1()
                .items_center()
                .border_b_1()
                .border_color(shadcn::border())
                .bg(shadcn::background())
                .child(filter_tabs(status_filter, summary, cx)),
        )
        // Execution list
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .bg(shadcn::background())
                .child(
                    v_flex()
                        // Pinned section
                        .when(!pinned_execs.is_empty(), |this| {
                            this.child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .bg(shadcn::muted())
                                    .text_xs()
                                    .text_color(shadcn::muted_foreground())
                                    .child("📌 Pinned"),
                            )
                            .children(
                                pinned_execs.iter().enumerate().map(|(i, e)| {
                                    execution_row(
                                        e,
                                        i,
                                        e.id == selected_id,
                                        true,
                                        selected_state.clone(),
                                        selected_node.clone(),
                                        pinned.clone(),
                                    )
                                }),
                            )
                        })
                        // All executions
                        .children(unpinned_execs.iter().enumerate().map(|(i, e)| {
                            execution_row(
                                e,
                                i + pinned_execs.len(),
                                e.id == selected_id,
                                false,
                                selected_state.clone(),
                                selected_node.clone(),
                                pinned.clone(),
                            )
                        })),
                ),
        )
}

fn search_input(
    _search_query: gpui::Entity<String>,
    _cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    // Simplified search UI - in real implementation would use TextInput
    h_flex()
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .bg(shadcn::muted())
        .items_center()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("🔍"),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("id, workflow..."),
        )
        .child(
            div()
                .px_1()
                .py(px(1.))
                .rounded(px(3.))
                .bg(shadcn::background())
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(shadcn::muted_foreground())
                .child("⌘K"),
        )
}

fn mini_stat(value: u32, label: &'static str, color: gpui::Rgba) -> impl gpui::IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(
            div()
                .text_sm()
                .font_family(fonts::MONO)
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(value.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
}

fn filter_tabs(
    status_filter: gpui::Entity<Option<MonitorStatus>>,
    summary: &crate::data::MonitorSummary,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let current = *status_filter.read(cx);
    let sf_all = status_filter.clone();
    let sf_running = status_filter.clone();
    let sf_queued = status_filter.clone();
    let sf_completed = status_filter.clone();
    let sf_failed = status_filter.clone();

    h_flex()
        .gap_1()
        .child(filter_tab_with_count(
            "ALL",
            (summary.running + summary.queued + summary.done_1m + summary.failed_1m) as usize,
            current.is_none(),
            move |_, _, cx| {
                sf_all.update(cx, |v, _| *v = None);
            },
        ))
        .child(filter_tab_with_count(
            "RUNNING",
            summary.running as usize,
            current == Some(MonitorStatus::Running),
            move |_, _, cx| {
                sf_running.update(cx, |v, _| *v = Some(MonitorStatus::Running));
            },
        ))
        .child(filter_tab_with_count(
            "QUEUED",
            summary.queued as usize,
            current == Some(MonitorStatus::Queued),
            move |_, _, cx| {
                sf_queued.update(cx, |v, _| *v = Some(MonitorStatus::Queued));
            },
        ))
        .child(filter_tab_with_count(
            "COMPLETED",
            summary.done_1m as usize,
            current == Some(MonitorStatus::Completed),
            move |_, _, cx| {
                sf_completed.update(cx, |v, _| *v = Some(MonitorStatus::Completed));
            },
        ))
        .child(filter_tab_with_count(
            "FAILED",
            summary.failed_1m as usize,
            current == Some(MonitorStatus::Failed),
            move |_, _, cx| {
                sf_failed.update(cx, |v, _| *v = Some(MonitorStatus::Failed));
            },
        ))
}

fn filter_tab_with_count<F>(
    label: &'static str,
    count: usize,
    active: bool,
    on_click: F,
) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    let text = format!("{} {}", label.to_lowercase(), count);
    div()
        .id((label, count))
        .px_2()
        .py_1()
        .rounded(px(4.))
        .text_xs()
        .font_family(fonts::MONO)
        .cursor_pointer()
        .when(active, |d| {
            d.bg(shadcn::muted())
                .text_color(shadcn::foreground())
                .font_weight(FontWeight::MEDIUM)
                .border_b_2()
                .border_color(shadcn::success())
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(text)
}

fn filter_tab<F>(label: &'static str, active: bool, on_click: F) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    div()
        .id(label)
        .px_2()
        .py_1()
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(shadcn::muted())
                .text_color(shadcn::foreground())
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(label)
}

fn execution_row(
    e: &MonitorExecution,
    index: usize,
    is_selected: bool,
    is_pinned: bool,
    selected_state: gpui::Entity<String>,
    selected_node: gpui::Entity<Option<usize>>,
    pinned: gpui::Entity<Vec<String>>,
) -> impl gpui::IntoElement {
    let exec_id = e.id.clone();
    let exec_id_for_pin = e.id.clone();
    let status_color = match e.status {
        MonitorStatus::Running => shadcn::success(),
        MonitorStatus::Queued => shadcn::muted_foreground(),
        MonitorStatus::Completed => shadcn::flow(),
        MonitorStatus::Failed => shadcn::destructive(),
    };

    // SLA indicator (Killer Feature #3)
    let is_slow =
        e.duration.contains("1.") || e.duration.contains("2.") || e.duration.contains("3.");

    div()
        .id(("exec-row", index))
        .w_full()
        .px_3()
        .py_2()
        .cursor_pointer()
        .border_b_1()
        .border_color(shadcn::border())
        .when(is_selected, |d| {
            d.bg(shadcn::muted())
                .border_l_2()
                .border_color(shadcn::flow())
        })
        .when(!is_selected, |d| d.hover(|s| s.bg(shadcn::muted())))
        .on_click(move |_, _, cx| {
            selected_state.update(cx, |v, _| *v = exec_id.clone());
            selected_node.update(cx, |v, _| *v = None);
        })
        .child(
            v_flex()
                .gap(px(3.))
                .w_full()
                // Row 1: Status + Workflow + Pin
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(px(6.))
                                .h(px(6.))
                                .rounded_full()
                                .bg(status_color),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(shadcn::foreground())
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(e.workflow.clone()),
                        )
                        // Retries badge
                        .when(e.retries > 0, |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .px_1()
                                    .rounded(px(3.))
                                    .bg(shadcn::warning().opacity(0.2))
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::warning())
                                    .child(format!("↺{}", e.retries)),
                            )
                        })
                        // SLA warning indicator
                        .when(is_slow && e.status == MonitorStatus::Completed, |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(shadcn::warning())
                                    .child("⚡"),
                            )
                        })
                        // Pin button
                        .child(
                            div()
                                .id(("pin", index))
                                .flex_shrink_0()
                                .px_1()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(if is_pinned {
                                    shadcn::warning()
                                } else {
                                    shadcn::muted_foreground()
                                })
                                .hover(|s| s.text_color(shadcn::warning()))
                                .on_click(move |_, _, cx| {
                                    pinned.update(cx, |v, _| {
                                        if v.contains(&exec_id_for_pin) {
                                            v.retain(|id| id != &exec_id_for_pin);
                                        } else {
                                            v.push(exec_id_for_pin.clone());
                                        }
                                    });
                                })
                                .child(if is_pinned { "📌" } else { "○" }),
                        ),
                )
                // Row 2: ID + trigger
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .ml(px(14.))
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(e.id.clone()),
                        )
                        .child(trigger_badge(&e.trigger)),
                )
                // Row 3: Time + duration + nodes
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .ml(px(14.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child(e.started.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("·"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if is_slow {
                                    shadcn::warning()
                                } else {
                                    shadcn::foreground()
                                })
                                .child(e.duration.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(e.nodes.clone()),
                        )
                        .when_some(e.output_size.as_ref(), |this, out| {
                            this.child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format!("out {}", out)),
                            )
                        }),
                )
                // Error
                .when_some(e.error.as_ref(), |this, err| {
                    this.child(
                        div()
                            .mt_1()
                            .ml(px(14.))
                            .text_xs()
                            .text_color(shadcn::destructive())
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(err.clone()),
                    )
                }),
        )
}

fn trigger_badge(trigger: &str) -> impl gpui::IntoElement {
    let color = match trigger {
        "webhook" => shadcn::id_blue(),
        "cron" => shadcn::success(),
        "manual" => shadcn::violet(),
        _ => shadcn::muted_foreground(),
    };

    div()
        .flex_shrink_0()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(3.))
        .bg(color.opacity(0.12))
        .border_1()
        .border_color(color.opacity(0.3))
        .text_xs()
        .font_family(fonts::MONO)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(trigger.to_uppercase())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MIDDLE PANEL: Execution Trace with Replay Controls
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn execution_trace_panel(
    selected_id: &str,
    selected_exec: Option<&MonitorExecution>,
    steps: &[TraceStep],
    log_entries: &[LogEntry],
    events: &[EventHistoryEntry],
    total_ms: u32,
    selected_node: gpui::Entity<Option<usize>>,
    view_mode: gpui::Entity<TraceViewMode>,
    center_tab: gpui::Entity<CenterTab>,
    log_level_filter: gpui::Entity<Option<LogLevel>>,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let sel_node = *selected_node.read(cx);
    let current_view = *view_mode.read(cx);
    let tab = *center_tab.read(cx);
    let is_failed = selected_exec.map_or(false, |e| e.status == MonitorStatus::Failed);

    v_flex()
        .flex_1()
        .h_full()
        .min_w(px(380.))
        .min_h(px(0.))
        .overflow_hidden()
        .border_r_1()
        .border_color(shadcn::border())
        .bg(shadcn::background())
        .child(trace_header(
            selected_id,
            selected_exec,
            view_mode.clone(),
            center_tab.clone(),
            is_failed,
            cx,
        ))
        // Progress bar under header (design: colored segments by node status)
        .when(
            tab == CenterTab::Timeline && total_ms > 0 && !steps.is_empty(),
            |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(shadcn::border())
                        .child(progress_bar_by_steps(steps, total_ms)),
                )
            },
        )
        // Center tabs: Timeline | Logs | Events
        .child(center_tabs(
            center_tab.clone(),
            events.len(),
            log_entries.len(),
            sel_node,
            steps,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(match tab {
                    CenterTab::Timeline => v_flex()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p_4()
                        .child(match current_view {
                            TraceViewMode::Compact => compact_trace_view(
                                steps,
                                total_ms,
                                sel_node,
                                selected_node.clone(),
                                selected_id,
                                cx,
                            )
                            .into_any_element(),
                            TraceViewMode::Timeline => timeline_trace_view(
                                steps,
                                total_ms,
                                sel_node,
                                selected_node.clone(),
                                selected_id,
                                cx,
                            )
                            .into_any_element(),
                            TraceViewMode::Json => json_trace_view(steps).into_any_element(),
                        })
                        .into_any_element(),
                    CenterTab::Logs => v_flex()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p_3()
                        .child(logs_content(log_entries, log_level_filter.clone(), cx))
                        .into_any_element(),
                    CenterTab::Events => v_flex()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p_3()
                        .child(events_content(events))
                        .into_any_element(),
                }),
        )
}

fn trace_header(
    selected_id: &str,
    selected_exec: Option<&MonitorExecution>,
    view_mode: gpui::Entity<TraceViewMode>,
    center_tab: gpui::Entity<CenterTab>,
    is_failed: bool,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let current_view = *view_mode.read(cx);
    let vm_compact = view_mode.clone();
    let vm_timeline = view_mode.clone();
    let vm_json = view_mode.clone();

    v_flex()
        .flex_shrink_0()
        .border_b_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        // Top row
        .child(
            h_flex()
                .h(px(48.))
                .px_4()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(shadcn::id_blue())
                                .child(selected_id.to_string()),
                        )
                        .when_some(selected_exec, |this, e| {
                            let (color, text) = match e.status {
                                MonitorStatus::Running => (shadcn::success(), "Running"),
                                MonitorStatus::Queued => (shadcn::muted_foreground(), "Queued"),
                                MonitorStatus::Completed => (shadcn::flow(), "Completed"),
                                MonitorStatus::Failed => (shadcn::destructive(), "Failed"),
                            };
                            this.child(status_badge(text, color)).child(
                                div()
                                    .text_sm()
                                    .text_color(shadcn::muted_foreground())
                                    .child(e.workflow.clone()),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .when(is_failed, |this| {
                            this.child(Button::new("replay").primary().xsmall().child("↻ Replay"))
                        })
                        .child(Button::new("copy-id").ghost().xsmall().child("Copy ID"))
                        .child(
                            Button::new("open-editor")
                                .outline()
                                .xsmall()
                                .child("✎ Editor"),
                        ),
                ),
        )
        // View mode tabs (Compact / Timeline / JSON) when center tab is Timeline
        .when(*center_tab.read(cx) == CenterTab::Timeline, |this| {
            this.child(
                h_flex()
                    .h(px(36.))
                    .px_4()
                    .gap_1()
                    .items_center()
                    .border_t_1()
                    .border_color(shadcn::border())
                    .bg(shadcn::background())
                    .child(view_tab(
                        "Compact",
                        current_view == TraceViewMode::Compact,
                        move |_, _, cx| {
                            vm_compact.update(cx, |v, _| *v = TraceViewMode::Compact);
                        },
                    ))
                    .child(view_tab(
                        "Timeline",
                        current_view == TraceViewMode::Timeline,
                        move |_, _, cx| {
                            vm_timeline.update(cx, |v, _| *v = TraceViewMode::Timeline);
                        },
                    ))
                    .child(view_tab(
                        "JSON",
                        current_view == TraceViewMode::Json,
                        move |_, _, cx| {
                            vm_json.update(cx, |v, _| *v = TraceViewMode::Json);
                        },
                    )),
            )
        })
}

fn center_tabs(
    center_tab: gpui::Entity<CenterTab>,
    events_count: usize,
    logs_count: usize,
    sel_node: Option<usize>,
    steps: &[TraceStep],
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let tab = *center_tab.read(cx);
    let ct_timeline = center_tab.clone();
    let ct_logs = center_tab.clone();
    let ct_events = center_tab.clone();

    h_flex()
        .flex_shrink_0()
        .h(px(40.))
        .px_4()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(shadcn::border())
        .bg(shadcn::background())
        .child(center_tab_btn(
            "Timeline",
            tab == CenterTab::Timeline,
            move |_, _, cx| {
                ct_timeline.update(cx, |v, _| *v = CenterTab::Timeline);
            },
        ))
        .child(
            div()
                .id("tab-logs")
                .px_3()
                .py_2()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .when(tab == CenterTab::Logs, |d| {
                    d.bg(shadcn::muted())
                        .text_color(shadcn::foreground())
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(tab != CenterTab::Logs, |d| {
                    d.text_color(shadcn::muted_foreground())
                        .hover(|s| s.bg(shadcn::muted()))
                })
                .on_click(move |_, _, cx| {
                    ct_logs.update(cx, |v, _| *v = CenterTab::Logs);
                })
                .child(format!("Logs {}", logs_count)),
        )
        .child(
            div()
                .id("tab-events")
                .px_3()
                .py_2()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .when(tab == CenterTab::Events, |d| {
                    d.bg(shadcn::muted())
                        .text_color(shadcn::foreground())
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(tab != CenterTab::Events, |d| {
                    d.text_color(shadcn::muted_foreground())
                        .hover(|s| s.bg(shadcn::muted()))
                })
                .on_click(move |_, _, cx| {
                    ct_events.update(cx, |v, _| *v = CenterTab::Events);
                })
                .child(format!("Events {}", events_count)),
        )
        .child(
            div()
                .flex_1()
                .when(tab == CenterTab::Timeline && sel_node.is_some(), |d| {
                    d.child(
                        h_flex()
                            .justify_end()
                            .items_center()
                            .text_xs()
                            .text_color(shadcn::warning())
                            .child(
                                steps
                                    .iter()
                                    .find(|s| Some(s.id) == sel_node)
                                    .map(|s| format!("▼ {}", s.name))
                                    .unwrap_or_default(),
                            ),
                    )
                }),
        )
}

fn center_tab_btn<F>(label: &'static str, active: bool, on_click: F) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    div()
        .id(label)
        .px_3()
        .py_2()
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.border_b_2()
                .border_color(shadcn::id_blue())
                .text_color(shadcn::foreground())
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(label)
}

fn logs_content(
    log_entries: &[LogEntry],
    log_filter: gpui::Entity<Option<LogLevel>>,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let current = *log_filter.read(cx);
    let filtered: Vec<_> = match current {
        None => log_entries.iter().collect(),
        Some(level) => log_entries.iter().filter(|e| e.level == level).collect(),
    };
    let lf_all = log_filter.clone();
    let lf_info = log_filter.clone();
    let lf_warn = log_filter.clone();
    let lf_error = log_filter.clone();
    let warn_count = log_entries
        .iter()
        .filter(|e| e.level == LogLevel::Warn)
        .count();
    let error_count = log_entries
        .iter()
        .filter(|e| e.level == LogLevel::Error)
        .count();

    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(log_chip("All", current.is_none(), move |_, _, cx| {
                    lf_all.update(cx, |v, _| *v = None);
                }))
                .child(log_chip(
                    "INFO",
                    current == Some(LogLevel::Info),
                    move |_, _, cx| {
                        lf_info.update(cx, |v, _| *v = Some(LogLevel::Info));
                    },
                ))
                .child(log_chip_with_count(
                    "WARN",
                    warn_count,
                    current == Some(LogLevel::Warn),
                    move |_, _, cx| lf_warn.update(cx, |v, _| *v = Some(LogLevel::Warn)),
                ))
                .child(log_chip_with_count(
                    "ERROR",
                    error_count,
                    current == Some(LogLevel::Error),
                    move |_, _, cx| lf_error.update(cx, |v, _| *v = Some(LogLevel::Error)),
                )),
        )
        .child(
            v_flex()
                .gap(px(2.))
                .children(filtered.iter().map(|e| log_entry_row(e))),
        )
}

fn events_content(events: &[EventHistoryEntry]) -> impl gpui::IntoElement {
    if events.is_empty() {
        return div()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .h(px(120.))
            .text_xs()
            .text_color(shadcn::muted_foreground())
            .child("No event history");
    }
    v_flex().gap(px(2.)).children(events.iter().map(|ev| {
        let color = match ev.event_type.as_str() {
            "WorkflowExecutionStarted" | "WorkflowExecutionCompleted" => shadcn::success(),
            "ActivityTaskScheduled" | "ActivityTaskStarted" => shadcn::flow(),
            "ActivityTaskCompleted" => shadcn::success(),
            "ActivityTaskTimedOut" => shadcn::warning(),
            "ActivityTaskFailed" | "WorkflowExecutionFailed" => shadcn::destructive(),
            _ => shadcn::muted_foreground(),
        };
        h_flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .rounded(px(4.))
            .bg(shadcn::card())
            .border_1()
            .border_color(shadcn::border())
            .child(
                div()
                    .w(px(24.))
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::muted_foreground())
                    .child(format!("{}", ev.id)),
            )
            .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::foreground())
                    .child(
                        ev.event_type
                            .replace('A', " A")
                            .replace('T', " T")
                            .trim_start()
                            .to_string(),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::muted_foreground())
                    .child(ev.ts.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::muted_foreground())
                    .overflow_hidden()
                    .text_ellipsis()
                    .max_w(px(180.))
                    .child(ev.data_json.clone()),
            )
    }))
}

fn status_badge(text: &'static str, color: gpui::Rgba) -> impl gpui::IntoElement {
    div()
        .px_2()
        .py(px(3.))
        .rounded(px(4.))
        .bg(color.opacity(0.12))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(text)
}

fn view_tab<F>(label: &'static str, active: bool, on_click: F) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    div()
        .id(label)
        .px_3()
        .py_1()
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(shadcn::muted())
                .text_color(shadcn::foreground())
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(label)
}

// Compact view with step retry
fn compact_trace_view(
    steps: &[TraceStep],
    total_ms: u32,
    selected_node: Option<usize>,
    selected_node_state: gpui::Entity<Option<usize>>,
    selected_id: &str,
    _cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let node_detail = selected_node
        .and_then(|i| steps.get(i))
        .and_then(|s| node_detail(selected_id, s.id));

    v_flex()
        .gap_4()
        // Progress bar by step status
        .child(progress_bar_by_steps(steps, total_ms))
        // Duration bar with breakdown
        .child(
            v_flex()
                .gap_2()
                .px_3()
                .py_3()
                .rounded(px(8.))
                .bg(shadcn::card())
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child("Total Duration"),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_color(shadcn::flow())
                                .child(format_duration(total_ms)),
                        ),
                )
                .child(duration_breakdown_bar(steps, total_ms)),
        )
        // Steps
        .child(
            v_flex()
                .gap_2()
                .children(steps.iter().enumerate().map(|(i, step)| {
                    compact_step_row(
                        step,
                        i,
                        selected_node == Some(i),
                        selected_node_state.clone(),
                    )
                })),
        )
        // Selected step: rich node detail panel or simple step I/O
        .child(
            match (&node_detail, selected_node.and_then(|i| steps.get(i))) {
                (Some(d), _) => {
                    node_detail_panel(d.clone(), selected_node_state.clone()).into_any_element()
                }
                (None, Some(s)) => step_details_panel(s).into_any_element(),
                _ => div().into_any_element(),
            },
        )
}

fn progress_bar_by_steps(steps: &[TraceStep], total_ms: u32) -> impl gpui::IntoElement {
    if total_ms == 0 {
        return div().into_any_element();
    }
    let total_w = 400_f32;
    h_flex()
        .w_full()
        .h(px(6.))
        .rounded(px(4.))
        .bg(shadcn::muted())
        .overflow_hidden()
        .children(steps.iter().map(|step| {
            let color = match step.status {
                StepStatus::Error => shadcn::destructive(),
                StepStatus::Warn => shadcn::warning(),
                StepStatus::Ok => shadcn::success(),
            };
            let w = (step.duration_ms as f32 / total_ms as f32 * total_w).max(4.);
            div().h_full().w(px(w)).bg(color)
        }))
        .into_any_element()
}

fn duration_breakdown_bar(steps: &[TraceStep], total_ms: u32) -> impl gpui::IntoElement {
    h_flex()
        .w_full()
        .h(px(8.))
        .rounded(px(4.))
        .bg(shadcn::muted())
        .overflow_hidden()
        .children(steps.iter().map(|step| {
            let color = match step.status {
                StepStatus::Error => shadcn::destructive(),
                StepStatus::Warn => shadcn::warning(),
                StepStatus::Ok => match step.step_type {
                    TraceStepType::Trigger => shadcn::violet(),
                    TraceStepType::Http | TraceStepType::Db => shadcn::flow(),
                    TraceStepType::Action => shadcn::success(),
                    TraceStepType::Condition => shadcn::warning(),
                },
            };
            let width_pct = if total_ms > 0 {
                (step.duration_ms as f32 / total_ms as f32).max(0.02)
            } else {
                0.0
            };
            div().h_full().w(px(width_pct * 200.0)).bg(color)
        }))
}

fn compact_step_row(
    step: &TraceStep,
    index: usize,
    is_selected: bool,
    selected_node: gpui::Entity<Option<usize>>,
) -> impl gpui::IntoElement {
    let type_label = step_type_label(step.step_type);
    let type_color = match step.status {
        StepStatus::Error => shadcn::destructive(),
        StepStatus::Warn => shadcn::warning(),
        StepStatus::Ok => match step.step_type {
            TraceStepType::Trigger => shadcn::violet(),
            TraceStepType::Http | TraceStepType::Db => shadcn::flow(),
            TraceStepType::Action => shadcn::success(),
            TraceStepType::Condition => shadcn::warning(),
        },
    };
    let pct = format!("{}%", ((step.duration_ms as f32 / 547.0) * 100.0) as u32);

    div()
        .id(("step", index))
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .cursor_pointer()
        .border_1()
        .border_color(if is_selected {
            shadcn::flow()
        } else {
            shadcn::border()
        })
        .bg(if is_selected {
            shadcn::muted()
        } else {
            shadcn::card()
        })
        .hover(|s| s.bg(shadcn::muted()))
        .on_click(move |_, _, cx| {
            selected_node.update(cx, |v, _| *v = Some(index));
        })
        .child(
            h_flex()
                .gap_3()
                .items_center()
                // Step number
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(24.))
                        .h(px(24.))
                        .rounded(px(6.))
                        .bg(type_color.opacity(0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(type_color)
                        .child(format!("{}", index + 1)),
                )
                // Name + type
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(shadcn::foreground())
                                .child(step.name.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_xs().text_color(type_color).child(type_label))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(shadcn::muted_foreground())
                                        .child(pct),
                                ),
                        ),
                )
                // Timing
                .child(
                    v_flex()
                        .flex_shrink_0()
                        .items_end()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_sm()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(shadcn::foreground())
                                .child(format!("{}ms", step.duration_ms)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(format!("+{}", step.start_offset_ms)),
                        ),
                )
                // Retry button for actions (Killer Feature #5)
                .when(step.step_type == TraceStepType::Action, |this| {
                    this.child(
                        div()
                            .id(("retry", index))
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .text_xs()
                            .text_color(shadcn::muted_foreground())
                            .cursor_pointer()
                            .hover(|s| s.bg(shadcn::muted()).text_color(shadcn::foreground()))
                            .child("↻"),
                    )
                }),
        )
}

// Timeline view with Gantt bars and heatmap
fn timeline_trace_view(
    steps: &[TraceStep],
    total_ms: u32,
    selected_node: Option<usize>,
    selected_node_state: gpui::Entity<Option<usize>>,
    selected_id: &str,
    _cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let bar_width = 260.0_f32;
    let node_detail = selected_node
        .and_then(|i| steps.get(i))
        .and_then(|s| node_detail(selected_id, s.id));

    v_flex()
        .gap_4()
        // Time axis
        .child(
            h_flex()
                .items_center()
                .child(div().w(px(100.)).flex_shrink_0())
                .child(
                    div().w(px(bar_width)).child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child("0"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format_duration(total_ms)),
                            ),
                    ),
                ),
        )
        // Bars (status-colored)
        .child(
            v_flex()
                .gap_1()
                .children(steps.iter().enumerate().map(|(i, step)| {
                    let bar_color = match step.status {
                        StepStatus::Error => shadcn::destructive(),
                        StepStatus::Warn => shadcn::warning(),
                        StepStatus::Ok => match step.step_type {
                            TraceStepType::Trigger => shadcn::violet(),
                            TraceStepType::Http | TraceStepType::Db => shadcn::flow(),
                            TraceStepType::Action => shadcn::success(),
                            TraceStepType::Condition => shadcn::warning(),
                        },
                    };
                    let offset_pct = if total_ms > 0 {
                        step.start_offset_ms as f32 / total_ms as f32
                    } else {
                        0.0
                    };
                    let width_pct = if total_ms > 0 {
                        (step.duration_ms as f32 / total_ms as f32).max(0.03)
                    } else {
                        0.0
                    };
                    let is_selected = selected_node == Some(i);
                    let node_state = selected_node_state.clone();

                    div()
                        .id(("timeline-step", i))
                        .cursor_pointer()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .when(is_selected, |d| d.bg(shadcn::muted()))
                        .hover(|s| s.bg(shadcn::muted()))
                        .on_click(move |_, _, cx| {
                            node_state.update(cx, |v, _| *v = Some(i));
                        })
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(100.))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(shadcn::foreground())
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(step.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(bar_width))
                                        .h(px(20.))
                                        .rounded(px(4.))
                                        .bg(shadcn::muted())
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .h_full()
                                                .ml(px(offset_pct * bar_width))
                                                .w(px((width_pct * bar_width).max(6.0)))
                                                .rounded(px(4.))
                                                .bg(bar_color),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(50.))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(fonts::MONO)
                                        .text_color(match step.status {
                                            StepStatus::Error => shadcn::destructive(),
                                            StepStatus::Warn => shadcn::warning(),
                                            _ => shadcn::muted_foreground(),
                                        })
                                        .text_right()
                                        .child(format!("{}ms", step.duration_ms)),
                                ),
                        )
                })),
        )
        // Latency heatmap
        .child(
            v_flex()
                .gap_2()
                .pt_4()
                .border_t_1()
                .border_color(shadcn::border())
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child("LATENCY DISTRIBUTION · 24h"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::success())
                                .child("▲ this run"),
                        ),
                )
                .child(heatmap_viz()),
        )
        // Selected: rich node detail or step I/O
        .child(
            match (&node_detail, selected_node.and_then(|i| steps.get(i))) {
                (Some(d), _) => {
                    node_detail_panel(d.clone(), selected_node_state.clone()).into_any_element()
                }
                (None, Some(s)) => step_details_panel(s).into_any_element(),
                _ => div().into_any_element(),
            },
        )
}

fn heatmap_viz() -> impl gpui::IntoElement {
    let data = heatmap_data();
    v_flex().gap(px(2.)).children(data.iter().rev().map(|row| {
        h_flex().gap(px(2.)).children(row.iter().map(|&v| {
            let color = if v < 0.05 {
                shadcn::muted()
            } else {
                let intensity = (0.15 + v * 0.85).min(1.0);
                shadcn::warning().opacity(intensity * v)
            };
            div().flex_1().h(px(8.)).rounded(px(1.)).bg(color)
        }))
    }))
}

// JSON view
fn json_trace_view(steps: &[TraceStep]) -> impl gpui::IntoElement {
    let json_text = steps
        .iter()
        .map(|s| {
            let type_str = match s.step_type {
                TraceStepType::Trigger => "trigger",
                TraceStepType::Action => "action",
                TraceStepType::Condition => "condition",
                TraceStepType::Http => "http",
                TraceStepType::Db => "db",
            };
            format!(
                r#"  {{ "name": "{}", "type": "{}", "start": {}, "duration": {} }}"#,
                s.name, type_str, s.start_offset_ms, s.duration_ms
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    v_flex()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(shadcn::muted_foreground())
                        .child("Raw JSON"),
                )
                .child(Button::new("copy-json").ghost().xsmall().child("Copy")),
        )
        .child(
            div()
                .w_full()
                .p_4()
                .rounded(px(8.))
                .bg(shadcn::card())
                .border_1()
                .border_color(shadcn::border())
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::foreground())
                        .child(format!("[\n{}\n]", json_text)),
                ),
        )
}

/// Rich node detail panel: I/O, Metadata, Logs, Retries, Stack trace.
fn node_detail_panel(
    detail: NodeDetail,
    selected_node_state: gpui::Entity<Option<usize>>,
) -> impl gpui::IntoElement {
    let status_color = match detail.status {
        StepStatus::Ok => shadcn::success(),
        StepStatus::Warn => shadcn::warning(),
        StepStatus::Error => shadcn::destructive(),
    };
    let type_label = step_type_label(detail.step_type);

    v_flex()
        .mt_4()
        .gap_3()
        .rounded(px(8.))
        .border_1()
        .border_color(status_color)
        .bg(shadcn::card())
        .p_4()
        // Header with close
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(status_color))
                        .child(
                            div()
                                .px_2()
                                .py(px(1.))
                                .rounded(px(3.))
                                .bg(shadcn::muted())
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::foreground())
                                .child(type_label),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(shadcn::foreground())
                                .child(detail.name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(format!("node {} · {}ms", detail.id, detail.dur_ms)),
                        ),
                )
                .child(
                    div()
                        .id("close-node-detail")
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .text_xs()
                        .text_color(shadcn::muted_foreground())
                        .cursor_pointer()
                        .hover(|s| s.bg(shadcn::muted()).text_color(shadcn::foreground()))
                        .on_click(move |_, _, cx| {
                            selected_node_state.update(cx, |v, _| *v = None);
                        })
                        .child("✕"),
                ),
        )
        // I/O
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(shadcn::muted_foreground())
                        .child("INPUT / OUTPUT"),
                )
                .when_some(detail.input.as_ref(), |this, input| {
                    this.child(io_block("Input", input, shadcn::muted_foreground()))
                })
                .when_some(detail.output.as_ref(), |this, output| {
                    this.child(io_block("Output", output, shadcn::flow()))
                })
                .when(detail.output.is_none() && detail.error.is_some(), |this| {
                    this.child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(px(4.))
                            .bg(shadcn::destructive().opacity(0.12))
                            .border_1()
                            .border_color(shadcn::destructive())
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::destructive())
                                    .child(detail.error.unwrap_or_default()),
                            ),
                    )
                }),
        )
        // Metadata
        .when(!detail.meta.is_empty(), |this| {
            this.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(shadcn::muted_foreground())
                            .child("METADATA"),
                    )
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .children(detail.meta.iter().map(|(k, v)| {
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(shadcn::muted_foreground())
                                            .child(k.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_family(fonts::MONO)
                                            .text_color(shadcn::foreground())
                                            .child(v.clone()),
                                    )
                            })),
                    ),
            )
        })
        // Logs
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(shadcn::muted_foreground())
                        .child(format!("LOGS ({})", detail.logs.len())),
                )
                .child(v_flex().gap(px(2.)).children(detail.logs.iter().map(|log| {
                    let (color, level_str) = match log.level {
                        LogLevel::Info => (shadcn::muted_foreground(), "INFO"),
                        LogLevel::Warn => (shadcn::warning(), "WARN"),
                        LogLevel::Error => (shadcn::destructive(), "ERROR"),
                    };
                    h_flex()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .when(log.level == LogLevel::Error, |d| {
                            d.bg(shadcn::destructive().opacity(0.1))
                        })
                        .when(log.level == LogLevel::Warn, |d| {
                            d.bg(shadcn::warning().opacity(0.1))
                        })
                        .child(
                            div()
                                .w(px(40.))
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(format!("{}ms", log.t_ms)),
                        )
                        .child(
                            div()
                                .w(px(42.))
                                .text_xs()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color)
                                .child(level_str),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(shadcn::foreground())
                                .child(log.message.clone()),
                        )
                }))),
        )
        // Retries
        .when(!detail.retries.is_empty(), |this| {
            this.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(shadcn::muted_foreground())
                            .child(format!("RETRIES ({})", detail.retries.len())),
                    )
                    .children(detail.retries.iter().map(|r| {
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded(px(4.))
                            .bg(shadcn::destructive().opacity(0.08))
                            .border_1()
                            .border_color(shadcn::destructive().opacity(0.3))
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(shadcn::warning())
                                    .child(format!("Attempt {}", r.attempt)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(r.at.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(shadcn::destructive())
                                    .child(r.error.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(r.dur.clone()),
                            )
                    })),
            )
        })
        // Stack trace
        .when_some(detail.stack_trace.as_ref(), |this, trace| {
            this.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(shadcn::muted_foreground())
                            .child("STACK TRACE"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(px(4.))
                            .bg(shadcn::muted())
                            .text_xs()
                            .font_family(fonts::MONO)
                            .text_color(shadcn::foreground())
                            .child(trace.clone()),
                    ),
            )
        })
}

fn step_type_label(t: TraceStepType) -> &'static str {
    match t {
        TraceStepType::Trigger => "TRIGGER",
        TraceStepType::Http => "HTTP",
        TraceStepType::Db => "DB",
        TraceStepType::Condition => "CONDITION",
        TraceStepType::Action => "ACTION",
    }
}

fn step_details_panel(step: &TraceStep) -> impl gpui::IntoElement {
    v_flex()
        .mt_4()
        .p_4()
        .gap_3()
        .rounded(px(8.))
        .border_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(shadcn::muted_foreground())
                        .child(format!("{} — Input / Output", step.name)),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(Button::new("copy-input").ghost().xsmall().child("Copy"))
                        .child(Button::new("diff").ghost().xsmall().child("Diff")),
                ),
        )
        .when_some(step.input.as_ref(), |this, input| {
            this.child(io_block("Input", input, shadcn::muted_foreground()))
        })
        .when_some(step.output.as_ref(), |this, output| {
            this.child(io_block("Output", output, shadcn::flow()))
        })
}

fn io_block(label: &'static str, content: &str, accent: gpui::Rgba) -> impl gpui::IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .w_full()
                .px_3()
                .py_2()
                .rounded(px(4.))
                .bg(shadcn::muted())
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(accent)
                .overflow_x_scrollbar()
                .child(content.to_string()),
        )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RIGHT PANEL: Metadata, SLO, Percentiles, Workers, Sparkline, Related
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn right_panel(
    summary: &crate::data::MonitorSummary,
    details: Option<&ExecutionDetails>,
    selected_exec: Option<&MonitorExecution>,
    selected_id: &str,
) -> impl gpui::IntoElement {
    let slo_ok = summary.slo_current_pct >= summary.slo_target_pct;
    let (p50, _p90, p95, p99) = summary.percentiles_ms;
    let worker_list = workers();
    let related = selected_exec.map_or(vec![], |e| related_executions(selected_id, &e.workflow, 5));

    v_flex()
        .w(px(240.))
        .h_full()
        .flex_shrink_0()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .border_l_1()
        .border_color(shadcn::border())
        .bg(shadcn::background())
        // Metadata
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child("METADATA"),
                )
                .when_some(selected_exec, |this, e| {
                    this.child(meta_row("ID", &e.id, shadcn::flow()))
                        .child(meta_row("Workflow", &e.workflow, shadcn::foreground()))
                        .child(meta_row("Trigger", &e.trigger, shadcn::violet()))
                        .child(meta_row("Nodes", &e.nodes, shadcn::success()))
                        .child(meta_row("Input", &e.input_size, shadcn::muted_foreground()))
                        .child(meta_row(
                            "Output",
                            e.output_size.as_deref().unwrap_or("—"),
                            shadcn::muted_foreground(),
                        ))
                        .child(meta_row(
                            "Retries",
                            &e.retries.to_string(),
                            shadcn::muted_foreground(),
                        ))
                })
                .when_some(details, |this, d| {
                    this.child(meta_row("Tenant", &d.tenant, shadcn::muted_foreground()))
                }),
        )
        // SLO
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child("SLO · 30d"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if slo_ok {
                                    shadcn::success()
                                } else {
                                    shadcn::destructive()
                                })
                                .child(format!("{:.1}%", summary.slo_current_pct)),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded(px(999.))
                        .bg(shadcn::muted())
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(px((summary.slo_current_pct / 100.0 * 200.) as f32))
                                .rounded(px(999.))
                                .bg(if slo_ok {
                                    shadcn::success()
                                } else {
                                    shadcn::destructive()
                                }),
                        ),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(format!("target {:.1}%", summary.slo_target_pct)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(if slo_ok {
                                    shadcn::success()
                                } else {
                                    shadcn::destructive()
                                })
                                .child(if slo_ok {
                                    "✓ within budget"
                                } else {
                                    "↓ over budget"
                                }),
                        ),
                ),
        )
        // Percentiles
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child("PERCENTILES"),
                )
                .child(percentile_row("P50", p50, shadcn::success(), 25))
                .child(percentile_row(
                    "P90",
                    summary.percentiles_ms.1,
                    shadcn::warning(),
                    60,
                ))
                .child(percentile_row("P95", p95, shadcn::warning(), 85))
                .child(percentile_row("P99", p99, shadcn::destructive(), 100)),
        )
        // Workers
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child("WORKERS"),
                )
                .children(worker_list.iter().map(|w| {
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(
                            if w.status == "active" {
                                shadcn::success()
                            } else {
                                shadcn::muted()
                            },
                        ))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(w.id.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(format!("q{}", w.queue_len)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child(w.cpu.clone()),
                        )
                })),
        )
        // Success sparkline
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_color(shadcn::border())
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .child("SUCCESS · 20m"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(fonts::MONO)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(shadcn::success())
                                .child(format!("{}%", summary.success_rate_pct)),
                        ),
                )
                .child(sparkline_bar(
                    &summary.success_sparkline,
                    shadcn::success(),
                    200.,
                    24.,
                )),
        )
        // Related executions
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::muted_foreground())
                        .child(
                            selected_exec
                                .map(|e| format!("RELATED · {}", e.workflow.to_uppercase()))
                                .unwrap_or_else(|| "RELATED".into()),
                        ),
                )
                .children(related.iter().map(|r| {
                    let status_color = match r.status {
                        MonitorStatus::Running => shadcn::warning(),
                        MonitorStatus::Queued => shadcn::muted_foreground(),
                        MonitorStatus::Completed => shadcn::success(),
                        MonitorStatus::Failed => shadcn::destructive(),
                    };
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(status_color))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_family(fonts::MONO)
                                .text_color(shadcn::muted_foreground())
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(r.id.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(shadcn::muted_foreground())
                                .child(r.started.clone()),
                        )
                        .when_some(r.duration_ms, |this, ms| {
                            this.child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::MONO)
                                    .text_color(shadcn::muted_foreground())
                                    .child(format_duration(ms)),
                            )
                        })
                })),
        )
}

fn meta_row(label: &str, value: &str, value_color: gpui::Rgba) -> impl gpui::IntoElement {
    let label = label.to_string();
    let value = value.to_string();
    h_flex()
        .justify_between()
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(value_color)
                .child(value),
        )
}

fn percentile_row(
    label: &str,
    value_ms: u32,
    color: gpui::Rgba,
    width_pct: u32,
) -> impl gpui::IntoElement {
    let label = label.to_string();
    let value_str = if value_ms >= 1000 {
        format!("{:.1}s", value_ms as f32 / 1000.0)
    } else {
        format!("{}ms", value_ms)
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .w(px(28.))
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(shadcn::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .h(px(4.))
                .rounded(px(999.))
                .bg(shadcn::muted())
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(px(width_pct as f32 / 100.0 * 120.))
                        .rounded(px(999.))
                        .bg(color),
                ),
        )
        .child(
            div()
                .w(px(36.))
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(color)
                .text_right()
                .child(value_str),
        )
}

fn log_level_filters(
    log_filter: gpui::Entity<Option<LogLevel>>,
    cx: &mut gpui::Context<MonitorPage>,
) -> impl gpui::IntoElement {
    let current = *log_filter.read(cx);
    let lf_all = log_filter.clone();
    let lf_warn = log_filter.clone();
    let lf_error = log_filter.clone();

    h_flex()
        .gap_1()
        .child(log_chip("All", current.is_none(), move |_, _, cx| {
            lf_all.update(cx, |v, _| *v = None);
        }))
        .child(log_chip(
            "Warn",
            current == Some(LogLevel::Warn),
            move |_, _, cx| {
                lf_warn.update(cx, |v, _| *v = Some(LogLevel::Warn));
            },
        ))
        .child(log_chip(
            "Error",
            current == Some(LogLevel::Error),
            move |_, _, cx| {
                lf_error.update(cx, |v, _| *v = Some(LogLevel::Error));
            },
        ))
}

fn log_chip<F>(label: &'static str, active: bool, on_click: F) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    div()
        .id(label)
        .px_2()
        .py(px(3.))
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(shadcn::muted()).text_color(shadcn::foreground())
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(label)
}

fn log_chip_with_count<F>(
    label: &'static str,
    count: usize,
    active: bool,
    on_click: F,
) -> impl gpui::IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    let text = if count > 0 {
        format!("{} {}", label, count)
    } else {
        label.to_string()
    };
    div()
        .id((label, count))
        .px_2()
        .py(px(3.))
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(shadcn::muted()).text_color(shadcn::foreground())
        })
        .when(!active, |d| {
            d.text_color(shadcn::muted_foreground())
                .hover(|s| s.bg(shadcn::muted()))
        })
        .on_click(on_click)
        .child(text)
}

fn log_entry_row(entry: &LogEntry) -> impl gpui::IntoElement {
    let (level_color, level_text) = match entry.level {
        LogLevel::Info => (shadcn::flow(), "INFO"),
        LogLevel::Warn => (shadcn::warning(), "WARN"),
        LogLevel::Error => (shadcn::destructive(), "ERR"),
    };

    h_flex()
        .gap_2()
        .items_start()
        .px_2()
        .py_1()
        .rounded(px(4.))
        .hover(|s| s.bg(shadcn::muted()))
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(shadcn::muted_foreground())
                .child(format!("{}ms", entry.timestamp_ms)),
        )
        .child(
            div()
                .w(px(36.))
                .flex_shrink_0()
                .px_1()
                .rounded(px(3.))
                .bg(level_color.opacity(0.12))
                .text_xs()
                .font_family(fonts::MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(level_color)
                .text_center()
                .child(level_text),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(if entry.level == LogLevel::Error {
                    shadcn::destructive()
                } else if entry.level == LogLevel::Warn {
                    shadcn::warning()
                } else {
                    shadcn::foreground()
                })
                .child(entry.message.clone()),
        )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UTILITIES
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn format_duration(ms: u32) -> String {
    if ms >= 1000 {
        format!("{:.2}s", ms as f32 / 1000.0)
    } else {
        format!("{}ms", ms)
    }
}

trait RgbaExt {
    fn opacity(self, alpha: f32) -> gpui::Rgba;
}

impl RgbaExt for gpui::Rgba {
    fn opacity(self, alpha: f32) -> gpui::Rgba {
        gpui::Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a: alpha,
        }
    }
}
