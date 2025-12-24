//! Date input widget for DateParameter.
//!
//! Custom calendar popup with iOS-style design.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use chrono::{Datelike, Local, NaiveDate};
use egui::{RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use egui_phosphor::regular::{CALENDAR, CARET_LEFT, CARET_RIGHT};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::DateParameter;

/// Widget for date selection with custom calendar.
pub struct DateWidget {
    parameter: DateParameter,
    date: NaiveDate,
    popup_id: egui::Id,
    view_year: i32,
    view_month: u32,
    text_input: String,
    popup_open: bool,
}

impl ParameterWidget for DateWidget {
    type Parameter = DateParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let date = parameter
            .default
            .as_ref()
            .and_then(|s| NaiveDate::parse_from_str(s.as_str(), "%Y-%m-%d").ok())
            .unwrap_or_else(|| Local::now().date_naive());

        let popup_id = egui::Id::new("date_popup").with(parameter.metadata().key.as_str());
        let text_input = date.format("%Y-%m-%d").to_string();

        Self {
            parameter,
            date,
            popup_id,
            view_year: date.year(),
            view_month: date.month(),
            text_input,
            popup_open: false,
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

        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Label
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
                                                RichText::new(&name)
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
                            row.add_ui(item().grow(1.0), |_ui| {});
                        });
                });

                // Date input: text field + calendar button inside frame (like SecretWidget)
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .show(ui, |row| {
                            row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                                let width = ui.available_width();
                                let is_open = self.popup_open;

                                // Check if current text is valid date (for real-time feedback)
                                let is_valid_date = self.text_input.is_empty()
                                    || NaiveDate::parse_from_str(&self.text_input, "%Y-%m-%d")
                                        .is_ok();

                                // Frame to contain input + icon using theme helper
                                let frame_response =
                                    theme.input_frame(is_open, !is_valid_date).show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.style_mut().visuals.widgets.inactive.bg_stroke =
                                                egui::Stroke::NONE;
                                            ui.style_mut().visuals.widgets.hovered.bg_stroke =
                                                egui::Stroke::NONE;
                                            ui.style_mut().visuals.widgets.active.bg_stroke =
                                                egui::Stroke::NONE;

                                            // Text input - no frame
                                            // Account for: padding (8*2=16) + icon button (~24) + spacing
                                            let text_edit =
                                                egui::TextEdit::singleline(&mut self.text_input)
                                                    .hint_text("YYYY-MM-DD")
                                                    .frame(false)
                                                    .desired_width(width - 56.0);

                                            let text_response = ui.add(text_edit);

                                            // Parse date when text changes
                                            if text_response.changed() {
                                                if let Ok(parsed_date) = NaiveDate::parse_from_str(
                                                    &self.text_input,
                                                    "%Y-%m-%d",
                                                ) {
                                                    self.date = parsed_date;
                                                    self.view_year = parsed_date.year();
                                                    self.view_month = parsed_date.month();
                                                    response.changed = true;
                                                }
                                            }

                                            // Validate on focus lost
                                            if text_response.lost_focus() {
                                                if NaiveDate::parse_from_str(
                                                    &self.text_input,
                                                    "%Y-%m-%d",
                                                )
                                                .is_err()
                                                {
                                                    // Reset to current date if invalid
                                                    self.text_input =
                                                        self.date.format("%Y-%m-%d").to_string();
                                                }
                                            }

                                            // Calendar icon button - inside the frame with hover
                                            let fluent_blue = egui::Color32::from_rgb(96, 205, 245);
                                            let icon_color = if is_open {
                                                fluent_blue
                                            } else {
                                                theme.hint_color
                                            };

                                            let button = egui::Button::new(
                                                RichText::new(CALENDAR)
                                                    .size(16.0)
                                                    .color(icon_color),
                                            )
                                            .fill(egui::Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::NONE)
                                            .corner_radius(4.0);

                                            let btn_response = ui.add(button);

                                            // Hover effect - repaint with lighter color
                                            if btn_response.hovered() {
                                                ui.ctx().request_repaint();
                                            }

                                            if btn_response.clicked() {
                                                self.view_year = self.date.year();
                                                self.view_month = self.date.month();
                                                self.popup_open = !self.popup_open;
                                            }
                                        });
                                    });

                                // Popup below the frame
                                if is_open {
                                    let popup_width = 280.0; // Fixed width for calendar
                                    egui::Area::new(self.popup_id)
                                        .order(egui::Order::Foreground)
                                        .pivot(egui::Align2::LEFT_TOP)
                                        .fixed_pos(frame_response.response.rect.left_bottom())
                                        .show(ui.ctx(), |ui| {
                                            theme.popup_frame().show(ui, |ui| {
                                                ui.set_width(popup_width);
                                                self.show_calendar(ui, theme, &mut response);
                                            });
                                        });

                                    // Close on click outside
                                    if ui.input(|i| i.pointer.any_click()) {
                                        let popup_rect = egui::Rect::from_min_size(
                                            frame_response.response.rect.left_bottom(),
                                            egui::vec2(popup_width + 16.0, 250.0),
                                        );
                                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                            if !popup_rect.contains(pos)
                                                && !frame_response.response.rect.contains(pos)
                                            {
                                                self.popup_open = false;
                                            }
                                        }
                                    }
                                }

                                // Show validation error in real-time
                                if !is_valid_date && !self.text_input.is_empty() {
                                    ui.add_space(2.0);
                                    ui.label(
                                        RichText::new("Invalid format. Use YYYY-MM-DD")
                                            .size(theme.hint_font_size)
                                            .color(theme.error),
                                    );
                                }
                            });
                        });
                });

                // Hint
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

