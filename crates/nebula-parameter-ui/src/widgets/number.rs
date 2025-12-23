//! Number input widget for NumberParameter with multiple display modes.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{Align, DragValue, RichText, Slider, TextEdit, Ui, Widget};
use egui_flex::{Flex, FlexAlign, FlexAlignContent, item};
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::{NumberDisplayMode, NumberParameter};

/// Widget for numeric input with configurable display modes.
pub struct NumberWidget {
    parameter: NumberParameter,
    value: f64,
    /// Text buffer for Text and SliderText modes
    text_buffer: String,
    /// Track if the input is currently focused.
    focused: bool,
}

impl ParameterWidget for NumberWidget {
    type Parameter = NumberParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let value = parameter.get().copied().unwrap_or(0.0);
        let text_buffer = format_value(value, parameter.get_precision());
        Self {
            parameter,
            value,
            text_buffer,
            focused: false,
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
        let hint = metadata.hint.clone();
        let required = metadata.required;
        let display_mode = self.parameter.get_display_mode();

        // Outer Flex: vertical container for the entire widget (left-aligned)
        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Render based on display mode
                match display_mode {
                    NumberDisplayMode::Text => {
                        self.show_text_mode(flex, theme, &name, required, &mut response);
                    }
                    NumberDisplayMode::Drag => {
                        self.show_drag_mode(flex, theme, &name, required, &mut response);
                    }
                    NumberDisplayMode::Slider => {
                        self.show_slider_mode(flex, theme, &name, required, &mut response);
                    }
                    NumberDisplayMode::SliderText => {
                        self.show_slider_text_mode(flex, theme, &name, required, &mut response);
                    }
                }

                // Hint (description) - as a flex item
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

                // Error - as a flex item
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

        // Update parameter on change
        if response.changed {
            if let Err(e) = self.parameter.set(self.value) {
                response.error = Some(e.to_string());
                response.changed = false;
            }
        }

        response
    }
}

