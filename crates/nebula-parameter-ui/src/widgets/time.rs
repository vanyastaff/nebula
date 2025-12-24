//! Time input widget for TimeParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{DragValue, RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::TimeParameter;

/// Widget for time selection.
/// ```text
/// ┌─────────────────────────────────────┐
/// │ Label *                             │  <- Row 1: label
/// │ [12]h : [30]m : [00]s [PM] [Now]    │  <- Row 2: time inputs + buttons
/// │ Hint text                           │  <- Row 3: hint
/// └─────────────────────────────────────┘
/// ```
pub struct TimeWidget {
    parameter: TimeParameter,
    hours: u32,
    minutes: u32,
    seconds: u32,
    is_pm: bool,
}

impl ParameterWidget for TimeWidget {
    type Parameter = TimeParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let (hours, minutes, seconds) = parameter
            .default
            .as_ref()
            .map(|t| parse_time(t.as_str()))
            .unwrap_or((0, 0, 0));

        let is_pm = hours >= 12;

        Self {
            parameter,
            hours,
            minutes,
            seconds,
            is_pm,
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

        let use_12_hour = self.parameter.uses_12_hour_format();
        let include_seconds = self.parameter.includes_seconds();
        let step_minutes = self.parameter.get_step_minutes();

        // Outer Flex: vertical container (left-aligned)
        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Row 1: Label (left-aligned, bold)
                flex.add_ui(item(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&name)
                                .size(theme.label_font_size)
                                .color(theme.label_color)
                                .strong(),
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

                // Row 2: Time inputs
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .gap(egui::vec2(theme.spacing_xs, 0.0))
                        .show(ui, |row| {
                            // Hours
                            row.add_ui(item().basis(50.0), |ui| {
                                let display_hours = if use_12_hour {
                                    let h = self.hours % 12;
                                    if h == 0 { 12 } else { h }
                                } else {
                                    self.hours
                                };

                                let mut hours_edit = display_hours;
                                let hours_range = if use_12_hour { 1..=12 } else { 0..=23 };

                                if ui
                                    .add(
                                        DragValue::new(&mut hours_edit)
                                            .range(hours_range)
                                            .suffix("h"),
                                    )
                                    .changed()
                                {
                                    if use_12_hour {
                                        self.hours = if self.is_pm {
                                            if hours_edit == 12 {
                                                12
                                            } else {
                                                hours_edit + 12
                                            }
                                        } else if hours_edit == 12 {
                                            0
                                        } else {
                                            hours_edit
                                        };
                                    } else {
                                        self.hours = hours_edit;
                                    }
                                    self.update_parameter(&mut response);
                                }
                            });

                            // Separator
                            row.add_ui(item(), |ui| {
                                ui.label(":");
                            });

                            // Minutes
                            row.add_ui(item().basis(50.0), |ui| {
                                let mut minutes_edit = self.minutes;
                                if ui
                                    .add(
                                        DragValue::new(&mut minutes_edit).range(0..=59).suffix("m"),
                                    )
                                    .changed()
                                {
                                    if step_minutes > 1 {
                                        self.minutes = (minutes_edit / step_minutes) * step_minutes;
                                    } else {
                                        self.minutes = minutes_edit;
                                    }
                                    self.update_parameter(&mut response);
                                }
                            });

                            // Seconds (optional)
                            if include_seconds {
                                row.add_ui(item(), |ui| {
                                    ui.label(":");
                                });

                                row.add_ui(item().basis(50.0), |ui| {
                                    let mut seconds_edit = self.seconds;
                                    if ui
                                        .add(
                                            DragValue::new(&mut seconds_edit)
                                                .range(0..=59)
                                                .suffix("s"),
                                        )
                                        .changed()
                                    {
                                        self.seconds = seconds_edit;
                                        self.update_parameter(&mut response);
                                    }
                                });
                            }

                            // AM/PM toggle (12-hour format)
                            if use_12_hour {
                                row.add_ui(item().basis(40.0), |ui| {
                                    let am_pm_text = if self.is_pm { "PM" } else { "AM" };
                                    if ui.small_button(am_pm_text).clicked() {
                                        self.is_pm = !self.is_pm;
                                        if self.is_pm && self.hours < 12 {
                                            self.hours += 12;
                                        } else if !self.is_pm && self.hours >= 12 {
                                            self.hours -= 12;
                                        }
                                        self.update_parameter(&mut response);
                                    }
                                });
                            }

                            // Spacer
                            row.add_ui(item().grow(1.0), |_ui| {});

                            // Now button
                            row.add_ui(item().basis(50.0), |ui| {
                                if ui.small_button("Now").clicked() {
                                    self.set_now();
                                    response.changed = true;
                                }
                            });
                        });
                });

                // Row 3: Hint
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

impl TimeWidget {
    fn update_parameter(&mut self, response: &mut WidgetResponse) {
        // Value is stored in the widget, not in the parameter
        response.changed = true;
    }

    #[must_use]
    pub fn value(&self) -> String {
        if self.parameter.includes_seconds() {
            format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
        } else {
            format!("{:02}:{:02}", self.hours, self.minutes)
        }
    }

    pub fn set_time(&mut self, hours: u32, minutes: u32, seconds: u32) {
        self.hours = hours.min(23);
        self.minutes = minutes.min(59);
        self.seconds = seconds.min(59);
        self.is_pm = self.hours >= 12;
    }

    pub fn set_now(&mut self) {
        let now = chrono::Local::now();
        let hours: u32 = now.format("%H").to_string().parse().unwrap_or(0);
        let minutes: u32 = now.format("%M").to_string().parse().unwrap_or(0);
        let seconds: u32 = now.format("%S").to_string().parse().unwrap_or(0);
        self.set_time(hours, minutes, seconds);
    }
}

fn parse_time(time: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = time.split(':').collect();
    let hours = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minutes = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let seconds = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (hours, minutes, seconds)
}
