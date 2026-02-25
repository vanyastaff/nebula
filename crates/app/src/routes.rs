//! Route definitions for the application.

use gpui::{App, div, prelude::*};
use gpui_navigator::{Route, Transition, init_router};

use crate::pages::{HomePage, MonitorPage, ResourcesPage};

/// Configures all application routes.
pub fn configure_routes(cx: &mut App) {
    init_router(cx, |router| {
        router.add_route(
            Route::component("/", HomePage::new)
                .name("home")
                .transition(Transition::fade(200)),
        );
        router.add_route(
            Route::view("/workflows", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Workflows — coming soon")
                    .into_any_element()
            })
            .name("workflows")
            .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::view("/executions", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Executions — coming soon")
                    .into_any_element()
            })
            .name("executions")
            .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::component("/monitor", MonitorPage::new)
                .name("monitor")
                .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::component("/resources", ResourcesPage::new)
                .name("resources")
                .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::view("/credentials", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Credentials — coming soon")
                    .into_any_element()
            })
            .name("credentials")
            .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::view("/registry", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Registry — coming soon")
                    .into_any_element()
            })
            .name("registry")
            .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::view("/settings", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Settings — coming soon")
                    .into_any_element()
            })
            .name("settings")
            .transition(Transition::slide_left(250)),
        );
        router.add_route(
            Route::view("/account", || {
                div()
                    .flex()
                    .flex_1()
                    .p_4()
                    .child("Account — coming soon")
                    .into_any_element()
            })
            .name("account")
            .transition(Transition::slide_left(250)),
        );
    });
}
