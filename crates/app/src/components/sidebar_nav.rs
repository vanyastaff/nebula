//! Sidebar navigation matching the design.

use gpui::prelude::*;
use gpui::{SharedString, div, px};

use gpui_component::Sizable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::sidebar::{SidebarMenu, SidebarMenuItem};
use gpui_component::v_flex;
use gpui_component::{Icon, IconName};
use gpui_navigator::Navigator;

use crate::theme::shadcn;

/// Navigation sidebar with full menu from design.
pub struct SidebarNav;

impl SidebarNav {
    pub fn new() -> Self {
        Self
    }
}

impl gpui::Render for SidebarNav {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let current_path = Navigator::current_path(cx);
        let collapsed_state = window.use_keyed_state("sidebar_collapsed", cx, |_, _| false);
        let collapsed = *collapsed_state.read(cx);

        let sidebar_width = if collapsed { px(64.) } else { px(255.) };

        v_flex()
            .id("sidebar")
            .w(sidebar_width)
            .flex_shrink_0()
            .h_full()
            .overflow_hidden()
            .bg(shadcn::background())
            .text_color(shadcn::foreground())
            .border_r_1()
            .border_color(shadcn::border())
            .child(
                div()
                    .id("header")
                    .flex_shrink_0()
                    .child(
                        div()
                            .h(px(56.))
                            .px_4()
                            .border_b_1()
                            .border_color(shadcn::border())
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(collapsed, |this| this.justify_center().flex_col())
                            .child(
                                Icon::new(IconName::Star)
                                    .size_6()
                                    .text_color(shadcn::primary()),
                            )
                            .when(!collapsed, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(shadcn::foreground())
                                        .child("NEBULA"),
                                )
                            })
                            .child(
                                Button::new("sidebar-collapse")
                                    .ghost()
                                    .xsmall()
                                    .icon(
                                        Icon::new(if collapsed {
                                            IconName::PanelLeftOpen
                                        } else {
                                            IconName::PanelLeftClose
                                        })
                                        .size_4()
                                        .text_color(shadcn::muted_foreground()),
                                    )
                                    .tooltip(if collapsed {
                                        "Expand sidebar"
                                    } else {
                                        "Collapse sidebar"
                                    })
                                    .on_click(move |_, _, cx| {
                                        collapsed_state.update(cx, |v, _| *v = !*v);
                                    }),
                            ),
                    )
                    .when(!collapsed, |this| {
                        this.child(
                            div()
                                .px_4()
                                .pt_4()
                                .pb_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(shadcn::muted_foreground())
                                        .child("WORKSPACE"),
                                )
                                .child(
                                    div()
                                        .mt_2()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .bg(shadcn::muted())
                                        .border_1()
                                        .border_color(shadcn::border())
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(shadcn::foreground())
                                                .child("Production"),
                                        )
                                        .child(
                                            Icon::new(IconName::ChevronDown)
                                                .size_4()
                                                .text_color(shadcn::muted_foreground()),
                                        ),
                                ),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("content")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p_3()
                    .text_color(shadcn::muted_foreground())
                    .child(
                        div().mt_4().child(
                            SidebarMenu::new()
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Dashboard"))
                                        .icon(Icon::new(IconName::LayoutDashboard).size_4())
                                        .active(current_path == "/")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Workflows"))
                                        .icon(Icon::new(IconName::Building2).size_4())
                                        .active(current_path == "/workflows")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/workflows".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Executions"))
                                        .icon(Icon::new(IconName::SquareTerminal).size_4())
                                        .active(current_path == "/executions")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/executions".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Monitor"))
                                        .icon(Icon::new(IconName::ChartPie).size_4())
                                        .active(current_path == "/monitor")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/monitor".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Resources"))
                                        .icon(Icon::new(IconName::File).size_4())
                                        .active(current_path == "/resources")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/resources".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Credentials"))
                                        .icon(Icon::new(IconName::CircleUser).size_4())
                                        .active(current_path == "/credentials")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/credentials".to_string())
                                        }),
                                )
                                .child(
                                    SidebarMenuItem::new(SharedString::from("Registry"))
                                        .icon(Icon::new(IconName::Inbox).size_4())
                                        .active(current_path == "/registry")
                                        .collapsed(collapsed)
                                        .on_click(move |_, _, cx| {
                                            Navigator::push(cx, "/registry".to_string())
                                        }),
                                ),
                        ),
                    ),
            )
            .child(
                div()
                    .id("sidebar-bottom")
                    .flex_shrink_0()
                    .p_3()
                    .border_t_1()
                    .border_color(shadcn::border())
                    .text_color(shadcn::muted_foreground())
                    .child(
                        SidebarMenu::new()
                            .child(
                                SidebarMenuItem::new(SharedString::from("Settings"))
                                    .icon(Icon::new(IconName::Settings).size_4())
                                    .active(current_path == "/settings")
                                    .collapsed(collapsed)
                                    .on_click(move |_, _, cx| {
                                        Navigator::push(cx, "/settings".to_string())
                                    }),
                            )
                            .child(
                                SidebarMenuItem::new(SharedString::from("Account"))
                                    .icon(Icon::new(IconName::User).size_4())
                                    .active(current_path == "/account")
                                    .collapsed(collapsed)
                                    .on_click(move |_, _, cx| {
                                        Navigator::push(cx, "/account".to_string())
                                    }),
                            ),
                    ),
            )
    }
}
