//! Shared controls. Views build buttons, fields, badges and banners through these, never ad hoc.
use crate::theme;
use eframe::egui::{self, Color32, RichText, Stroke};

/// Status colour family. Each tone pairs a strong foreground with a soft background.
#[derive(Clone, Copy)]
pub(crate) enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Tone {
    fn colors(self) -> (Color32, Color32) {
        match self {
            Self::Neutral => (theme::TEXT_MUTED, theme::FIELD),
            Self::Accent => (theme::ACCENT, theme::ACCENT_SOFT),
            Self::Success => (theme::SUCCESS, theme::SUCCESS_SOFT),
            Self::Warning => (theme::WARNING, theme::WARNING_SOFT),
            Self::Danger => (theme::DANGER, theme::DANGER_SOFT),
        }
    }
}

/// Main action of a view. At most one per visible group.
pub(crate) fn primary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).strong()).fill(theme::ACCENT)
}

/// Destructive action, such as discarding unsaved edits.
pub(crate) fn danger_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label).color(theme::DANGER)).fill(theme::DANGER_SOFT)
}

pub(crate) fn field(value: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value)
        .desired_width(f32::INFINITY)
        .margin(egui::Margin::symmetric(8, 8))
}

/// A field with its label above it. Password fields hide their contents.
pub(crate) fn labeled_field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    ui.label(RichText::new(label).color(theme::TEXT_MUTED).size(13.0));
    ui.add(field(value).password(password));
}

/// Picks one of a few options. The chosen option is drawn selected, so a mode switch reads as a control.
pub(crate) fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    options: &[(T, &str)],
) {
    ui.horizontal(|ui| {
        for &(value, label) in options {
            if ui
                .add(egui::Button::new(label).selected(*current == value))
                .clicked()
            {
                *current = value;
            }
        }
    });
}

/// Page-level heading.
pub(crate) fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(22.0).strong());
}

/// Large heading for a landing page, above the title scale. It shrinks on narrow windows so it does not
/// fill the first screen by itself.
pub(crate) fn headline(ui: &mut egui::Ui, text: &str) {
    let size = if ui.available_width() >= theme::WIDE_LAYOUT_MIN {
        30.0
    } else {
        24.0
    };
    ui.label(RichText::new(text).size(size).strong());
}

/// Heading of a group inside a view.
pub(crate) fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(17.0).strong());
}

pub(crate) fn caption(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .color(theme::TEXT_MUTED)
            .size(13.0),
    );
}

pub(crate) fn badge(ui: &mut egui::Ui, text: impl Into<String>, tone: Tone) {
    let (foreground, background) = tone.colors();
    egui::Frame::new()
        .fill(background)
        .corner_radius(theme::RADIUS_SM)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text.into())
                    .color(foreground)
                    .size(13.0)
                    .strong(),
            );
        });
}

/// Full-width message for feedback or a warning that needs attention.
pub(crate) fn banner(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let (foreground, background) = tone.colors();
    egui::Frame::new()
        .fill(background)
        .stroke(Stroke::new(1.0, foreground.gamma_multiply(0.3)))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            // Long messages wrap inside the available width instead of widening the page.
            ui.set_max_width(ui.available_width());
            ui.colored_label(foreground, text);
        });
}

/// Placeholder for a view that has nothing to show yet.
pub(crate) fn empty_state(ui: &mut egui::Ui, heading: &str, body: &str) {
    ui.add_space(theme::SPACE_XL);
    section(ui, heading);
    caption(ui, body);
}

/// Centers a column no wider than `max_width`. Narrower windows use the full width the page offers.
pub(crate) fn page_column(
    ui: &mut egui::Ui,
    max_width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let width = max_width.min(ui.available_width());
    let inset = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(inset);
        ui.vertical(|ui| {
            ui.set_width(width);
            add_contents(ui);
        });
    });
}
