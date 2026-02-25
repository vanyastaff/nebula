//! Top header bar with breadcrumbs, status, and controls.

use gpui::prelude::*;
use gpui::{div, px, FontWeight};

use gpui_component::{button::*, h_flex};

use crate::theme::shadcn;

/// App header with breadcrumbs, LIVE tag, status pills, search, and user.
pub struct AppHeader;

impl AppHeader {
    pub fn new() -> Self {
        Self
    }
}

impl gpui::Render for AppHeader {
    fn render(&mut self, _: &mut gpui::Window, _cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        h_flex()
            .w_full()
            .h(px(56.))
            .px_4()
            .gap_4()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(shadcn::border())
            .bg(shadcn::background())
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(shadcn::muted_foreground())
                                    .child("nebula"),
                            )
                            .child(div().text_sm().text_color(shadcn::muted_foreground()).child(">"))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(shadcn::foreground())
                                    .child("Production"),
                            )
                            .child(div().text_sm().text_color(shadcn::muted_foreground()).child(">"))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(shadcn::foreground())
                                    .child("Dashboard"),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(shadcn::success())
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(shadcn::primary_foreground())
                                    .child("LIVE"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(shadcn::success())
                            .child("3 running"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(shadcn::muted())
                            .border_1()
                            .border_color(shadcn::input())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(shadcn::muted_foreground())
                                    .child("Search..."),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(shadcn::success())
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(shadcn::primary_foreground())
                                    .child("3 Running"),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(shadcn::warning())
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(shadcn::primary_foreground())
                                    .child("2 Warning"),
                            ),
                    )
                    .child(
                        Button::new("new-workflow")
                            .primary()
                            .label("New Workflow")
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        div()
                            .w(px(32.))
                            .h(px(32.))
                            .rounded_full()
                            .bg(shadcn::violet())
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(shadcn::primary_foreground())
                                    .child("V"),
                            ),
                    ),
            )
    }
}
