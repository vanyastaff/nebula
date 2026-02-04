//! Slider component for numeric input.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Widget};

/// Slider orientation
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliderOrientation {
    /// Horizontal slider (default)
    #[default]
    Horizontal,
    /// Vertical slider
    Vertical,
}

/// A styled slider component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Slider;
///
/// ui.add(Slider::new(&mut value, 0.0..=100.0).label("Volume"));
/// ```
pub struct Slider<'a> {
    value: &'a mut f64,
    range: std::ops::RangeInclusive<f64>,
    label: Option<&'a str>,
    show_value: bool,
    step: Option<f64>,
    orientation: SliderOrientation,
    disabled: bool,
}

impl<'a> Slider<'a> {
    /// Create a new slider
    pub fn new(value: &'a mut f64, range: std::ops::RangeInclusive<f64>) -> Self {
        Self {
            value,
            range,
            label: None,
            show_value: false,
            step: None,
            orientation: SliderOrientation::Horizontal,
            disabled: false,
        }
    }

    /// Set a label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Show the current value
    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Set step increment
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Set orientation
    pub fn orientation(mut self, orientation: SliderOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Make vertical
    pub fn vertical(mut self) -> Self {
        self.orientation = SliderOrientation::Vertical;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the slider
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Slider<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.vertical(|ui| {
            // Label and value row
            if self.label.is_some() || self.show_value {
                ui.horizontal(|ui| {
                    if let Some(label) = self.label {
                        ui.label(
                            RichText::new(label)
                                .size(tokens.font_size_sm)
                                .color(tokens.foreground),
                        );
                    }

                    if self.show_value {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let formatted = if self.step.map_or(false, |s| s >= 1.0) {
                                format!("{:.0}", self.value)
                            } else {
                                format!("{:.2}", self.value)
                            };
                            ui.label(
                                RichText::new(formatted)
                                    .size(tokens.font_size_sm)
                                    .color(tokens.muted_foreground),
                            );
                        });
                    }
                });
                ui.add_space(tokens.spacing_xs);
            }

            // Slider
            let mut slider = egui::Slider::new(self.value, self.range)
                .show_value(false)
                .trailing_fill(true);

            if let Some(step) = self.step {
                slider = slider.step_by(step);
            }

            if self.orientation == SliderOrientation::Vertical {
                slider = slider.vertical();
            }

            ui.add_enabled(!self.disabled, slider)
        })
        .inner
    }
}

/// A range slider for selecting a range of values
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::RangeSlider;
///
/// ui.add(RangeSlider::new(&mut min, &mut max, 0.0..=100.0));
/// ```
pub struct RangeSlider<'a> {
    min_value: &'a mut f64,
    max_value: &'a mut f64,
    range: std::ops::RangeInclusive<f64>,
    label: Option<&'a str>,
    show_value: bool,
    disabled: bool,
}

impl<'a> RangeSlider<'a> {
    /// Create a new range slider
    pub fn new(
        min_value: &'a mut f64,
        max_value: &'a mut f64,
        range: std::ops::RangeInclusive<f64>,
    ) -> Self {
        Self {
            min_value,
            max_value,
            range,
            label: None,
            show_value: false,
            disabled: false,
        }
    }

    /// Set a label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Show values
    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the range slider
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for RangeSlider<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.vertical(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(label)
                            .size(tokens.font_size_sm)
                            .color(tokens.foreground),
                    );

                    if self.show_value {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{:.0} - {:.0}",
                                    self.min_value, self.max_value
                                ))
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                            );
                        });
                    }
                });
                ui.add_space(tokens.spacing_xs);
            }

            // Two sliders for range
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !self.disabled,
                    egui::Slider::new(self.min_value, self.range.clone())
                        .show_value(false)
                        .trailing_fill(true),
                );
            });

            ui.horizontal(|ui| {
                ui.add_enabled(
                    !self.disabled,
                    egui::Slider::new(self.max_value, self.range.clone())
                        .show_value(false)
                        .trailing_fill(true),
                );
            });

            // Ensure min <= max
            if *self.min_value > *self.max_value {
                std::mem::swap(self.min_value, self.max_value);
            }
        })
        .response
    }
}
