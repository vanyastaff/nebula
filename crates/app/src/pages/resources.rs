//! Resources page — resource health overview and table.

use gpui::prelude::*;
use gpui::{FontWeight, div, px};

use gpui_component::Sizable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::chart::LineChart;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::data::{Resource, ResourceHealth, ResourceScope, resource_summary, resources};
use crate::theme::{fonts, shadcn};

/// Resources page — list and health of connected resources.
pub struct ResourcesPage;

impl ResourcesPage {
    pub fn new() -> Self {
        Self
    }
}

impl gpui::Render for ResourcesPage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let summary = resource_summary();
        let items = resources();

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
                // Summary row: health cards + New Resource (no duplicate title — header shows "Resources")
                h_flex()
                    .justify_between()
                    .items_center()
                    .mb_5()
                    .child(div().flex().flex_wrap().gap(px(12.)).children([
                        health_summary_card("Healthy", summary.healthy, shadcn::success()),
                        health_summary_card("Degraded", summary.degraded, shadcn::warning()),
                        health_summary_card("Down", summary.down, shadcn::destructive()),
                    ]))
                    .child(
                        Button::new("new-resource")
                            .primary()
                            .small()
                            .child("New Resource"),
                    ),
            )
            .child(
                // Table — no duplicate header
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
                        div()
                            .w_full()
                            .overflow_x_scrollbar()
                            .min_w(px(0.))
                            .child(resources_table(items)),
                    ),
            )
    }
}

fn health_summary_card(
    label: impl gpui::IntoElement,
    count: u32,
    accent: gpui::Rgba,
) -> impl gpui::IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .rounded_lg()
        .border_1()
        .border_color(shadcn::border())
        .bg(shadcn::card())
        .child(div().w(px(3.)).h(px(12.)).rounded_full().bg(accent))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(accent)
                        .child(count.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(shadcn::muted_foreground())
                        .child(label),
                ),
        )
}

fn resources_table(rows: Vec<Resource>) -> impl gpui::IntoElement {
    let col_gap = px(12.);
    let header = h_flex()
        .w_full()
        .gap(col_gap)
        .py_3()
        .px_4()
        .border_b_1()
        .border_color(shadcn::border())
        .child(header_cell("HEALTH", px(90.)))
        .child(header_cell("NAME", px(140.)))
        .child(header_cell("TYPE", px(100.)))
        .child(header_cell("SCOPE", px(90.)))
        .child(header_cell("CONNECTIONS", px(100.)))
        .child(header_cell("LATENCY", px(70.)))
        .child(header_cell("UPTIME (%)", px(90.)))
        .child(header_cell("ACTIVITY", px(100.)))
        .child(header_cell("LAST CHECK", px(90.)))
        .child(
            div()
                .w(px(100.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(""),
        );

    let body = v_flex().children(rows.into_iter().map(|r| resource_row(r, col_gap)));

    v_flex().w_full().child(header).child(body)
}

fn header_cell(text: impl gpui::IntoElement, w: gpui::Pixels) -> impl gpui::IntoElement {
    div()
        .w(w)
        .flex_shrink_0()
        .text_xs()
        .text_color(shadcn::muted_foreground())
        .child(text)
}

fn resource_row(r: Resource, col_gap: gpui::Pixels) -> impl gpui::IntoElement {
    let (health_color, health_text) = match r.health {
        ResourceHealth::Healthy => (shadcn::success(), "healthy"),
        ResourceHealth::Degraded => (shadcn::warning(), "degraded"),
        ResourceHealth::Down => (shadcn::destructive(), "down"),
    };

    let scope_text = match r.scope {
        ResourceScope::Global => "Global",
        ResourceScope::Workflow => "Workflow",
        ResourceScope::Execution => "Execution",
    };

    let scope_color = match r.scope {
        ResourceScope::Global => shadcn::flow(),
        ResourceScope::Workflow => shadcn::warning(),
        ResourceScope::Execution => shadcn::violet(),
    };

    let latency_text = r
        .latency_ms
        .map(|v| format!("{}ms", format_float(v)))
        .unwrap_or_else(|| "—".into());

    let latency_color = match r.health {
        ResourceHealth::Healthy => shadcn::success(),
        ResourceHealth::Degraded => shadcn::warning(),
        ResourceHealth::Down => shadcn::destructive(),
    };

    let conn_pct = if r.connections.1 > 0 {
        (r.connections.0 as f32 / r.connections.1 as f32) * 100.0
    } else {
        0.0
    };

    h_flex()
        .w_full()
        .gap(col_gap)
        .py_3()
        .px_4()
        .border_b_1()
        .border_color(shadcn::border())
        .child(
            h_flex()
                .gap_2()
                .w(px(90.))
                .flex_shrink_0()
                .items_center()
                .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(health_color))
                .child(
                    div()
                        .text_xs()
                        .text_color(shadcn::foreground())
                        .child(health_text),
                ),
        )
        .child(
            div()
                .w(px(140.))
                .flex_shrink_0()
                .text_sm()
                .text_color(shadcn::foreground())
                .font_family(fonts::MONO)
                .truncate()
                .child(r.name),
        )
        .child(
            div().w(px(100.)).flex_shrink_0().child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(shadcn::muted())
                    .text_xs()
                    .text_color(shadcn::flow())
                    .child(r.resource_type),
            ),
        )
        .child(
            div().w(px(90.)).flex_shrink_0().child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(shadcn::muted())
                    .text_xs()
                    .text_color(scope_color)
                    .child(scope_text),
            ),
        )
        .child(
            v_flex()
                .w(px(100.))
                .flex_shrink_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_family(fonts::MONO)
                        .text_color(shadcn::foreground())
                        .child(if r.connections.1 > 0 {
                            format!("{}/{}", r.connections.0, r.connections.1)
                        } else {
                            "—".into()
                        }),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(4.))
                        .rounded_full()
                        .bg(shadcn::muted())
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(px(conn_pct.min(100.0)))
                                .bg(shadcn::success())
                                .rounded_full(),
                        ),
                ),
        )
        .child(
            div()
                .w(px(70.))
                .flex_shrink_0()
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(latency_color)
                .child(latency_text),
        )
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_xs()
                .font_family(fonts::MONO)
                .text_color(shadcn::foreground())
                .child(format!("{:.1}%", r.uptime_pct)),
        )
        .child(
            div()
                .w(px(100.))
                .flex_shrink_0()
                .h(px(36.))
                .child(activity_sparkline(&r.activity)),
        )
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_xs()
                .text_color(shadcn::muted_foreground())
                .child(r.last_check),
        )
        .child(Button::new("config").outline().xsmall().child("Config"))
}

fn activity_sparkline(data: &[f32]) -> impl gpui::IntoElement {
    if data.is_empty() {
        return div().h(px(36.)).into_any_element();
    }
    let points: Vec<(usize, f64)> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| (i, v as f64))
        .collect();
    div()
        .h(px(36.))
        .overflow_hidden()
        .child(
            LineChart::new(points)
                .x(|d| d.0.to_string())
                .y(|d| d.1)
                .stroke(shadcn::flow())
                .tick_margin(data.len().max(1)),
        )
        .into_any_element()
}

fn format_float(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{:.0}", v)
    } else {
        format!("{:.1}", v)
    }
}
