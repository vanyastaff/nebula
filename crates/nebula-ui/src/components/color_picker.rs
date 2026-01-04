//! Color picker component.

use crate::theme::current_theme;
use egui::{Color32, Response, Ui, Vec2, Widget};

/// A color picker component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::ColorPicker;
/// use egui::Color32;
///
/// let mut color = Color32::RED;
/// ui.add(ColorPicker::new(&mut color));
/// ```
pub struct ColorPicker<'a> {
    color: &'a mut Color32,
    alpha: bool,
    label: Option<&'a str>,
    size: ColorPickerSize,
}

/// Color picker size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorPickerSize {
    /// Small (24px)
    Sm,
    /// Medium (32px)
    #[default]
    Md,
    /// Large (40px)
    Lg,
}

impl ColorPickerSize {
    fn pixels(&self) -> f32 {
        match self {
            Self::Sm => 24.0,
            Self::Md => 32.0,
            Self::Lg => 40.0,
        }
    }
}

impl<'a> ColorPicker<'a> {
    /// Create a new color picker
    pub fn new(color: &'a mut Color32) -> Self {
        Self {
            color,
            alpha: false,
            label: None,
            size: ColorPickerSize::Md,
        }
    }

    /// Enable alpha channel editing
    pub fn with_alpha(mut self) -> Self {
        self.alpha = true;
        self
    }

    /// Set a label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the size
    pub fn size(mut self, size: ColorPickerSize) -> Self {
        self.size = size;
        self
    }

    /// Show the color picker
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for ColorPicker<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.horizontal(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.label(label);
                ui.add_space(tokens.spacing_sm);
            }

            // Color swatch button
            let size = self.size.pixels();
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());

            if ui.is_rect_visible(rect) {
                let painter = ui.painter();

                // Checkerboard pattern for alpha
                if self.color.a() < 255 {
                    let checker_size = size / 4.0;
                    for i in 0..4 {
                        for j in 0..4 {
                            let is_light = (i + j) % 2 == 0;
                            let color = if is_light {
                                Color32::from_gray(200)
                            } else {
                                Color32::from_gray(150)
                            };
                            let check_rect = egui::Rect::from_min_size(
                                egui::Pos2::new(
                                    rect.left() + i as f32 * checker_size,
                                    rect.top() + j as f32 * checker_size,
                                ),
                                Vec2::splat(checker_size),
                            );
                            painter.rect_filled(check_rect.intersect(rect), 0.0, color);
                        }
                    }
                }

                // Color fill
                painter.rect_filled(rect, tokens.rounding_sm(), *self.color);

                // Border
                painter.rect_stroke(
                    rect,
                    tokens.rounding_sm(),
                    egui::Stroke::new(1.0, tokens.border),
                    egui::StrokeKind::Inside,
                );
            }

            // Popup for color editing
            let popup_id = ui.make_persistent_id("color_picker_popup");

            if response.clicked() {
                ui.memory_mut(|mem| mem.toggle_popup(popup_id));
            }

            egui::popup_below_widget(
                ui,
                popup_id,
                &response,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_min_width(200.0);

                    if self.alpha {
                        egui::color_picker::color_edit_button_srgba(
                            ui,
                            self.color,
                            egui::color_picker::Alpha::OnlyBlend,
                        );
                    } else {
                        let mut rgb = [self.color.r(), self.color.g(), self.color.b()];
                        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                            *self.color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        }
                    }
                },
            );

            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            response
        })
        .inner
    }
}
