//! Dashboard page — overview metrics, execution chart, workflow health, recent runs.
//!
//! Responsive layout matching n8n/Zapier-style workflow platform dashboards.

use gpui::prelude::*;
use gpui::{FontWeight, div, px};

use gpui_component::chart::BarChart;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::data::{
    ExecutionStatus, hourly_executions, metric_cards, recent_executions, workflow_health,
};
use crate::theme::{fonts, shadcn};

/// Main dashboard with overview metrics and execution history.
pub struct HomePage;

impl HomePage {
    pub fn new() -> Self {
        Self
    }
}

impl gpui::Render for HomePage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let cards = metric_cards();
        let hourly = hourly_executions();
        let health = workflow_health();
        let executions = recent_executions();

        v_flex()
            .w_full()
            .size_full()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_y_scrollbar()
            .px_6()
            .pt_5()
            .pb_6()
            .bg(shadcn::background())
            .child(
                // KPI cards
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(20.))
                    .mb_5()
                    .children(cards.into_iter().map(|c| metric_card(c, cx))),
            )
            .child(
                // Chart + Workflow health
                h_flex()
                    .gap(px(20.))
                    .flex_wrap()
                    .flex_1()
                    .min_h(px(0.))
                    .mb_5()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(280.))
                            .min_h(px(200.))
                            .gap_3()
                            .border_1()
                            .border_color(shadcn::border())
                            .rounded_lg()
                            .p_5()
                            .bg(shadcn::card())
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(shadcn::foreground())
                                            .child("Executions"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(shadcn::muted_foreground())
                                            .child("Last 12h"),
                                    ),
                            )
                            .child(
                                div().flex_1().min_h(px(160.)).child(
                                    BarChart::new(hourly)
                                        .x(|d| d.hour.clone())
                                        .y(|d| d.count as f64),
                                ),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(240.))
                            .max_w(px(300.))
                            .border_1()
                            .border_color(shadcn::border())
                            .rounded_lg()
                            .overflow_hidden()
                            .bg(shadcn::card())
                            .child(
                                div()
                                    .px_4()
                                    .py_3()
                                    .border_b_1()
                                    .border_color(shadcn::border())
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(shadcn::foreground())
                                            .child("Workflow health"),
                                    ),
                            )
                            .child(v_flex().children(health.into_iter().map(|w| {
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .px_4()
                                    .py_2()
                                    .child(
                                        div()
                                            .w(px(6.))
                                            .h(px(6.))
                                            .rounded_full()
                                            .flex_shrink_0()
                                            .bg(shadcn::success()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(shadcn::foreground())
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .child(w.name),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_family(fonts::MONO)
                                            .text_color(shadcn::muted_foreground())
                                            .flex_shrink_0()
                                            .child(format!("{:.1}%", w.success_rate)),
                                    )
                            }))),
                    ),
            )
            .child(
                // Recent executions — full width
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_3()
                    .border_1()
                    .border_color(shadcn::border())
                    .rounded_lg()
                    .p_5()
                    .bg(shadcn::card())
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(shadcn::foreground())
                                    .child("Recent executions"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(shadcn::flow())
                                    .cursor_pointer()
                                    .child("View all →"),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .overflow_x_scrollbar()
                            .min_w(px(0.))
                            .child(executions_table(executions)),
                    ),
            )
    }
}

fn metric_card(
    c: crate::data::MetricCard,
    _cx: &gpui::Context<HomePage>,
) -> impl gpui::IntoElement {
    v_flex()
        .flex_1()
        .flex_basis(px(160.))
        .min_w(px(140.))
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        .child(
            div()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(c.title),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(shadcn::foreground())
                .child(c.value),
        )
        .when_some(c.subtitle, |this, s| {
            this.child(
                div()
                    .text_xs()
                    .text_color(if c.subtitle_positive {
                        shadcn::success()
                    } else {
                        shadcn::muted_foreground()
                    })
                    .child(s),
            )
        })
        .when_some(c.trend, |this, t| {
            this.child(
                div()
                    .text_xs()
                    .text_color(if t.up {
                        shadcn::success()
                    } else {
                        shadcn::destructive()
                    })
                    .child(format!(
                        "{} {} {}",
                        if t.up { "▲" } else { "▼" },
                        t.value,
                        t.label
                    )),
            )
        })
}

fn executions_table(rows: Vec<crate::data::RecentExecution>) -> impl gpui::IntoElement {
    let col_gap = px(16.);
    let header = h_flex()
        .w_full()
        .gap(col_gap)
        .py_3()
        .px_2()
        .border_b_1()
        .border_color(shadcn::border())
        .child(
            div()
                .w(px(100.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("ID"),
        )
        .child(
            div()
                .w(px(140.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("WORKFLOW"),
        )
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("STATUS"),
        )
        .child(
            div()
                .w(px(60.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("PROGRESS"),
        )
        .child(
            div()
                .w(px(70.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("DURATION"),
        )
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("TRIGGER"),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(80.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child("STARTED"),
        );

    let body = v_flex().children(rows.into_iter().map(|r| {
        let status_color = match r.status {
            ExecutionStatus::Completed => shadcn::success(),
            ExecutionStatus::Failed => shadcn::destructive(),
            ExecutionStatus::Running => shadcn::primary(),
        };
        let status_text = match r.status {
            ExecutionStatus::Completed => "completed",
            ExecutionStatus::Failed => "failed",
            ExecutionStatus::Running => "running",
        };
        h_flex()
            .w_full()
            .gap(col_gap)
            .py_3()
            .px_2()
            .border_b_1()
            .border_color(shadcn::border())
            .child(
                div()
                    .w(px(100.))
                    .flex_shrink_0()
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::flow())
                    .cursor_pointer()
                    .child(r.id),
            )
            .child(
                div()
                    .w(px(140.))
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(shadcn::foreground())
                    .truncate()
                    .child(r.workflow),
            )
            .child(
                h_flex()
                    .gap_2()
                    .w(px(90.))
                    .flex_shrink_0()
                    .items_center()
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(status_color))
                    .child(
                        div()
                            .text_xs()
                            .text_color(shadcn::foreground())
                            .child(status_text),
                    ),
            )
            .child(
                div()
                    .w(px(60.))
                    .flex_shrink_0()
                    .text_xs()
                    .font_family(fonts::MONO)
                    .text_color(shadcn::muted_foreground())
                    .child(format!("{}/{}", r.progress.0, r.progress.1)),
            )
            .child(
                div()
                    .w(px(70.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(shadcn::muted_foreground())
                    .child(r.duration),
            )
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w_full()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(shadcn::muted())
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(shadcn::muted_foreground())
                                    .truncate()
                                    .child(r.trigger),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(80.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(shadcn::muted_foreground())
                    .child(r.started),
            )
    }));

    v_flex().w_full().child(header).child(body)
}
