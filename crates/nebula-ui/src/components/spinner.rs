//! Loading spinner and progress bar components.

use crate::theme::current_theme;
use egui::{Response, Ui, Vec2, Widget};
use std::f32::consts::TAU;

/// A loading spinner
pub struct Spinner {
    size: f32,
    color: Option<egui::Color32>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    /// Create a new spinner
    pub fn new() -> Self {
        Self {
            size: 24.0,
            color: None,
        }
    }

    /// Set size
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Set custom color
    pub fn color(mut self, color: egui::Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Small spinner
    pub fn small(mut self) -> Self {
        self.size = 16.0;
        self
    }

    /// Large spinner
    pub fn large(mut self) -> Self {
        self.size = 32.0;
        self
    }

    /// Show the spinner
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Widget for Spinner {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let color = self.color.unwrap_or(theme.tokens.primary);

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let time = ui.ctx().input(|i| i.time);
            let angle = (time * 2.0) as f32 % TAU;

            let center = rect.center();
            let radius = self.size / 2.0 - 2.0;
            let stroke_width = self.size / 8.0;

            // Draw arc
            let painter = ui.painter();

            // Background circle (faded)
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(stroke_width, color.gamma_multiply(0.2)),
            );

            // Spinning arc
            let arc_length = TAU * 0.75;
            let points: Vec<egui::Pos2> = (0..32)
                .map(|i| {
                    let t = i as f32 / 31.0;
                    let a = angle + t * arc_length;
                    egui::Pos2::new(center.x + radius * a.cos(), center.y + radius * a.sin())
                })
                .collect();

            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(stroke_width, color),
            ));

            // Request repaint for animation
            ui.ctx().request_repaint();
        }

        response
    }
}

/// A progress bar
pub struct ProgressBar {
    progress: f32,
    height: f32,
    show_text: bool,
    text_format: Option<Box<dyn Fn(f32) -> String>>,
    color: Option<egui::Color32>,
    indeterminate: bool,
}

impl ProgressBar {
    /// Create a new progress bar with given progress (0.0 - 1.0)
    pub fn new(progress: f32) -> Self {
        Self {
            progress: progress.clamp(0.0, 1.0),
            height: 8.0,
            show_text: false,
            text_format: None,
            color: None,
            indeterminate: false,
        }
    }

    /// Create an indeterminate progress bar
    pub fn indeterminate() -> Self {
        Self {
            progress: 0.0,
            height: 8.0,
            show_text: false,
            text_format: None,
            color: None,
            indeterminate: true,
        }
    }

    /// Set height
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Show percentage text
    pub fn show_text(mut self) -> Self {
        self.show_text = true;
        self
    }

    /// Custom text format
    pub fn text_format(mut self, f: impl Fn(f32) -> String + 'static) -> Self {
        self.text_format = Some(Box::new(f));
        self.show_text = true;
        self
    }

    /// Set custom color
    pub fn color(mut self, color: egui::Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Show the progress bar
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Widget for ProgressBar {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let color = self.color.unwrap_or(tokens.primary);

        // Use a lighter track color for light theme
        let track_color = if theme.is_dark {
            tokens.muted
        } else {
            tokens.border
        };

        ui.vertical(|ui| {
            let available_width = ui.available_width();

            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(available_width, self.height),
                egui::Sense::hover(),
            );

            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                let rounding = self.height / 2.0;

                // Background
                painter.rect_filled(rect, rounding, track_color);

                if self.indeterminate {
                    // Animated indeterminate bar
                    let time = ui.ctx().input(|i| i.time) as f32;
                    let bar_width = rect.width() * 0.3;
                    let position = (time * 1.5).sin() * 0.5 + 0.5;
                    let x = rect.left() + (rect.width() - bar_width) * position;

                    let bar_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(x, rect.top()),
                        Vec2::new(bar_width, self.height),
                    );

                    painter.rect_filled(bar_rect, rounding, color);
                    ui.ctx().request_repaint();
                } else {
                    // Determinate progress
                    let progress_width = rect.width() * self.progress;
                    if progress_width > 0.0 {
                        let progress_rect = egui::Rect::from_min_size(
                            rect.min,
                            Vec2::new(progress_width, self.height),
                        );
                        painter.rect_filled(progress_rect, rounding, color);
                    }
                }
            }

            // Text
            if self.show_text && !self.indeterminate {
                ui.add_space(tokens.spacing_xs);

                let text = if let Some(format) = self.text_format {
                    format(self.progress)
                } else {
                    format!("{:.0}%", self.progress * 100.0)
                };

                ui.label(
                    egui::RichText::new(text)
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                );
            }

            response
        })
        .inner
    }
}

/// Loading state wrapper
pub struct Loading<'a> {
    loading: bool,
    text: Option<&'a str>,
}

impl<'a> Loading<'a> {
    /// Create a new loading wrapper
    pub fn new(loading: bool) -> Self {
        Self {
            loading,
            text: None,
        }
    }

    /// Set loading text
    pub fn text(mut self, text: &'a str) -> Self {
        self.text = Some(text);
        self
    }

    /// Show content or loading state
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        if self.loading {
            let theme = current_theme();
            let tokens = &theme.tokens;

            ui.vertical_centered(|ui| {
                ui.add_space(tokens.spacing_lg);
                Spinner::new().show(ui);

                if let Some(text) = self.text {
                    ui.add_space(tokens.spacing_sm);
                    ui.label(
                        egui::RichText::new(text)
                            .size(tokens.font_size_sm)
                            .color(tokens.muted_foreground),
                    );
                }

                ui.add_space(tokens.spacing_lg);
            });
            None
        } else {
            Some(add_contents(ui))
        }
    }
}
