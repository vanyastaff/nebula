//! Calendar component for date selection.

use crate::theme::current_theme;
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use egui::{RichText, Ui, Vec2};

/// Calendar display mode
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CalendarMode {
    /// Single date selection
    #[default]
    Single,
    /// Date range selection
    Range,
    /// Multiple dates selection
    Multiple,
}

/// Calendar component for date picking
///
/// # Example
///
/// ```rust,ignore
/// let mut selected = None;
/// Calendar::new(&mut selected)
///     .show(ui);
/// ```
pub struct Calendar<'a> {
    selected: &'a mut Option<NaiveDate>,
    mode: CalendarMode,
    min_date: Option<NaiveDate>,
    max_date: Option<NaiveDate>,
    highlighted: Vec<NaiveDate>,
    disabled_dates: Vec<NaiveDate>,
    show_week_numbers: bool,
    first_day_of_week: Weekday,
    current_month: Option<NaiveDate>,
}

impl<'a> Calendar<'a> {
    /// Create a new calendar
    pub fn new(selected: &'a mut Option<NaiveDate>) -> Self {
        Self {
            selected,
            mode: CalendarMode::Single,
            min_date: None,
            max_date: None,
            highlighted: Vec::new(),
            disabled_dates: Vec::new(),
            show_week_numbers: false,
            first_day_of_week: Weekday::Mon,
            current_month: None,
        }
    }

    /// Set calendar mode
    pub fn mode(mut self, mode: CalendarMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set minimum selectable date
    pub fn min_date(mut self, date: NaiveDate) -> Self {
        self.min_date = Some(date);
        self
    }

    /// Set maximum selectable date
    pub fn max_date(mut self, date: NaiveDate) -> Self {
        self.max_date = Some(date);
        self
    }

    /// Set highlighted dates
    pub fn highlighted(mut self, dates: Vec<NaiveDate>) -> Self {
        self.highlighted = dates;
        self
    }

    /// Set disabled dates
    pub fn disabled(mut self, dates: Vec<NaiveDate>) -> Self {
        self.disabled_dates = dates;
        self
    }

    /// Show week numbers
    pub fn week_numbers(mut self) -> Self {
        self.show_week_numbers = true;
        self
    }

    /// Set first day of week
    pub fn first_day(mut self, day: Weekday) -> Self {
        self.first_day_of_week = day;
        self
    }

    /// Set initial month to display
    pub fn month(mut self, date: NaiveDate) -> Self {
        self.current_month = Some(date);
        self
    }

    /// Check if a date is disabled
    fn is_disabled(&self, date: NaiveDate) -> bool {
        if self.disabled_dates.contains(&date) {
            return true;
        }
        if let Some(min) = self.min_date {
            if date < min {
                return true;
            }
        }
        if let Some(max) = self.max_date {
            if date > max {
                return true;
            }
        }
        false
    }

    /// Show the calendar
    pub fn show(self, ui: &mut Ui) -> CalendarResponse {
        self.show_inner(ui)
    }

    fn show_inner(self, ui: &mut Ui) -> CalendarResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let today = Local::now().date_naive();
        let display_month = self.current_month.or(*self.selected).unwrap_or(today);

        let first_of_month =
            NaiveDate::from_ymd_opt(display_month.year(), display_month.month(), 1)
                .unwrap_or(today);
        let days_in_month = days_in_month(display_month.year(), display_month.month());

        let mut response = CalendarResponse {
            changed: false,
            selected: *self.selected,
        };

        let frame = egui::Frame::NONE
            .fill(tokens.card)
            .stroke(egui::Stroke::new(1.0, tokens.border))
            .corner_radius(tokens.rounding_lg())
            .inner_margin(tokens.spacing_md as i8);

        frame.show(ui, |ui| {
            ui.set_min_width(280.0);

            // Header with month/year and navigation
            ui.horizontal(|ui| {
                // Previous month button
                if ui.small_button("◀").clicked() {
                    // Would need state management for month navigation
                }

                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        let month_name = month_name(display_month.month());
                        ui.label(
                            RichText::new(format!("{} {}", month_name, display_month.year()))
                                .size(tokens.font_size_md)
                                .strong()
                                .color(tokens.foreground),
                        );
                    },
                );

                // Next month button
                if ui.small_button("▶").clicked() {
                    // Would need state management for month navigation
                }
            });

            ui.add_space(tokens.spacing_sm);

            // Day headers
            ui.horizontal(|ui| {
                let cell_size = Vec2::splat(32.0);

                if self.show_week_numbers {
                    ui.allocate_space(Vec2::new(24.0, cell_size.y));
                }

                for i in 0..7 {
                    let day = weekday_from_offset(self.first_day_of_week, i);
                    let day_name = short_weekday_name(day);

                    let (rect, _) = ui.allocate_exact_size(cell_size, egui::Sense::hover());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        day_name,
                        egui::FontId::proportional(tokens.font_size_xs),
                        tokens.muted_foreground,
                    );
                }
            });

            ui.add_space(tokens.spacing_xs);

            // Calendar grid
            let first_weekday = first_of_month.weekday();
            let offset = weekday_offset(self.first_day_of_week, first_weekday);

            let mut current_day = 1i32;
            let cell_size = Vec2::splat(32.0);