impl NumberWidget {
    /// Text mode: Full-width centered input
    /// ```text
    /// ┌─────────────────────────────────────┐
    /// │ Label *                    0 – 1000 │  <- Row 1: label + range
    /// │ $ [        0.00        ] suffix     │  <- Row 2: prefix + input + suffix
    /// └─────────────────────────────────────┘
    /// ```
    fn show_text_mode(
        &mut self,
        flex: &mut egui_flex::FlexInstance,
        theme: &ParameterTheme,
        name: &str,
        required: bool,
        response: &mut WidgetResponse,
    ) {
        let prefix = self.parameter.get_prefix().map(|s| s.to_string());
        let suffix = self.parameter.get_suffix().map(|s| s.to_string());
        let min = self.parameter.get_min().unwrap_or(f64::MIN);
        let max = self.parameter.get_max().unwrap_or(f64::MAX);
        let precision = self.parameter.get_precision();
        let allow_negative = min < 0.0;
        let allow_decimal = precision.map_or(true, |p| p > 0);
        let has_range = self.parameter.get_min().is_some() || self.parameter.get_max().is_some();

        // Row 1: Label ... Range (nested horizontal Flex)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .show(ui, |row| {
                    // Left group: Label + required marker (bold)
                    row.add_ui(item(), |ui| {
                        Flex::horizontal()
                            .align_items(FlexAlign::Center)
                            .gap(egui::vec2(2.0, 0.0))
                            .show(ui, |label_group| {
                                label_group.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(name)
                                            .size(theme.label_font_size)
                                            .color(theme.label_color)
                                            .strong(),
                                    );
                                });
                                if required {
                                    label_group.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new("*")
                                                .size(theme.label_font_size)
                                                .color(theme.error),
                                        );
                                    });
                                }
                            });
                    });

                    // Spacer - grows to push range to the right
                    row.add_ui(item().grow(1.0), |_ui| {});

                    // Right: Range hint
                    if has_range {
                        row.add_ui(item(), |ui| {
                            let range_text =
                                format_range(self.parameter.get_min(), self.parameter.get_max());
                            ui.label(
                                RichText::new(range_text)
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                });
        });

        // Row 2: [Prefix] [Input] [Suffix] (nested horizontal Flex)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .align_content(FlexAlignContent::Stretch)
                .gap(egui::vec2(theme.spacing_xs, 0.0))
                .show(ui, |row| {
                    // Prefix (fixed width based on content)
                    if let Some(ref pfx) = prefix {
                        let prefix_width = pfx.len() as f32 * 8.0 + theme.spacing_xs;
                        row.add_ui(item().basis(prefix_width), |ui| {
                            ui.label(
                                RichText::new(pfx)
                                    .size(theme.input_font_size)
                                    .color(theme.label_color),
                            );
                        });
                    }

                    // Text input with styled frame - GROWS to fill all remaining space
                    row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                        let width = ui.available_width();
                        let has_error = response.error.is_some();

                        // Apply consistent input frame styling
                        let frame = theme.input_frame(self.focused, has_error);
                        let inner_response = frame.show(ui, |ui| {
                            ui.set_width(width - 20.0); // Account for frame margins
                            TextEdit::singleline(&mut self.text_buffer)
                                .horizontal_align(Align::Center)
                                .frame(false) // We use our own frame
                                .desired_width(ui.available_width())
                                .ui(ui)
                        });

                        let edit_response = inner_response.inner;

                        // Track focus state
                        if edit_response.gained_focus() {
                            self.focused = true;
                        }
                        if edit_response.lost_focus() {
                            self.focused = false;
                            response.lost_focus = true;
                            self.text_buffer = format_value(self.value, precision);
                        }

                        if edit_response.changed() {
                            let filtered = filter_numeric_input(
                                &self.text_buffer,
                                allow_negative,
                                allow_decimal,
                            );
                            if filtered != self.text_buffer {
                                self.text_buffer = filtered;
                            }

                            if let Ok(parsed) = self.text_buffer.trim().parse::<f64>() {
                                self.value = parsed.clamp(min, max);
                                response.changed = true;
                            }
                        }
                    });

                    // Suffix (fixed width based on content)
                    if let Some(ref sfx) = suffix {
                        let suffix_width = sfx.len() as f32 * 8.0 + theme.spacing_xs;
                        row.add_ui(item().basis(suffix_width), |ui| {
                            ui.label(
                                RichText::new(sfx)
                                    .size(theme.input_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                });
        });
    }

    /// Drag mode: Full-width drag value
    /// ```text
    /// ┌─────────────────────────────────────┐
    /// │ Label *                     0 – 100 │  <- Row 1: label + range
    /// │ [      ◀ 0 pcs ▶      ]             │  <- Row 2: drag value (full width)
    /// └─────────────────────────────────────┘
    /// ```
    fn show_drag_mode(
        &mut self,
        flex: &mut egui_flex::FlexInstance,
        theme: &ParameterTheme,
        name: &str,
        required: bool,
        response: &mut WidgetResponse,
    ) {
        let min = self.parameter.get_min();
        let max = self.parameter.get_max();
        let prefix = self.parameter.get_prefix();
        let suffix = self.parameter.get_suffix();
        let has_range = min.is_some() || max.is_some();

        // Row 1: Label ... Range (nested horizontal Flex)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .show(ui, |row| {
                    // Left group: Label + required marker (bold)
                    row.add_ui(item(), |ui| {
                        Flex::horizontal()
                            .align_items(FlexAlign::Center)
                            .gap(egui::vec2(2.0, 0.0))
                            .show(ui, |label_group| {
                                label_group.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(name)
                                            .size(theme.label_font_size)
                                            .color(theme.label_color)
                                            .strong(),
                                    );
                                });
                                if required {
                                    label_group.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new("*")
                                                .size(theme.label_font_size)
                                                .color(theme.error),
                                        );
                                    });
                                }
                            });
                    });

                    // Spacer
                    row.add_ui(item().grow(1.0), |_ui| {});

                    // Right: Range hint
                    if has_range {
                        row.add_ui(item(), |ui| {
                            let range_text = format_range(min, max);
                            ui.label(
                                RichText::new(range_text)
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                });
        });

        // Row 2: Drag value (grows to full width)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .show(ui, |row| {
                    row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                        let drag_width = ui.available_width();

                        let mut drag_value = DragValue::new(&mut self.value);

                        match (min, max) {
                            (Some(mn), Some(mx)) => drag_value = drag_value.range(mn..=mx),
                            (Some(mn), None) => drag_value = drag_value.range(mn..=f64::MAX),
                            (None, Some(mx)) => drag_value = drag_value.range(f64::MIN..=mx),
                            _ => {}
                        }

                        if let Some(step) = self.parameter.get_step() {
                            drag_value = drag_value.speed(step * 0.1);
                        } else {
                            drag_value = drag_value.speed(0.1);
                        }

                        if let Some(precision) = self.parameter.get_precision() {
                            drag_value = drag_value.max_decimals(precision as usize);
                        }
                        if let Some(pfx) = prefix {
                            drag_value = drag_value.prefix(pfx);
                        }
                        if let Some(sfx) = suffix {
                            drag_value = drag_value.suffix(sfx);
                        }

                        let drag_response =
                            ui.add_sized([drag_width, theme.control_height], drag_value);

                        if drag_response.changed() {
                            response.changed = true;
                        }
                        if drag_response.lost_focus() {
                            response.lost_focus = true;
                        }
                    });
                });
        });
    }

    /// Slider mode: Slider with inline value display
    /// ```text
    /// ┌─────────────────────────────────────┐
    /// │ Label *                             │  <- Row 1: label
    /// │ [━━━━━━━○━━━━━━━━━━━━━━━━━] 0.0%    │  <- Row 2: slider + value
    /// └─────────────────────────────────────┘
    /// ```
    fn show_slider_mode(
        &mut self,
        flex: &mut egui_flex::FlexInstance,
        theme: &ParameterTheme,
        name: &str,
        required: bool,
        response: &mut WidgetResponse,
    ) {
        let min = self.parameter.get_min().unwrap_or(0.0);
        let max = self.parameter.get_max().unwrap_or(100.0);
        let suffix = self.parameter.get_suffix();
        let precision = self.parameter.get_precision().unwrap_or(1);
        let step = self.parameter.get_step();

        // Row 1: Label (nested horizontal Flex, bold)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .show(ui, |row| {
                    row.add_ui(item(), |ui| {
                        Flex::horizontal()
                            .align_items(FlexAlign::Center)
                            .gap(egui::vec2(2.0, 0.0))
                            .show(ui, |label_group| {
                                label_group.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(name)
                                            .size(theme.label_font_size)
                                            .color(theme.label_color)
                                            .strong(),
                                    );
                                });
                                if required {
                                    label_group.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new("*")
                                                .size(theme.label_font_size)
                                                .color(theme.error),
                                        );
                                    });
                                }
                            });
                    });
                });
        });

        // Prepare value text
        let value_text = if let Some(sfx) = suffix {
            format!("{:.prec$}{}", self.value, sfx, prec = precision as usize)
        } else {
            format!("{:.prec$}", self.value, prec = precision as usize)
        };

        // Calculate value text width
        let value_width = value_text.len() as f32 * 8.0 + theme.spacing_sm;

        // Row 2: Slider + Value (nested horizontal Flex)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .align_content(FlexAlignContent::Stretch)
                .gap(egui::vec2(theme.spacing_sm, 0.0))
                .show(ui, |row| {
                    // Slider - GROWS to fill remaining space
                    row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                        ui.style_mut().spacing.slider_width = ui.available_width();

                        let mut slider = Slider::new(&mut self.value, min..=max).show_value(false);

                        if let Some(s) = step {
                            slider = slider.step_by(s);
                        }

                        let slider_response = slider.ui(ui);

                        if slider_response.changed() {
                            response.changed = true;
                        }
                        if slider_response.lost_focus() {
                            response.lost_focus = true;
                        }
                    });

                    // Value label - fixed width based on content
                    row.add_ui(item().basis(value_width), |ui| {
                        ui.label(
                            RichText::new(&value_text)
                                .size(theme.input_font_size)
                                .color(theme.label_color),
                        );
                    });
                });
        });
    }

    /// Slider + Text mode: Slider with editable input
    /// ```text
    /// ┌─────────────────────────────────────┐
    /// │ Label *                             │  <- Row 1: label
    /// │ [━━━━━━━○━━━━━━━━━━━] [0.0] %       │  <- Row 2: slider + input + suffix
    /// └─────────────────────────────────────┘
    /// ```
    fn show_slider_text_mode(
        &mut self,
        flex: &mut egui_flex::FlexInstance,
        theme: &ParameterTheme,
        name: &str,
        required: bool,
        response: &mut WidgetResponse,
    ) {
        let min = self.parameter.get_min().unwrap_or(0.0);
        let max = self.parameter.get_max().unwrap_or(100.0);
        let step = self.parameter.get_step();
        let precision = self.parameter.get_precision();
        let suffix = self.parameter.get_suffix().map(|s| s.to_string());

        // Row 1: Label (nested horizontal Flex, bold)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .show(ui, |row| {
                    row.add_ui(item(), |ui| {
                        Flex::horizontal()
                            .align_items(FlexAlign::Center)
                            .gap(egui::vec2(2.0, 0.0))
                            .show(ui, |label_group| {
                                label_group.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(name)
                                            .size(theme.label_font_size)
                                            .color(theme.label_color)
                                            .strong(),
                                    );
                                });
                                if required {
                                    label_group.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new("*")
                                                .size(theme.label_font_size)
                                                .color(theme.error),
                                        );
                                    });
                                }
                            });
                    });
                });
        });

        // Row 2: Slider + Input + Suffix (nested horizontal Flex)
        flex.add_ui(item().grow(1.0), |ui| {
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Center)
                .align_content(FlexAlignContent::Stretch)
                .gap(egui::vec2(theme.spacing_sm, 0.0))
                .show(ui, |row| {
                    // Slider - GROWS to fill remaining space
                    row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                        ui.style_mut().spacing.slider_width = ui.available_width();

                        let mut slider = Slider::new(&mut self.value, min..=max).show_value(false);

                        if let Some(s) = step {
                            slider = slider.step_by(s);
                        }

                        let slider_response = slider.ui(ui);

                        if slider_response.changed() {
                            self.text_buffer = format_value(self.value, precision);
                            response.changed = true;
                        }
                    });

                    // Text input - fixed width
                    row.add_ui(item().basis(theme.slider_text_input_width), |ui| {
                        let edit_response = TextEdit::singleline(&mut self.text_buffer)
                            .horizontal_align(Align::Center)
                            .desired_width(theme.slider_text_input_width)
                            .ui(ui);

                        if edit_response.changed() {
                            if let Ok(parsed) = self.text_buffer.trim().parse::<f64>() {
                                self.value = parsed.clamp(min, max);
                                response.changed = true;
                            }
                        }

                        if edit_response.lost_focus() {
                            self.text_buffer = format_value(self.value, precision);
                        }
                    });

                    // Suffix - fixed width based on content
                    if let Some(ref sfx) = suffix {
                        let suffix_width = sfx.len() as f32 * 8.0 + theme.spacing_xs;
                        row.add_ui(item().basis(suffix_width), |ui| {
                            ui.label(
                                RichText::new(sfx)
                                    .size(theme.input_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                });
        });
    }

    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = value;
        self.text_buffer = format_value(value, self.parameter.get_precision());
        let _ = self.parameter.set(value);
    }
}

