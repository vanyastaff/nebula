//! Workbench surfaces and readable type hierarchy.
use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

pub(crate) const BACKGROUND: Color32 = Color32::from_rgb(237, 241, 246);
pub(crate) const NAVIGATION: Color32 = Color32::from_rgb(247, 249, 252);
pub(crate) const TEXT: Color32 = Color32::from_rgb(31, 43, 61);
pub(crate) const MUTED: Color32 = Color32::from_rgb(83, 100, 123);
pub(crate) const BORDER: Color32 = Color32::from_rgb(215, 223, 234);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(35, 95, 198);
pub(crate) const ERROR: Color32 = Color32::from_rgb(161, 42, 38);
pub(crate) const WARNING: Color32 = Color32::from_rgb(137, 84, 16);
pub(crate) const SUCCESS: Color32 = Color32::from_rgb(27, 112, 80);

pub(crate) fn install(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/Inter.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    context.set_fonts(fonts);
    let mut style = egui::Style {
        visuals: egui::Visuals::light(),
        ..Default::default()
    };
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(16.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(15.0));
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = Color32::WHITE;
    style.visuals.window_fill = Color32::WHITE;
    style.visuals.extreme_bg_color = NAVIGATION;
    style.visuals.text_edit_bg_color = Some(Color32::from_rgb(239, 243, 249));
    style.visuals.selection.bg_fill = Color32::from_rgb(220, 232, 252);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.inactive.bg_fill = NAVIGATION;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(231, 239, 251);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    context.set_global_style(style);
}

pub(crate) fn panel(fill: Color32) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(18)
}

pub(crate) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(label).color(Color32::WHITE)).fill(ACCENT)
}

pub(crate) fn field(value: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value)
        .desired_width(f32::INFINITY)
        .margin(egui::Margin::symmetric(8, 8))
}

pub(crate) fn caption(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).color(MUTED).size(13.0));
}