            for week in 0..6 {
                if current_day > days_in_month as i32 {
                    break;
                }

                ui.horizontal(|ui| {
                    // Week number
                    if self.show_week_numbers {
                        let week_date =
                            first_of_month + Duration::days((week * 7) as i64 - offset as i64);
                        let week_num = week_date.iso_week().week();
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(24.0, cell_size.y),
                            egui::Sense::hover(),
                        );
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("{}", week_num),
                            egui::FontId::proportional(tokens.font_size_xs),
                            tokens.muted_foreground,
                        );
                    }

                    for day_of_week in 0..7 {
                        let cell_index = week * 7 + day_of_week;

                        if cell_index < offset || current_day > days_in_month as i32 {
                            // Empty cell
                            ui.allocate_space(cell_size);
                        } else {
                            let date = NaiveDate::from_ymd_opt(
                                display_month.year(),
                                display_month.month(),
                                current_day as u32,
                            );

                            if let Some(date) = date {
                                let is_today = date == today;
                                let is_selected = *self.selected == Some(date);
                                let is_highlighted = self.highlighted.contains(&date);
                                let is_disabled = self.is_disabled(date);

                                let (rect, cell_response) = ui.allocate_exact_size(
                                    cell_size,
                                    if is_disabled {
                                        egui::Sense::hover()
                                    } else {
                                        egui::Sense::click()
                                    },
                                );

                                // Background
                                let bg_color = if is_selected {
                                    tokens.primary
                                } else if cell_response.hovered() && !is_disabled {
                                    tokens.accent
                                } else if is_highlighted {
                                    tokens.accent
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                if bg_color != egui::Color32::TRANSPARENT {
                                    ui.painter().rect_filled(
                                        rect.shrink(2.0),
                                        tokens.radius_md,
                                        bg_color,
                                    );
                                }

                                // Today indicator
                                if is_today && !is_selected {
                                    ui.painter().rect_stroke(
                                        rect.shrink(2.0),
                                        tokens.radius_md,
                                        egui::Stroke::new(1.0, tokens.primary),
                                        egui::StrokeKind::Inside,
                                    );
                                }

                                // Day number
                                let text_color = if is_selected {
                                    tokens.primary_foreground
                                } else if is_disabled {
                                    tokens.muted_foreground
                                } else {
                                    tokens.foreground
                                };

                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    format!("{}", current_day),
                                    egui::FontId::proportional(tokens.font_size_sm),
                                    text_color,
                                );

                                // Handle click
                                if cell_response.clicked() && !is_disabled {
                                    *self.selected = Some(date);
                                    response.changed = true;
                                    response.selected = Some(date);
                                }
                            }

                            current_day += 1;
                        }
                    }
                });
            }
        });

        response
    }
}

/// Response from calendar interaction
#[derive(Clone, Debug)]
pub struct CalendarResponse {
    /// Whether the selection changed
    pub changed: bool,
    /// Currently selected date
    pub selected: Option<NaiveDate>,
}

/// Date range picker using two calendars
pub struct DateRangePicker<'a> {
    start: &'a mut Option<NaiveDate>,
    end: &'a mut Option<NaiveDate>,
    min_date: Option<NaiveDate>,
    max_date: Option<NaiveDate>,
}

impl<'a> DateRangePicker<'a> {
    /// Create a new date range picker
    pub fn new(start: &'a mut Option<NaiveDate>, end: &'a mut Option<NaiveDate>) -> Self {
        Self {
            start,
            end,
            min_date: None,
            max_date: None,
        }
    }

    /// Set minimum date
    pub fn min_date(mut self, date: NaiveDate) -> Self {
        self.min_date = Some(date);
        self
    }

    /// Set maximum date
    pub fn max_date(mut self, date: NaiveDate) -> Self {
        self.max_date = Some(date);
        self
    }

    /// Show the date range picker
    pub fn show(self, ui: &mut Ui) -> DateRangeResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut response = DateRangeResponse {
            changed: false,
            start: *self.start,
            end: *self.end,
        };

        ui.horizontal(|ui| {
            // Start date calendar
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Start Date")
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                );
                let mut calendar = Calendar::new(self.start);
                if let Some(min) = self.min_date {
                    calendar = calendar.min_date(min);
                }
                if let Some(end) = *self.end {
                    calendar = calendar.max_date(end);
                } else if let Some(max) = self.max_date {
                    calendar = calendar.max_date(max);
                }
                let cal_response = calendar.show(ui);
                if cal_response.changed {
                    response.changed = true;
                    response.start = cal_response.selected;
                }
            });

            ui.add_space(tokens.spacing_md);

            // End date calendar
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("End Date")
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                );
                let mut calendar = Calendar::new(self.end);
                if let Some(start) = *self.start {
                    calendar = calendar.min_date(start);
                } else if let Some(min) = self.min_date {
                    calendar = calendar.min_date(min);
                }
                if let Some(max) = self.max_date {
                    calendar = calendar.max_date(max);
                }
                let cal_response = calendar.show(ui);
                if cal_response.changed {
                    response.changed = true;
                    response.end = cal_response.selected;
                }
            });
        });

        response
    }
}

/// Response from date range picker
#[derive(Clone, Debug)]
pub struct DateRangeResponse {
    /// Whether selection changed
    pub changed: bool,
    /// Start date
    pub start: Option<NaiveDate>,
    /// End date
    pub end: Option<NaiveDate>,
}

// Helper functions

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
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

fn short_weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mo",
        Weekday::Tue => "Tu",
        Weekday::Wed => "We",
        Weekday::Thu => "Th",
        Weekday::Fri => "Fr",
        Weekday::Sat => "Sa",
        Weekday::Sun => "Su",
    }
}

fn weekday_from_offset(first_day: Weekday, offset: usize) -> Weekday {
    let start = first_day.num_days_from_monday();
    let day_num = (start + offset as u32) % 7;
    match day_num {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        6 => Weekday::Sun,
        _ => Weekday::Mon,
    }
}

fn weekday_offset(first_day: Weekday, target: Weekday) -> usize {
    let first = first_day.num_days_from_monday();
    let target_num = target.num_days_from_monday();
    ((target_num + 7 - first) % 7) as usize
}
