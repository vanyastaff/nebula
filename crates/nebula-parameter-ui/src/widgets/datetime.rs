//! DateTime input widget for DateTimeParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::Ui;
use egui_extras::DatePickerButton;
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::DateTimeParameter;

/// Widget for combined date and time selection.
pub struct DateTimeWidget {
    parameter: DateTimeParameter,
    /// Selected date for picker
    date: chrono::NaiveDate,
    /// Hours (0-23)
    hours: u32,
    /// Minutes (0-59)
    minutes: u32,
    /// Seconds (0-59)
    seconds: u32,
}

impl ParameterWidget for DateTimeWidget {
    type Parameter = DateTimeParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let (date, hours, minutes, seconds) = parameter
            .default
            .as_ref()
            .map(|t| parse_datetime(t.as_str()))
            .unwrap_or_else(|| {
                let now = chrono::Local::now();
                (
                    now.date_naive(),
                    now.format("%H").to_string().parse().unwrap_or(0),
                    now.format("%M").to_string().parse().unwrap_or(0),
                    now.format("%S").to_string().parse().unwrap_or(0),
                )
            });

        Self {
            parameter,
            date,
            hours,
            minutes,
            seconds,
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

        // Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).color(theme.label_color));
            if required {
                ui.label(egui::RichText::new("*").color(theme.error));
            }
        });

        ui.add_space(2.0);

        let use_12_hour = self.parameter.uses_12_hour_format();

        // DateTime input - flat, simple row
        ui.horizontal(|ui| {
            // Date picker from egui_extras
            let old_date = self.date;
            ui.add(DatePickerButton::new(&mut self.date).id_salt("datetime_picker"));

            if self.date != old_date {
                self.update_parameter(&mut response);
            }

            ui.add_space(8.0);
            ui.label(egui::RichText::new("|").color(theme.hint_color));
            ui.add_space(8.0);

            // Hours
            let display_hours = if use_12_hour {
                let h = self.hours % 12;
                if h == 0 { 12 } else { h }
            } else {
                self.hours
            };

            let mut hours_edit = display_hours;
            let hours_range = if use_12_hour { 1..=12 } else { 0..=23 };

            ui.add_sized(
                [36.0, 20.0],
                egui::DragValue::new(&mut hours_edit)
                    .range(hours_range)
                    .speed(0.1),
            );

            if hours_edit != display_hours {
                if use_12_hour {
                    let is_pm = self.hours >= 12;
                    self.hours = if is_pm {
                        if hours_edit == 12 {
                            12
                        } else {
                            hours_edit + 12
                        }
                    } else {
                        if hours_edit == 12 { 0 } else { hours_edit }
                    };
                } else {
                    self.hours = hours_edit;
                }
                self.update_parameter(&mut response);
            }

            ui.label(egui::RichText::new(":").color(theme.label_color));

            // Minutes
            let mut minutes_edit = self.minutes;
            ui.add_sized(
                [36.0, 20.0],
                egui::DragValue::new(&mut minutes_edit)
                    .range(0..=59)
                    .speed(0.1),
            );

            if minutes_edit != self.minutes {
                self.minutes = minutes_edit;
                self.update_parameter(&mut response);
            }

            ui.label(egui::RichText::new(":").color(theme.label_color));

            // Seconds
            let mut seconds_edit = self.seconds;
            ui.add_sized(
                [36.0, 20.0],
                egui::DragValue::new(&mut seconds_edit)
                    .range(0..=59)
                    .speed(0.1),
            );

            if seconds_edit != self.seconds {
                self.seconds = seconds_edit;
                self.update_parameter(&mut response);
            }

            // AM/PM toggle
            if use_12_hour {
                ui.add_space(4.0);
                let is_pm = self.hours >= 12;
                let am_pm_text = if is_pm { "PM" } else { "AM" };
                if ui.small_button(am_pm_text).clicked() {
                    if is_pm {
                        self.hours -= 12;
                    } else {
                        self.hours += 12;
                    }
                    self.update_parameter(&mut response);
                }
            }

            // Now button
            ui.add_space(4.0);
            if ui.small_button("Now").clicked() {
                self.set_now();
                response.changed = true;
            }
        });

        // Timezone info
        if let Some(tz) = self.parameter.get_timezone() {
            ui.label(
                egui::RichText::new(format!("Timezone: {}", tz))
                    .small()
                    .color(theme.hint_color),
            );
        }

        // Hint (help text below field)
        if let Some(hint_text) = hint {
            if !hint_text.is_empty() {
                ui.label(
                    egui::RichText::new(&hint_text)
                        .small()
                        .color(theme.hint_color),
                );
            }
        }

        // Error
        if let Some(ref error) = response.error {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(error).small().color(theme.error));
        }

        response
    }
}

impl DateTimeWidget {
    fn update_parameter(&mut self, response: &mut WidgetResponse) {
        // Value is stored in the widget, not in the parameter
        response.changed = true;
    }

    /// Get the current datetime as a formatted string.
    #[must_use]
    pub fn value(&self) -> String {
        format!(
            "{} {:02}:{:02}:{:02}",
            self.date.format("%Y-%m-%d"),
            self.hours,
            self.minutes,
            self.seconds
        )
    }

    /// Set to current datetime.
    pub fn set_now(&mut self) {
        let now = chrono::Local::now();
        self.date = now.date_naive();
        self.hours = now.format("%H").to_string().parse().unwrap_or(0);
        self.minutes = now.format("%M").to_string().parse().unwrap_or(0);
        self.seconds = now.format("%S").to_string().parse().unwrap_or(0);
    }
}

/// Parse datetime string into date and time components.
fn parse_datetime(datetime: &str) -> (chrono::NaiveDate, u32, u32, u32) {
    // Try to split on space or T
    let parts: Vec<&str> = if datetime.contains('T') {
        datetime.split('T').collect()
    } else {
        datetime.split(' ').collect()
    };

    let date_str = parts.first().unwrap_or(&"2024-01-01");
    let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());

    let time_str = parts.get(1).unwrap_or(&"00:00:00");
    let time_parts: Vec<&str> = time_str.split(':').collect();

    let hours = time_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minutes = time_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let seconds = time_parts
        .get(2)
        .and_then(|s| s.trim_end_matches('Z').parse().ok())
        .unwrap_or(0);

    (date, hours, minutes, seconds)
}