impl DateWidget {
    fn show_calendar(
        &mut self,
        ui: &mut Ui,
        theme: &ParameterTheme,
        response: &mut WidgetResponse,
    ) {
        let today = Local::now().date_naive();
        let _popup_id = self.popup_id;

        // Compact calendar layout
        let cell_size = 32.0;
        let grid_spacing = 2.0;

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

            // Header: [◀] December 2025 [▶] - centered
            ui.horizontal(|ui| {
                // Previous month button
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(CARET_LEFT).size(16.0).color(theme.hint_color),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if self.view_month == 1 {
                        self.view_month = 12;
                        self.view_year -= 1;
                    } else {
                        self.view_month -= 1;
                    }
                }

                // Centered month/year label
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                month_name(self.view_month),
                                self.view_year
                            ))
                            .size(14.0)
                            .strong()
                            .color(theme.label_color),
                        );
                    },
                );

                // Next month button
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(CARET_RIGHT)
                                .size(16.0)
                                .color(theme.hint_color),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if self.view_month == 12 {
                        self.view_month = 1;
                        self.view_year += 1;
                    } else {
                        self.view_month += 1;
                    }
                }
            });

            ui.add_space(8.0);

            // Day headers: Mo Tu We Th Fr Sa Su
            egui::Grid::new("weekday_headers")
                .num_columns(7)
                .spacing([grid_spacing, grid_spacing])
                .show(ui, |ui| {
                    let days = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
                    for (i, day) in days.iter().enumerate() {
                        let color = if i >= 5 {
                            theme.hint_color.gamma_multiply(0.7)
                        } else {
                            theme.hint_color
                        };
                        ui.add_sized(
                            [cell_size, 20.0],
                            egui::Label::new(RichText::new(*day).size(11.0).color(color)),
                        );
                    }
                    ui.end_row();
                });

            ui.add_space(4.0);

            // Calendar grid
            let first_day = NaiveDate::from_ymd_opt(self.view_year, self.view_month, 1).unwrap();
            let current_month_days = days_in_month(self.view_year, self.view_month);

            // Monday = 0
            let first_weekday = first_day.weekday().num_days_from_monday() as i32;

            let prev_month_days = if self.view_month == 1 {
                days_in_month(self.view_year - 1, 12)
            } else {
                days_in_month(self.view_year, self.view_month - 1)
            };

            let mut day_counter: i32 = 1 - first_weekday;

            egui::Grid::new("calendar_days")
                .num_columns(7)
                .spacing([grid_spacing, grid_spacing])
                .show(ui, |ui| {
                    for _week in 0..6 {
                        for weekday in 0..7 {
                            let (day_num, is_current_month, date_opt): (
                                i32,
                                bool,
                                Option<NaiveDate>,
                            ) = if day_counter < 1 {
                                let d = prev_month_days as i32 + day_counter;
                                (d, false, None)
                            } else if day_counter > current_month_days as i32 {
                                let d = day_counter - current_month_days as i32;
                                (d, false, None)
                            } else {
                                let date = NaiveDate::from_ymd_opt(
                                    self.view_year,
                                    self.view_month,
                                    day_counter as u32,
                                );
                                (day_counter, true, date)
                            };

                            let is_selected = date_opt.map_or(false, |d| d == self.date);
                            let is_today = date_opt.map_or(false, |d| d == today);
                            let is_weekend = weekday >= 5;

                            // Fluent Dark colors
                            let fluent_blue = egui::Color32::from_rgb(96, 205, 245); // #60CDF5
                            let other_month_color = egui::Color32::from_rgb(128, 128, 128); // #808080

                            let (bg_color, text_color) = if is_selected {
                                (fluent_blue, egui::Color32::BLACK) // Blue bg, dark text
                            } else if !is_current_month {
                                (egui::Color32::TRANSPARENT, other_month_color)
                            } else if is_today {
                                // Today: blue text
                                (egui::Color32::TRANSPARENT, fluent_blue)
                            } else if is_weekend {
                                (
                                    egui::Color32::TRANSPARENT,
                                    theme.hint_color.gamma_multiply(0.7),
                                )
                            } else {
                                (egui::Color32::TRANSPARENT, theme.label_color)
                            };

                            let button = egui::Button::new(
                                RichText::new(format!("{}", day_num))
                                    .size(14.0)
                                    .color(text_color),
                            )
                            .fill(bg_color)
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(cell_size / 2.0)
                            .min_size(egui::vec2(cell_size, cell_size));

                            let btn_response = ui.add(button);

                            // Click selects immediately (iOS style)
                            if btn_response.clicked() && is_current_month {
                                if let Some(date) = date_opt {
                                    self.date = date;
                                    let date_str = self.date.format("%Y-%m-%d").to_string();
                                    self.text_input = date_str;
                                    response.changed = true;
                                    self.popup_open = false;
                                }
                            }

                            day_counter += 1;
                        }
                        ui.end_row();

                        if day_counter > current_month_days as i32 {
                            break;
                        }
                    }
                });
        });
    }

    #[must_use]
    pub fn value(&self) -> String {
        self.date.format("%Y-%m-%d").to_string()
    }

    pub fn set_value(&mut self, value: &str) {
        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            self.date = date;
            self.view_year = date.year();
            self.view_month = date.month();
            self.text_input = value.to_string();
        }
    }

    pub fn set_today(&mut self) {
        self.date = Local::now().date_naive();
        self.view_year = self.date.year();
        self.view_month = self.date.month();
        let date_str = self.date.format("%Y-%m-%d").to_string();
        self.text_input = date_str;
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}
