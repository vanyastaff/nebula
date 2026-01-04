//! Checkbox and Switch components.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// A themed checkbox
///
/// # Example
///
/// ```rust,ignore
/// let mut enabled = false;
/// Checkbox::new(&mut enabled, "Enable feature").show(ui);
/// ```
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    label: &'a str,
    disabled: bool,
    indeterminate: bool,
}

impl<'a> Checkbox<'a> {
    /// Create a new checkbox
    pub fn new(checked: &'a mut bool, label: &'a str) -> Self {
        Self {
            checked,
            label,
            disabled: false,
            indeterminate: false,
        }
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set indeterminate state (shows dash instead of check)
    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    /// Show the checkbox
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Checkbox<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let size = 18.0;
        let spacing = tokens.spacing_sm;

        // Calculate label color
        let label_color = if self.disabled {
            tokens.muted_foreground
        } else {
            tokens.foreground
        };

        // Calculate total width needed
        let text_galley = ui.painter().layout_no_wrap(
            self.label.to_string(),
            egui::FontId::proportional(tokens.font_size_md),
            label_color,
        );
        let text_width = if self.label.is_empty() {
            0.0
        } else {
            text_galley.size().x + spacing
        };
        let total_width = size + text_width;

        let (total_rect, response) = ui.allocate_exact_size(
            Vec2::new(total_width, size.max(text_galley.size().y)),
            if self.disabled {
                egui::Sense::hover()
            } else {
                egui::Sense::click()
            },
        );

        if response.clicked() && !self.disabled {
            *self.checked = !*self.checked;
        }

        if ui.is_rect_visible(total_rect) {
            let painter = ui.painter();
            let rounding = tokens.radius_sm;

            // Checkbox rect
            let checkbox_rect = egui::Rect::from_min_size(total_rect.min, Vec2::splat(size));

            // Background and border
            let (bg, border_color) = if *self.checked {
                (tokens.primary, tokens.primary)
            } else {
                (
                    if self.disabled {
                        tokens.muted
                    } else {
                        tokens.background
                    },
                    tokens.border,
                )
            };

            painter.rect(
                checkbox_rect,
                rounding,
                bg,
                egui::Stroke::new(1.5, border_color),
                egui::StrokeKind::Outside,
            );

            // Checkmark or indeterminate dash
            if *self.checked || self.indeterminate {
                let color = tokens.primary_foreground;
                let center = checkbox_rect.center();

                if self.indeterminate {
                    // Dash for indeterminate
                    let dash_width = size * 0.5;
                    painter.line_segment(
                        [
                            egui::Pos2::new(center.x - dash_width / 2.0, center.y),
                            egui::Pos2::new(center.x + dash_width / 2.0, center.y),
                        ],
                        egui::Stroke::new(2.0, color),
                    );
                } else {
                    // Checkmark
                    let scale = size / 18.0;
                    let points = [
                        egui::Pos2::new(checkbox_rect.left() + 4.0 * scale, center.y),
                        egui::Pos2::new(
                            checkbox_rect.left() + 7.5 * scale,
                            checkbox_rect.bottom() - 4.5 * scale,
                        ),
                        egui::Pos2::new(
                            checkbox_rect.right() - 4.0 * scale,
                            checkbox_rect.top() + 5.0 * scale,
                        ),
                    ];
                    painter.line_segment([points[0], points[1]], egui::Stroke::new(2.0, color));
                    painter.line_segment([points[1], points[2]], egui::Stroke::new(2.0, color));
                }
            }

            // Label
            if !self.label.is_empty() {
                let text_pos = egui::Pos2::new(
                    checkbox_rect.right() + spacing,
                    total_rect.center().y - text_galley.size().y / 2.0,
                );
                painter.text(
                    text_pos,
                    egui::Align2::LEFT_TOP,
                    self.label,
                    egui::FontId::proportional(tokens.font_size_md),
                    label_color,
                );
            }
        }

        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
    }
}

/// A toggle switch component
///
/// # Example
///
/// ```rust,ignore
/// let mut dark_mode = true;
/// Switch::new(&mut dark_mode)
///     .label("Dark mode")
///     .show(ui);
/// ```
pub struct Switch<'a> {
    on: &'a mut bool,
    label: Option<&'a str>,
    label_right: bool,
    disabled: bool,
    size: SwitchSize,
}

