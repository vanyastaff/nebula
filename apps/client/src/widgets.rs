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
    /// Foreground and background of the tone.
    pub(crate) fn colors(self) -> (Color32, Color32) {
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
    egui::Button::new(RichText::new(label).color(theme::ON_ACCENT).strong()).fill(theme::ACCENT)
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
pub(crate) fn labeled_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    password: bool,
) -> egui::Response {
    ui.label(
        RichText::new(label)
            .color(theme::TEXT_MUTED)
            .size(theme::SIZE_BODY),
    );
    ui.add(field(value).password(password))
}

/// Enter was pressed in this field, which submits its form.
pub(crate) fn submitted(ui: &egui::Ui, field: &egui::Response) -> bool {
    field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// Picks one of a few options. The chosen option is drawn selected, so a mode switch reads as a control.
/// Options wrap on narrow windows instead of widening their container.
pub(crate) fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    options: &[(T, &str)],
) {
    ui.horizontal_wrapped(|ui| {
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

/// Rows at least this wide keep their actions at the right edge.
const ACTIONS_BESIDE_MIN: f32 = 560.0;

/// A row of information with its actions: the actions at the right edge where there is room, or on
/// a line of their own under the information on a narrow window, so a row never widens the page.
/// `actions` adds its buttons rightmost first.
pub(crate) fn row_with_actions<R>(
    ui: &mut egui::Ui,
    info: impl FnOnce(&mut egui::Ui),
    actions: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if ui.available_width() >= ACTIONS_BESIDE_MIN {
        ui.horizontal(|ui| {
            info(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions)
                .inner
        })
        .inner
    } else {
        ui.horizontal_wrapped(info);
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
            actions,
        )
        .inner
    }
}

/// The letter a mark shows for a name: its first letter or digit, upper-cased.
pub(crate) fn initial(name: &str) -> String {
    name.chars()
        .find(|character| character.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

/// Makes a drawn card one click target: anywhere on it activates it, the pointer shows it can be
/// clicked, it takes keyboard focus, and assistive technology reads it as a button named `label`.
pub(crate) fn card_clicked(card: &egui::Response, label: &str, enabled: bool) -> bool {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let mut response = card.interact(sense);
    if enabled {
        response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.clicked()
}

/// A rounded square with a letter, the shape of a node badge on the canvas. The brand uses it too.
pub(crate) fn mark(ui: &mut egui::Ui, letter: &str, color: Color32, side: f32) {
    let (square, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    ui.painter().rect_filled(square, theme::RADIUS_SM, color);
    ui.painter().text(
        square.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(side * 0.58),
        theme::ON_ACCENT,
    );
}

/// Tabs as words over a hairline; the open tab is underlined in the accent colour.
pub(crate) fn tabs<T: Copy + PartialEq>(ui: &mut egui::Ui, current: &mut T, options: &[(T, &str)]) {
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_LG;
        for &(value, label) in options {
            let active = *current == value;
            let color = if active {
                theme::TEXT
            } else {
                theme::TEXT_MUTED
            };
            let text = RichText::new(label).color(color);
            let response = ui.add(
                egui::Button::new(if active { text.strong() } else { text })
                    .frame(false)
                    .min_size(egui::vec2(0.0, 30.0)),
            );
            if active {
                let rect = response.rect;
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom() + 3.0,
                    Stroke::new(2.0, theme::ACCENT),
                );
            }
            if response.clicked() {
                *current = value;
            }
        }
    });
    let rect = row.response.rect;
    ui.painter().hline(
        ui.max_rect().x_range(),
        rect.bottom() + 4.0,
        Stroke::new(1.0, theme::BORDER),
    );
    ui.add_space(theme::SPACE_SM);
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
    ui.label(RichText::new(text).size(theme::SIZE_BRAND).strong());
}

/// A small frameless text button, as for a field's Fixed / Expression / Reset switch. The active one is
/// drawn in the accent colour.
pub(crate) fn link(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    let color = if active {
        theme::ACCENT
    } else {
        theme::TEXT_MUTED
    };
    ui.add(egui::Button::new(RichText::new(text).size(theme::SIZE_SMALL).color(color)).frame(false))
}

/// A labelled value in a read-only field, selectable, with a Copy button: an address, a secret
/// shown once, an identity.
pub(crate) fn copyable(ui: &mut egui::Ui, label: &str, value: &str) {
    caption(ui, label);
    ui.horizontal(|ui| {
        // A `&str` buffer is read-only: the value can be selected but not edited.
        let mut text = value;
        ui.add(
            egui::TextEdit::singleline(&mut text)
                .font(egui::TextStyle::Monospace)
                .desired_width((ui.available_width() - 70.0).max(80.0)),
        );
        if ui
            .button("Copy")
            .on_hover_text(format!("Copy the {}", label.to_lowercase()))
            .clicked()
        {
            ui.ctx().copy_text(value.to_owned());
        }
    });
}

/// A hairline across the width with a short word in its middle, such as "or" between two ways in.
pub(crate) fn divider_label(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SIZE_SMALL),
        theme::TEXT_MUTED,
    );
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), galley.size().y),
        egui::Sense::hover(),
    );
    let half = galley.size().x / 2.0 + theme::SPACE_SM;
    let stroke = Stroke::new(1.0, theme::BORDER);
    let y = rect.center().y;
    ui.painter()
        .hline(rect.left()..=rect.center().x - half, y, stroke);
    ui.painter()
        .hline(rect.center().x + half..=rect.right(), y, stroke);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::TEXT_MUTED,
    );
}

pub(crate) fn caption(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .color(theme::TEXT_MUTED)
            .size(theme::SIZE_BODY),
    );
}

pub(crate) fn badge(ui: &mut egui::Ui, text: impl Into<String>, tone: Tone) {
    const PADDING: egui::Vec2 = egui::vec2(8.0, 3.0);
    let (foreground, background) = tone.colors();
    let galley = egui::WidgetText::from(RichText::new(text.into()).size(theme::SIZE_BODY).strong())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        );
    // Sized to its text, so a tall row around it does not stretch it.
    let (rect, response) =
        ui.allocate_exact_size(galley.size() + 2.0 * PADDING, egui::Sense::hover());
    let label = galley.text().to_owned();
    ui.painter().rect_filled(rect, theme::RADIUS_SM, background);
    ui.painter().galley(rect.min + PADDING, galley, foreground);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
}

/// Full-width message for feedback or a warning that needs attention.
pub(crate) fn banner(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let (foreground, background) = tone.colors();
    let frame = egui::Frame::new()
        .fill(background)
        .stroke(Stroke::new(1.0, foreground.gamma_multiply(0.3)))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 8));
    // A banner spans its column and wraps long messages. The width is measured before the frame and
    // net of its margins and stroke: filling the inner width would grow a resizable panel every frame.
    let width = (ui.available_width() - frame.total_margin().sum().x).max(0.0);
    frame.show(ui, |ui| {
        ui.set_width(width);
        ui.colored_label(foreground, text);
    });
}

/// Title row of a side panel with Close at the right edge. Returns true when Close was clicked.
pub(crate) fn panel_header(ui: &mut egui::Ui, text: &str) -> bool {
    ui.horizontal(|ui| {
        section(ui, text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.button("Close").clicked()
        })
        .inner
    })
    .inner
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
