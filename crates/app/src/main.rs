//! Nebula desktop application — workflow automation toolkit.
//!
//! Built with GPUI, gpui-component, and gpui-navigator.

mod components;
mod data;
mod pages;
mod routes;
mod theme;

use components::AppShell;
use gpui::{App, Application, Bounds, Hsla, WindowBounds, WindowOptions, prelude::*, px, size};
use gpui_component::{Colorize, Root, Theme, ThemeMode};
use gpui_component_assets::Assets;
use routes::configure_routes;

fn main() {
    let app = Application::new().with_assets(Assets);

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        configure_routes(cx);

        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Nebula");
                Theme::change(ThemeMode::Dark, Some(window), cx);

                // Nebula palette: base #080A0F, amber #F0A030 primary, cyan #00C8E0 flow
                // IBM Plex Mono (IDs/code) + IBM Plex Sans (UI)
                let theme = Theme::global_mut(cx);
                theme.font_family = "IBM Plex Sans".into();
                theme.mono_font_family = "IBM Plex Mono".into();

                if let Ok(c) = Hsla::parse_hex("#080A0F") {
                    theme.colors.background = c;
                    theme.colors.sidebar = c;
                }
                if let Ok(c) = Hsla::parse_hex("#E4E4E7") {
                    theme.colors.foreground = c;
                }
                if let Ok(c) = Hsla::parse_hex("#F0A030") {
                    theme.colors.primary = c;
                }
                if let Ok(c) = Hsla::parse_hex("#080A0F") {
                    theme.colors.primary_foreground = c;
                }
                if let Ok(c) = Hsla::parse_hex("#00C8E0") {
                    theme.colors.link = c;
                    theme.colors.sidebar_accent_foreground = c;
                }
                if let Ok(c) = Hsla::parse_hex("#71717a") {
                    theme.colors.sidebar_foreground = c;
                    theme.colors.muted_foreground = c;
                }
                if let Ok(c) = Hsla::parse_hex("#1a1f2e") {
                    theme.colors.sidebar_accent = c;
                    theme.colors.muted = c;
                }

                let view = cx.new(AppShell::new);
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .unwrap();

        cx.activate(true);
    });
}