/// Format a number with optional precision
fn format_value(value: f64, precision: Option<u8>) -> String {
    match precision {
        Some(p) => format!("{:.prec$}", value, prec = p as usize),
        None => {
            // Auto-format: remove trailing zeros
            let s = format!("{:.6}", value);
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.to_string()
        }
    }
}

/// Format range hint text
fn format_range(min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (Some(mn), Some(mx)) => format!("{} – {}", mn, mx),
        (Some(mn), None) => format!("≥ {}", mn),
        (None, Some(mx)) => format!("≤ {}", mx),
        _ => String::new(),
    }
}

/// Filter input to only allow numeric characters
fn filter_numeric_input(input: &str, allow_negative: bool, allow_decimal: bool) -> String {
    let mut result = String::with_capacity(input.len());
    let mut has_decimal = false;
    let mut has_minus = false;

    for (i, c) in input.chars().enumerate() {
        match c {
            '0'..='9' => result.push(c),
            '-' if allow_negative && i == 0 && !has_minus => {
                has_minus = true;
                result.push(c);
            }
            '.' if allow_decimal && !has_decimal => {
                has_decimal = true;
                result.push(c);
            }
            ',' if allow_decimal && !has_decimal => {
                has_decimal = true;
                result.push('.');
            }
            _ => {}
        }
    }

    result
}
