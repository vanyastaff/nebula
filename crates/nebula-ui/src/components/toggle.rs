//! Toggle/Switch button component.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// Toggle button size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToggleSize {
    /// Small toggle
    Small,
    /// Medium toggle (default)
    #[default]
    Medium,
    /// Large toggle
    Large,
}

impl ToggleSize {
    fn dimensions(&self) -> (f32, f32, f32) {
        // (width, height, knob_size)
        match self {
            ToggleSize::Small => (32.0, 18.0, 14.0),
            ToggleSize::Medium => (44.0, 24.0, 20.0),
            ToggleSize::Large => (56.0, 30.0, 26.0),
        }
    }
}

/// Toggle/Switch component
///
/// # Example
///
/// ```rust,ignore
/// let mut enabled = false;
/// Toggle::new(&mut enabled)
///     .label("Dark mode")
///     .show(ui);
/// ```
pub struct Toggle<'a> {
    checked: &'a mut bool,
    label: Option<&'a str>,
    description: Option<&'a str>,
    size: ToggleSize,
    disabled: bool,
    label_left: bool,
}

impl<'a> Toggle<'a> {
    /// Create a new toggle
    pub fn new(checked: &'a mut bool) -> Self {
        Self {
            checked,
            label: None,
            description: None,
            size: ToggleSize::Medium,
            disabled: false,
            label_left: false,
        }
    }

    /// Set label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set description
    pub fn description(mut self, desc: &'a str) -> Self {
        self.description = Some(desc);
        self
    }

    /// Set size
    pub fn size(mut self, size: ToggleSize) -> Self {
        self.size = size;
        self
    }

    /// Small size
    pub fn small(mut self) -> Self {
        self.size = ToggleSize::Small;
        self
    }

    /// Large size
    pub fn large(mut self) -> Self {
        self.size = ToggleSize::Large;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Put label on left side
    pub fn label_left(mut self) -> Self {
        self.label_left = true;
        self
    }

    /// Show the toggle
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Toggle<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (width, height, knob_size) = self.size.dimensions();
        let padding = (height - knob_size) / 2.0;

        ui.horizontal(|ui| {
            // Label on left
            if self.label_left {
                if let Some(label) = self.label {
                    self.show_label(ui, label);
                }
            }

            // Toggle track
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(width, height),
                if self.disabled {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                },
            );

            if response.clicked() && !self.disabled {
                *self.checked = !*self.checked;
            }

            // Animation
            let animation_progress = ui
                .ctx()
                .animate_bool(response.id.with("toggle_anim"), *self.checked);

            // Colors
            let track_color = if self.disabled {
                tokens.muted
            } else if *self.checked {
                egui::Color32::from_rgba_unmultiplied(
                    tokens.primary.r(),
                    tokens.primary.g(),
                    tokens.primary.b(),
                    ((0.5 + animation_progress * 0.5) * 255.0) as u8,
                )
            } else if response.hovered() {
                tokens.muted
            } else {
                tokens.input
            };

            let knob_color = if self.disabled {
                tokens.muted_foreground
            } else {
                egui::Color32::WHITE
            };

            // Draw track
            let track_rect = rect;
            ui.painter()
                .rect_filled(track_rect, height / 2.0, track_color);

            // Border
            if !*self.checked {
                ui.painter().rect_stroke(
                    track_rect,
                    height / 2.0,
                    egui::Stroke::new(1.0, tokens.border),
                    egui::StrokeKind::Inside,
                );
            }

            // Draw knob
            let knob_x =
                rect.min.x + padding + animation_progress * (width - knob_size - padding * 2.0);
            let knob_center = egui::Pos2::new(knob_x + knob_size / 2.0, rect.center().y);

            // Knob shadow
            if !self.disabled {
                ui.painter().circle_filled(
                    egui::Pos2::new(knob_center.x, knob_center.y + 1.0),
                    knob_size / 2.0,
                    egui::Color32::from_black_alpha(30),
                );
            }

            // Knob
            ui.painter()
                .circle_filled(knob_center, knob_size / 2.0, knob_color);

            // Label on right
            if !self.label_left {
                if let Some(label) = self.label {
                    ui.add_space(tokens.spacing_sm);
                    self.show_label(ui, label);
                }
            }

            response
        })
        .inner
    }
}

