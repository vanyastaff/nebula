//! Progress indicator components.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// Progress bar size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressSize {
    /// Small (4px height)
    Sm,
    /// Medium (8px height)
    #[default]
    Md,
    /// Large (12px height)
    Lg,
}

impl ProgressSize {
    fn height(&self) -> f32 {
        match self {
            Self::Sm => 4.0,
            Self::Md => 8.0,
            Self::Lg => 12.0,
        }
    }
}

/// A progress bar component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Progress;
///
/// ui.add(Progress::new(0.75).label("75% complete"));
/// ```
pub struct Progress<'a> {
    value: f32,
    label: Option<&'a str>,
    show_value: bool,
    size: ProgressSize,
    indeterminate: bool,
}

impl<'a> Progress<'a> {
    /// Create a new progress bar with value between 0.0 and 1.0
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            label: None,
            show_value: false,
            size: ProgressSize::Md,
            indeterminate: false,
        }
    }

    /// Create an indeterminate progress bar
    pub fn indeterminate() -> Self {
        Self {
            value: 0.0,
            label: None,
            show_value: false,
            size: ProgressSize::Md,
            indeterminate: true,
        }
    }

    /// Set a label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Show the percentage value
    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Set the size
    pub fn size(mut self, size: ProgressSize) -> Self {
        self.size = size;
        self
    }

    /// Show the progress bar
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Progress<'a> {
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

                    if self.show_value && !self.indeterminate {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{}%", (self.value * 100.0) as i32))
                                    .size(tokens.font_size_sm)
                                    .color(tokens.muted_foreground),
                            );
                        });
                    }
                });
                ui.add_space(tokens.spacing_xs);
            }

            // Progress bar
            let height = self.size.height();
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), height),
                egui::Sense::hover(),
            );

            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                let rounding = height / 2.0;

                // Background track
                painter.rect_filled(rect, rounding, tokens.muted);

                if self.indeterminate {
                    // Animated indeterminate bar
                    let time = ui.ctx().input(|i| i.time);
                    let cycle = (time * 2.0).sin() * 0.5 + 0.5;
                    let bar_width = rect.width() * 0.3;
                    let offset = (rect.width() - bar_width) * cycle as f32;

                    let bar_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(rect.left() + offset, rect.top()),
                        Vec2::new(bar_width, height),
                    );
                    painter.rect_filled(bar_rect, rounding, tokens.primary);

                    // Request repaint for animation
                    ui.ctx().request_repaint();
                } else {
                    // Determinate progress
                    let fill_width = rect.width() * self.value;
                    if fill_width > 0.0 {
                        let fill_rect =
                            egui::Rect::from_min_size(rect.min, Vec2::new(fill_width, height));
                        painter.rect_filled(fill_rect, rounding, tokens.primary);
                    }
                }
            }

            response
        })
        .inner
    }
}

/// A circular progress indicator
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::CircularProgress;
///
/// ui.add(CircularProgress::new(0.5).size(48.0));
/// ```
pub struct CircularProgress {
    value: f32,
    size: f32,
    stroke_width: f32,
    indeterminate: bool,
}

impl CircularProgress {
    /// Create a new circular progress
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            size: 32.0,
            stroke_width: 4.0,
            indeterminate: false,
        }
    }

    /// Create an indeterminate circular progress
    pub fn indeterminate() -> Self {
        Self {
            value: 0.0,
            size: 32.0,
            stroke_width: 4.0,
            indeterminate: true,
        }
    }

    /// Set the size
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Set the stroke width
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width;
        self
    }

    /// Show the progress indicator
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Widget for CircularProgress {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let center = rect.center();
            let radius = (self.size - self.stroke_width) / 2.0;

            // Background circle
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(self.stroke_width, tokens.muted),
            );

            if self.indeterminate {
                // Animated spinner
                let time = ui.ctx().input(|i| i.time);
                let start_angle = (time * 3.0) as f32;
                let sweep = std::f32::consts::PI * 1.5;

                draw_arc(
                    painter,
                    center,
                    radius,
                    start_angle,
                    sweep,
                    egui::Stroke::new(self.stroke_width, tokens.primary),
                );

                ui.ctx().request_repaint();
            } else {
                // Determinate progress
                let start_angle = -std::f32::consts::FRAC_PI_2;
                let sweep = std::f32::consts::TAU * self.value;

                if self.value > 0.0 {
                    draw_arc(
                        painter,
                        center,
                        radius,
                        start_angle,
                        sweep,
                        egui::Stroke::new(self.stroke_width, tokens.primary),
                    );
                }
            }
        }

        response
    }
}

/// Draw an arc on the painter
fn draw_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    start_angle: f32,
    sweep: f32,
    stroke: egui::Stroke,
) {
    let segments = (sweep.abs() * 20.0).ceil() as usize;
    if segments == 0 {
        return;
    }

    let points: Vec<egui::Pos2> = (0..=segments)
        .map(|i| {
            let angle = start_angle + sweep * (i as f32 / segments as f32);
            egui::Pos2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect();

    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
}
