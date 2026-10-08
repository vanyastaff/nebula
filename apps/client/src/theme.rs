//! Design tokens and the global egui style. Views take colors, spacing and type from here only.
use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};

// Surfaces: the canvas sits behind white cards; the sidebar and fields are a step away from white.
pub(crate) const CANVAS: Color32 = Color32::from_rgb(237, 241, 246);
pub(crate) const SURFACE: Color32 = Color32::WHITE;
pub(crate) const SIDEBAR: Color32 = Color32::from_rgb(247, 249, 252);
pub(crate) const FIELD: Color32 = Color32::from_rgb(243, 246, 251);
pub(crate) const BORDER: Color32 = Color32::from_rgb(215, 223, 234);

// Text
pub(crate) const TEXT: Color32 = Color32::from_rgb(31, 43, 61);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(83, 100, 123);

// Accent and status tones. Each tone has a strong foreground and a soft background for badges and banners.
pub(crate) const ACCENT: Color32 = Color32::from_rgb(35, 95, 198);
pub(crate) const ACCENT_SOFT: Color32 = Color32::from_rgb(232, 240, 252);
pub(crate) const SUCCESS: Color32 = Color32::from_rgb(27, 112, 80);
pub(crate) const SUCCESS_SOFT: Color32 = Color32::from_rgb(226, 244, 236);
pub(crate) const WARNING: Color32 = Color32::from_rgb(137, 84, 16);
pub(crate) const WARNING_SOFT: Color32 = Color32::from_rgb(252, 242, 224);
pub(crate) const DANGER: Color32 = Color32::from_rgb(161, 42, 38);
pub(crate) const DANGER_SOFT: Color32 = Color32::from_rgb(251, 232, 231);

// Graph: connection lines, and the badge colours that tell action kinds apart.
pub(crate) const EDGE: Color32 = Color32::from_rgb(150, 162, 180);
const NODE_ACCENTS: [Color32; 6] = [
    Color32::from_rgb(35, 95, 198),
    Color32::from_rgb(27, 112, 80),
    Color32::from_rgb(137, 84, 16),
    Color32::from_rgb(112, 74, 178),
    Color32::from_rgb(0, 122, 135),
    Color32::from_rgb(170, 70, 90),
];

/// A badge colour that stays the same for the same action key, so related nodes match at a glance.
pub(crate) fn node_accent(action_key: &str) -> Color32 {
    let hash = action_key.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(usize::from(byte))
    });
    NODE_ACCENTS[hash % NODE_ACCENTS.len()]
}

// Spacing scale in logical pixels, and corner radii.
pub(crate) const SPACE_XS: f32 = 4.0;
pub(crate) const SPACE_SM: f32 = 8.0;
pub(crate) const SPACE_MD: f32 = 12.0;
pub(crate) const SPACE_LG: f32 = 18.0;
pub(crate) const SPACE_XL: f32 = 28.0;
pub(crate) const RADIUS_SM: u8 = 6;
pub(crate) const RADIUS_MD: u8 = 10;

/// Layouts at least this wide show the navigator and runs as side panels.
pub(crate) const WIDE_LAYOUT_MIN: f32 = 760.0;

/// Frame for a top, side or bottom panel with a flat fill.
pub(crate) fn panel(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::same(SPACE_LG as i8))
}

/// Frame for the page behind the workbench content.
pub(crate) fn canvas() -> egui::Frame {
    panel(CANVAS)
}

/// A bordered white surface that groups related controls.
pub(crate) fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS_MD)
        .inner_margin(egui::Margin::same(SPACE_LG as i8))
}

/// A card that spans the full width of its parent. A frame otherwise shrinks to its content.
pub(crate) fn card_block(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add_contents(ui);
    });
}

pub(crate) fn install(context: &egui::Context) {
    context.set_fonts(fonts());
    context.set_global_style(style());
}

fn fonts() -> egui::FontDefinitions {
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
    fonts
}

fn style() -> egui::Style {
    let mut style = egui::Style {
        visuals: egui::Visuals::light(),
        ..Default::default()
    };
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(22.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(14.0));
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = SURFACE;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.text_edit_bg_color = Some(FIELD);
    style.visuals.selection.bg_fill = ACCENT_SOFT;
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    let radius = CornerRadius::same(RADIUS_SM);
    let resting = &mut style.visuals.widgets.inactive;
    resting.weak_bg_fill = SURFACE;
    resting.bg_fill = SURFACE;
    resting.bg_stroke = Stroke::new(1.0, BORDER);
    resting.fg_stroke = Stroke::new(1.0, TEXT);
    resting.corner_radius = radius;
    let hovered = &mut style.visuals.widgets.hovered;
    hovered.weak_bg_fill = ACCENT_SOFT;
    hovered.bg_fill = ACCENT_SOFT;
    hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    hovered.fg_stroke = Stroke::new(1.0, TEXT);
    hovered.corner_radius = radius;
    let active = &mut style.visuals.widgets.active;
    active.weak_bg_fill = ACCENT_SOFT;
    active.bg_fill = ACCENT_SOFT;
    active.bg_stroke = Stroke::new(1.0, ACCENT);
    active.fg_stroke = Stroke::new(1.0, TEXT);
    active.corner_radius = radius;
    style.spacing.item_spacing = egui::vec2(SPACE_MD, SPACE_MD);
    style.spacing.button_padding = egui::vec2(SPACE_MD, SPACE_SM - 1.0);
    style.spacing.interact_size.y = 34.0;
    style
}