impl<'a> Toggle<'a> {
    fn show_label(&self, ui: &mut Ui, label: &str) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let text_color = if self.disabled {
            tokens.muted_foreground
        } else {
            tokens.foreground
        };

        ui.vertical(|ui| {
            ui.label(
                RichText::new(label)
                    .size(tokens.font_size_sm)
                    .color(text_color),
            );

            if let Some(desc) = self.description {
                ui.label(
                    RichText::new(desc)
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                );
            }
        });
    }
}

/// Toggle group for multiple options
pub struct ToggleGroup<'a> {
    items: Vec<ToggleGroupItem<'a>>,
    vertical: bool,
}

/// Item in a toggle group
pub struct ToggleGroupItem<'a> {
    checked: &'a mut bool,
    label: &'a str,
    description: Option<&'a str>,
    disabled: bool,
}

impl<'a> ToggleGroupItem<'a> {
    /// Create a new toggle group item
    pub fn new(checked: &'a mut bool, label: &'a str) -> Self {
        Self {
            checked,
            label,
            description: None,
            disabled: false,
        }
    }

    /// Add description
    pub fn description(mut self, desc: &'a str) -> Self {
        self.description = Some(desc);
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

impl<'a> ToggleGroup<'a> {
    /// Create a new toggle group
    pub fn new(items: Vec<ToggleGroupItem<'a>>) -> Self {
        Self {
            items,
            vertical: true,
        }
    }

    /// Horizontal layout
    pub fn horizontal(mut self) -> Self {
        self.vertical = false;
        self
    }

    /// Show the toggle group
    pub fn show(self, ui: &mut Ui) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let layout = |ui: &mut Ui| {
            for item in self.items {
                Toggle::new(item.checked)
                    .label(item.label)
                    .description(item.description.unwrap_or(""))
                    .disabled(item.disabled)
                    .show(ui);

                if self.vertical {
                    ui.add_space(tokens.spacing_sm);
                }
            }
        };

        if self.vertical {
            ui.vertical(layout);
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = tokens.spacing_lg;
                layout(ui);
            });
        }
    }
}

/// Icon toggle button
pub struct IconToggle<'a> {
    checked: &'a mut bool,
    icon_on: &'a str,
    icon_off: &'a str,
    tooltip: Option<&'a str>,
    size: f32,
    disabled: bool,
}

impl<'a> IconToggle<'a> {
    /// Create a new icon toggle
    pub fn new(checked: &'a mut bool, icon_on: &'a str, icon_off: &'a str) -> Self {
        Self {
            checked,
            icon_on,
            icon_off,
            tooltip: None,
            size: 24.0,
            disabled: false,
        }
    }

    /// Set tooltip
    pub fn tooltip(mut self, tooltip: &'a str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }

    /// Set icon size
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Set disabled
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the icon toggle
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for IconToggle<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let icon = if *self.checked {
            self.icon_on
        } else {
            self.icon_off
        };

        let text_color = if self.disabled {
            tokens.muted_foreground
        } else if *self.checked {
            tokens.primary
        } else {
            tokens.foreground
        };

        let button = egui::Button::new(RichText::new(icon).size(self.size).color(text_color))
            .fill(egui::Color32::TRANSPARENT)
            .frame(false);

        let response = ui.add_enabled(!self.disabled, button);

        if response.clicked() {
            *self.checked = !*self.checked;
        }

        if let Some(tooltip) = self.tooltip {
            response.clone().on_hover_text(tooltip);
        }

        response
    }
}