/// Switch size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchSize {
    /// Small switch
    Sm,
    /// Default size
    #[default]
    Md,
    /// Large switch
    Lg,
}

impl<'a> Switch<'a> {
    /// Create a new switch
    pub fn new(on: &'a mut bool) -> Self {
        Self {
            on,
            label: None,
            label_right: true,
            disabled: false,
            size: SwitchSize::Md,
        }
    }

    /// Set label text
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Put label on the left side
    pub fn label_left(mut self) -> Self {
        self.label_right = false;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set size
    pub fn size(mut self, size: SwitchSize) -> Self {
        self.size = size;
        self
    }

    /// Small size
    pub fn small(mut self) -> Self {
        self.size = SwitchSize::Sm;
        self
    }

    /// Show the switch
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Switch<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (width, height, knob_size) = match self.size {
            SwitchSize::Sm => (32.0, 18.0, 14.0),
            SwitchSize::Md => (44.0, 24.0, 20.0),
            SwitchSize::Lg => (56.0, 30.0, 26.0),
        };

        let padding = (height - knob_size) / 2.0;

        ui.horizontal(|ui| {
            // Label left
            if let Some(label) = self.label {
                if !self.label_right {
                    ui.label(RichText::new(label).size(tokens.font_size_md).color(
                        if self.disabled {
                            tokens.muted_foreground
                        } else {
                            tokens.foreground
                        },
                    ));
                    ui.add_space(tokens.spacing_sm);
                }
            }

            // Switch track
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(width, height),
                if self.disabled {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                },
            );

            if response.clicked() && !self.disabled {
                *self.on = !*self.on;
            }

            // Draw track
            let track_color = if *self.on {
                if self.disabled {
                    crate::theme::color_mix(tokens.primary, tokens.muted, 0.5)
                } else {
                    tokens.primary
                }
            } else {
                if self.disabled {
                    tokens.muted
                } else {
                    tokens.secondary
                }
            };

            ui.painter().rect_filled(rect, height / 2.0, track_color);

            // Draw knob
            let knob_x = if *self.on {
                rect.right() - padding - knob_size / 2.0
            } else {
                rect.left() + padding + knob_size / 2.0
            };

            let knob_color = if self.disabled {
                tokens.muted_foreground
            } else {
                tokens.primary_foreground
            };

            ui.painter().circle_filled(
                egui::Pos2::new(knob_x, rect.center().y),
                knob_size / 2.0,
                knob_color,
            );

            // Hover effect
            if response.hovered() && !self.disabled {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            // Label right
            if let Some(label) = self.label {
                if self.label_right {
                    ui.add_space(tokens.spacing_sm);
                    ui.label(RichText::new(label).size(tokens.font_size_md).color(
                        if self.disabled {
                            tokens.muted_foreground
                        } else {
                            tokens.foreground
                        },
                    ));
                }
            }

            response
        })
        .inner
    }
}

/// Radio button group
pub struct RadioGroup<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: Vec<(T, &'a str)>,
    disabled: bool,
    horizontal: bool,
}

impl<'a, T: PartialEq + Clone> RadioGroup<'a, T> {
    /// Create a new radio group
    pub fn new(value: &'a mut T, options: Vec<(T, &'a str)>) -> Self {
        Self {
            value,
            options,
            disabled: false,
            horizontal: false,
        }
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Layout horizontally
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Show the radio group
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut add_options = |ui: &mut Ui| {
            let mut changed = false;

            for (option_value, label) in &self.options {
                let selected = self.value == option_value;

                let response =
                    ui.add_enabled(!self.disabled, egui::RadioButton::new(selected, *label));

                if response.clicked() {
                    *self.value = option_value.clone();
                    changed = true;
                }

                if response.hovered() && !self.disabled {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }

            changed
        };

        if self.horizontal {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = tokens.spacing_lg;
                add_options(ui);
            })
            .response
        } else {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = tokens.spacing_sm;
                add_options(ui);
            })
            .response
        }
    }
}
