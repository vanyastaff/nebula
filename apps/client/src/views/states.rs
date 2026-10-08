//! The states every page shares: data on its way, data that failed with a way to try again, and an
//! empty collection that says what to do next. Pages draw them through here so each looks the same
//! everywhere.

use crate::{
    theme,
    widgets::{self, Tone},
    workbench::Remote,
};
use eframe::egui::{self, RichText};

/// What a page draws for its data this frame.
pub(crate) enum Shown<'a, T> {
    /// The data, possibly while a fresh read is on its way; the header's activity shows that.
    Ready(&'a T),
    /// Nothing to draw yet; the loading state is already on screen.
    Waiting,
    /// The person asked to read the data again after a failure.
    Retry,
}

/// Draws `remote`'s loading or failure state and hands back its data when there is some. `what`
/// names the data for the messages, such as "executions".
pub(crate) fn show<'a, T>(ui: &mut egui::Ui, remote: &'a Remote<T>, what: &str) -> Shown<'a, T> {
    match remote {
        Remote::Ready(value) | Remote::Reloading(value) | Remote::Stale(value) => {
            Shown::Ready(value)
        },
        Remote::Idle | Remote::Loading => {
            loading(ui, &format!("Reading {what}…"));
            Shown::Waiting
        },
        Remote::Failed(reason) => {
            if failed(ui, &format!("The {what} could not be read."), reason) {
                Shown::Retry
            } else {
                Shown::Waiting
            }
        },
    }
}

pub(crate) fn loading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(theme::SPACE_MD);
    ui.horizontal(|ui| {
        ui.spinner();
        widgets::caption(ui, text);
    });
    ui.add_space(theme::SPACE_MD);
}

/// A failure with its reason and a Try again button. Returns true when it is pressed.
pub(crate) fn failed(ui: &mut egui::Ui, headline: &str, reason: &str) -> bool {
    ui.add_space(theme::SPACE_SM);
    widgets::banner(ui, Tone::Danger, &format!("{headline} {reason}"));
    ui.add_space(theme::SPACE_SM);
    ui.button("Try again")
        .on_hover_text("Read it from the server again")
        .clicked()
}

/// An empty collection: what it is, what to do, and the action that does it. Returns true when the
/// action is pressed.
pub(crate) fn empty(ui: &mut egui::Ui, headline: &str, body: &str, action: Option<&str>) -> bool {
    let mut pressed = false;
    ui.add_space(theme::SPACE_XL);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(headline).size(theme::SIZE_LABEL).strong());
        ui.add_space(theme::SPACE_XS);
        widgets::caption(ui, body);
        if let Some(action) = action {
            ui.add_space(theme::SPACE_SM);
            pressed = ui.add(widgets::primary_button(action)).clicked();
        }
    });
    ui.add_space(theme::SPACE_XL);
    pressed
}

/// The heading row of a page: its title and summary on the left, its actions on the right.
pub(crate) fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    summary: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            widgets::title(ui, title);
            widgets::caption(ui, summary);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
    ui.add_space(theme::SPACE_LG);
}
