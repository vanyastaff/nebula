//! Color picker widget for ColorParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{Color32, CornerRadius, RichText, Stroke, TextEdit, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::ColorParameter;

/// Widget for color selection.
/// ```text
/// ┌─────────────────────────────────────┐
/// │ Label *                             │  <- Row 1: label
/// │ [■] [#FF5733] R:255 G:87 B:51       │  <- Row 2: swatch + hex + RGB
/// │ [Color picker expanded...]          │  <- Row 3: picker (optional)
/// │ Hint text                           │  <- Row 4: hint
/// └─────────────────────────────────────┘
/// ```
pub struct ColorWidget {
    parameter: ColorParameter,
    buffer: String,
    color: Color32,
    show_picker: bool,
}

impl ParameterWidget for ColorWidget {
    type Parameter = ColorParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let buffer = parameter
            .default
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "#000000".to_string());
        let color = parse_hex_color(&buffer).unwrap_or(Color32::BLACK);

        Self {
            parameter,
            buffer,
            color,
            show_picker: false,
        }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let mut response = WidgetResponse::default();

        let metadata = self.parameter.metadata();
        let name = metadata.name.clone();
        let required = metadata.required;
        let hint = metadata.hint.clone();

        let allow_alpha = self
            .parameter
            .options
            .as_ref()
            .is_some_and(|o| o.allow_alpha);

        let palette = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.palette.clone());

        // Outer Flex: vertical container (left-aligned)
        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Row 1: Label (left-aligned)
                flex.add_ui(item(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&name)
                                .size(theme.label_font_size)
                                .color(theme.label_color),
                        );
                        if required {
                            ui.label(
                                RichText::new("*")
                                    .size(theme.label_font_size)
                                    .color(theme.error),
                            );
                        }
                    });
                });

                // Row 2: Swatch + Hex input + RGB values
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .gap(egui::vec2(theme.spacing_sm, 0.0))
                        .show(ui, |row| {
                            // Color swatch
                            row.add_ui(item().basis(28.0), |ui| {
                                let (rect, swatch_response) = ui.allocate_exact_size(
                                    egui::vec2(24.0, 24.0),
                                    egui::Sense::click(),
                                );

                                if ui.is_rect_visible(rect) {
                                    // Checkerboard for alpha
                                    if allow_alpha && self.color.a() < 255 {
                                        let checker_size = 6.0;
                                        for row_idx in 0..4 {
                                            for col in 0..4 {
                                                let x = rect.min.x + col as f32 * checker_size;
                                                let y = rect.min.y + row_idx as f32 * checker_size;
                                                let checker_rect = egui::Rect::from_min_size(
                                                    egui::pos2(x, y),
                                                    egui::vec2(checker_size, checker_size),
                                                )
                                                .intersect(rect);

                                                let checker_color = if (row_idx + col) % 2 == 0 {
                                                    Color32::from_gray(200)
                                                } else {
                                                    Color32::from_gray(150)
                                                };
                                                ui.painter().rect_filled(
                                                    checker_rect,
                                                    0.0,
                                                    checker_color,
                                                );
                                            }
                                        }
                                    }

                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::same(3),
                                        self.color,
                                    );
                                    ui.painter().rect_stroke(
                                        rect,
                                        CornerRadius::same(3),
                                        Stroke::new(1.0, theme.input_border),
                                        egui::StrokeKind::Outside,
                                    );
                                }

                                if swatch_response.clicked() {
                                    self.show_picker = !self.show_picker;
                                }
                            });

                            // Hex input
                            row.add_ui(item().basis(90.0), |ui| {
                                let placeholder = if allow_alpha { "#RRGGBBAA" } else { "#RRGGBB" };
                                let text_edit = TextEdit::singleline(&mut self.buffer)
                                    .hint_text(placeholder)
                                    .desired_width(80.0)
                                    .font(egui::TextStyle::Monospace);

                                if ui.add(text_edit).changed() {
                                    if let Some(color) = parse_hex_color(&self.buffer) {
                                        self.color = color;
                                        response.changed = true;
                                    }
                                }
                            });

                            // RGB values
                            row.add_ui(item().grow(1.0), |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "R:{} G:{} B:{}",
                                        self.color.r(),
                                        self.color.g(),
                                        self.color.b()
                                    ))
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                                );
                            });
                        });
                });

                // Row 3: Color picker (when expanded)
                if self.show_picker {
                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::vertical()
                            .w_full()
                            .gap(egui::vec2(0.0, theme.spacing_xs))
                            .show(ui, |picker_flex| {
                                // Color picker
                                picker_flex.add_ui(item().grow(1.0), |ui| {
                                    let mut rgb = [
                                        self.color.r() as f32 / 255.0,
                                        self.color.g() as f32 / 255.0,
                                        self.color.b() as f32 / 255.0,
                                    ];

                                    let color_changed = if allow_alpha {
                                        let mut rgba =
                                            [rgb[0], rgb[1], rgb[2], self.color.a() as f32 / 255.0];
                                        let changed = ui
                                            .color_edit_button_rgba_unmultiplied(&mut rgba)
                                            .changed();
                                        if changed {
                                            self.color = Color32::from_rgba_unmultiplied(
                                                (rgba[0] * 255.0) as u8,
                                                (rgba[1] * 255.0) as u8,
                                                (rgba[2] * 255.0) as u8,
                                                (rgba[3] * 255.0) as u8,
                                            );
                                        }
                                        changed
                                    } else {
                                        let changed = ui.color_edit_button_rgb(&mut rgb).changed();
                                        if changed {
                                            self.color = Color32::from_rgb(
                                                (rgb[0] * 255.0) as u8,
                                                (rgb[1] * 255.0) as u8,
                                                (rgb[2] * 255.0) as u8,
                                            );
                                        }
                                        changed
                                    };

                                    if color_changed {
                                        self.buffer = color_to_hex(self.color, allow_alpha);
                                        response.changed = true;
                                    }
                                });

                                // Palette (if defined)
                                if let Some(ref palette) = palette {
                                    picker_flex.add_ui(item().grow(1.0), |ui| {
                                        ui.label(
                                            RichText::new("Palette")
                                                .size(theme.hint_font_size)
                                                .color(theme.hint_color),
                                        );
                                    });

                                    picker_flex.add_ui(item().grow(1.0), |ui| {
                                        Flex::horizontal()
                                            .wrap(true)
                                            .gap(egui::vec2(4.0, 4.0))
                                            .show(ui, |palette_row| {
                                                for color_str in palette {
                                                    if let Some(color) = parse_hex_color(color_str)
                                                    {
                                                        palette_row.add_ui(
                                                            item().basis(24.0),
                                                            |ui| {
                                                                let (rect, btn_response) = ui
                                                                    .allocate_exact_size(
                                                                        egui::vec2(20.0, 20.0),
                                                                        egui::Sense::click(),
                                                                    );

                                                                if ui.is_rect_visible(rect) {
                                                                    ui.painter().rect_filled(
                                                                        rect,
                                                                        CornerRadius::same(3),
                                                                        color,
                                                                    );

                                                                    let stroke = if self.buffer
                                                                        == *color_str
                                                                    {
                                                                        Stroke::new(
                                                                            2.0,
                                                                            theme.label_color,
                                                                        )
                                                                    } else {
                                                                        Stroke::new(
                                                                            1.0,
                                                                            theme.input_border,
                                                                        )
                                                                    };
                                                                    ui.painter().rect_stroke(
                                                                        rect,
                                                                        CornerRadius::same(3),
                                                                        stroke,
                                                                        egui::StrokeKind::Outside,
                                                                    );
                                                                }

                                                                if btn_response.clicked() {
                                                                    self.color = color;
                                                                    self.buffer = color_str.clone();
                                                                    response.changed = true;
                                                                }
                                                            },
                                                        );
                                                    }
                                                }
                                            });
                                    });
                                }
                            });
                    });
                }

                // Row 4: Hint
                if let Some(hint_text) = &hint {
                    if !hint_text.is_empty() {
                        flex.add_ui(item().grow(1.0), |ui| {
                            ui.label(
                                RichText::new(hint_text)
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                }

                // Error
                if let Some(ref error) = response.error {
                    flex.add_ui(item().grow(1.0), |ui| {
                        ui.label(
                            RichText::new(error)
                                .size(theme.hint_font_size)
                                .color(theme.error),
                        );
                    });
                }
            });

        response
    }
}

impl ColorWidget {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.buffer
    }

    #[must_use]
    pub fn color(&self) -> Color32 {
        self.color
    }

    pub fn set_hex(&mut self, hex: &str) {
        if let Some(color) = parse_hex_color(hex) {
            self.color = color;
            self.buffer = hex.to_string();
        }
    }

    pub fn set_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.color = Color32::from_rgb(r, g, b);
        self.buffer = format!("#{:02X}{:02X}{:02X}", r, g, b);
    }
}

fn parse_hex_color(hex: &str) -> Option<Color32> {
    let hex = hex.trim_start_matches('#');

    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some(Color32::from_rgb(r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color32::from_rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color32::from_rgba_unmultiplied(r, g, b, a))
        }
        _ => None,
    }
}

fn color_to_hex(color: Color32, include_alpha: bool) -> String {
    if include_alpha && color.a() != 255 {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            color.r(),
            color.g(),
            color.b(),
            color.a()
        )
    } else {
        format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
    }
}
