//! Main application shell — layout with sidebar, content, and footer.

use gpui::{Entity, div, prelude::*, px};
use gpui_component::{h_flex, v_flex};
use gpui_navigator::{Navigator, router_view};

use super::SidebarNav;
use crate::theme::shadcn;

fn page_title_from_path(path: &str) -> &'static str {
    match path {
        "/" => "Dashboard",
        "/workflows" => "Workflows",
        "/executions" => "Executions",
        "/monitor" => "Monitor",
        "/resources" => "Resources",
        "/credentials" => "Credentials",
        "/registry" => "Registry",
        "/settings" => "Settings",
        "/account" => "Account",
        _ => "Page",
    }
}

fn footer_badge(label: &str, value: &str, healthy: bool) -> impl gpui::IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().w(px(4.)).h(px(4.)).rounded_full().bg(if healthy {
            shadcn::success()
        } else {
            shadcn::destructive()
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(shadcn::muted_foreground())
                        .child(format!("{label}:")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(shadcn::muted_foreground())
                        .child(value.to_string()),
                ),
        )
}

/// Root layout: sidebar + content + footer.
pub struct AppShell {
    sidebar: Entity<SidebarNav>,
}

impl AppShell {
    pub fn new(cx: &mut gpui::Context<Self>) -> Self {
        Self {
            sidebar: cx.new(|_| SidebarNav::new()),
        }
    }
}

impl gpui::Render for AppShell {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .id("content")
                    .flex_1()
                    .size_full()
                    .min_h(px(0.))
                    .child(self.sidebar.clone())
                    .child(
                        v_flex()
                            .flex_1()
                            .size_full()
                            .min_h(px(0.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("content-header")
                                    .flex_shrink_0()
                                    .w_full()
                                    .h(px(56.))
                                    .px_6()
                                    .border_b_1()
                                    .border_color(shadcn::border())
                                    .bg(shadcn::background())
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(shadcn::foreground())
                                            .child(page_title_from_path(&Navigator::current_path(
                                                cx,
                                            ))),
                                    )
                                    .child(div().id("content-controls")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .size_full()
                                    .min_h(px(0.))
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .w_full()
                                            .h_full()
                                            .min_h(px(0.))
                                            .min_w(px(0.))
                                            .child(router_view(window, cx)),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .id("app-footer")
                    .flex_shrink_0()
                    .w_full()
                    .px_4()
                    .py_1()
                    .border_t_1()
                    .border_color(shadcn::border())
                    .bg(shadcn::background())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(footer_badge("Database", "healthy", true))
                    .child(
                        div()
                            .w(px(1.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(shadcn::border()),
                    )
                    .child(footer_badge("Workers", "4/4", true))
                    .child(
                        div()
                            .w(px(1.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(shadcn::border()),
                    )
                    .child(footer_badge("Cluster", "1 node", true))
                    .child(
                        div()
                            .w(px(1.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(shadcn::border()),
                    )
                    .child(footer_badge("Uptime", "99.2%", true)),
            )
    }
}
