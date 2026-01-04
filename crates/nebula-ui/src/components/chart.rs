//! Chart components for data visualization.

use crate::theme::current_theme;
use egui::{Color32, Pos2, Rect, Response, Stroke, StrokeKind, Ui, Vec2};

/// Data point for charts
#[derive(Clone, Debug)]
pub struct DataPoint {
    /// X value
    pub x: f64,
    /// Y value
    pub y: f64,
    /// Optional label
    pub label: Option<String>,
}

impl DataPoint {
    /// Create a new data point
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y, label: None }
    }

    /// Create with label
    pub fn with_label(x: f64, y: f64, label: impl Into<String>) -> Self {
        Self {
            x,
            y,
            label: Some(label.into()),
        }
    }
}

/// Data series for charts
#[derive(Clone, Debug)]
pub struct Series {
    /// Series name
    pub name: String,
    /// Data points
    pub data: Vec<DataPoint>,
    /// Series color
    pub color: Option<Color32>,
}

impl Series {
    /// Create a new series
    pub fn new(name: impl Into<String>, data: Vec<DataPoint>) -> Self {
        Self {
            name: name.into(),
            data,
            color: None,
        }
    }

    /// Set series color
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
}

/// Line chart component
///
/// # Example
///
/// ```rust,ignore
/// let data = vec![
///     DataPoint::new(0.0, 10.0),
///     DataPoint::new(1.0, 20.0),
///     DataPoint::new(2.0, 15.0),
/// ];
/// LineChart::new(vec![Series::new("Sales", data)])
///     .show(ui);
/// ```
pub struct LineChart {
    series: Vec<Series>,
    width: f32,
    height: f32,
    show_grid: bool,
    show_legend: bool,
    show_points: bool,
    x_label: Option<String>,
    y_label: Option<String>,
    title: Option<String>,
}

impl LineChart {
    /// Create a new line chart
    pub fn new(series: Vec<Series>) -> Self {
        Self {
            series,
            width: 400.0,
            height: 250.0,
            show_grid: true,
            show_legend: true,
            show_points: true,
            x_label: None,
            y_label: None,
            title: None,
        }
    }

    /// Set chart size
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Hide grid
    pub fn hide_grid(mut self) -> Self {
        self.show_grid = false;
        self
    }

    /// Hide legend
    pub fn hide_legend(mut self) -> Self {
        self.show_legend = false;
        self
    }

    /// Hide data points
    pub fn hide_points(mut self) -> Self {
        self.show_points = false;
        self
    }

    /// Set X axis label
    pub fn x_label(mut self, label: impl Into<String>) -> Self {
        self.x_label = Some(label.into());
        self
    }

    /// Set Y axis label
    pub fn y_label(mut self, label: impl Into<String>) -> Self {
        self.y_label = Some(label.into());
        self
    }

    /// Set chart title
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Show the chart
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);

            // Background
            painter.rect_filled(rect, tokens.radius_md, tokens.card);
            painter.rect_stroke(
                rect,
                tokens.radius_md,
                Stroke::new(1.0, tokens.border),
                StrokeKind::Inside,
            );

            // Chart area (with margins for labels)
            let margin = 40.0;
            let chart_rect = Rect::from_min_max(
                Pos2::new(rect.min.x + margin, rect.min.y + 20.0),
                Pos2::new(rect.max.x - 20.0, rect.max.y - margin),
            );

            // Calculate data bounds
            let (min_x, max_x, min_y, max_y) = self.calculate_bounds();

            // Draw grid
            if self.show_grid {
                self.draw_grid(&painter, chart_rect, min_x, max_x, min_y, max_y, tokens);
            }

            // Draw series
            let colors = [
                tokens.primary,
                tokens.accent,
                tokens.destructive,
                Color32::from_rgb(34, 197, 94),  // Green
                Color32::from_rgb(168, 85, 247), // Purple
                Color32::from_rgb(249, 115, 22), // Orange
            ];

            for (i, series) in self.series.iter().enumerate() {
                let color = series.color.unwrap_or(colors[i % colors.len()]);
                self.draw_series(
                    &painter, chart_rect, series, min_x, max_x, min_y, max_y, color,
                );
            }

            // Title
            if let Some(title) = &self.title {
                painter.text(
                    Pos2::new(rect.center().x, rect.min.y + 10.0),
                    egui::Align2::CENTER_CENTER,
                    title,
                    egui::FontId::proportional(tokens.font_size_md),
                    tokens.foreground,
                );
            }

            // X axis label
            if let Some(label) = &self.x_label {
                painter.text(
                    Pos2::new(chart_rect.center().x, rect.max.y - 10.0),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(tokens.font_size_xs),
                    tokens.muted_foreground,
                );
            }

            // Legend
            if self.show_legend && !self.series.is_empty() {
                let mut legend_x = chart_rect.max.x - 10.0;
                for (i, series) in self.series.iter().enumerate().rev() {
                    let color = series.color.unwrap_or(colors[i % colors.len()]);
                    let text_width = series.name.len() as f32 * 6.0;

                    legend_x -= text_width + 20.0;

                    painter.circle_filled(Pos2::new(legend_x, chart_rect.min.y - 8.0), 4.0, color);

                    painter.text(
                        Pos2::new(legend_x + 8.0, chart_rect.min.y - 8.0),
                        egui::Align2::LEFT_CENTER,
                        &series.name,
                        egui::FontId::proportional(tokens.font_size_xs),
                        tokens.muted_foreground,
                    );
                }
            }
        }

        response
    }

    fn calculate_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;

        for series in &self.series {
            for point in &series.data {
                min_x = min_x.min(point.x);
                max_x = max_x.max(point.x);
                min_y = min_y.min(point.y);
                max_y = max_y.max(point.y);
            }
        }

        // Add some padding
        let y_range = max_y - min_y;
        min_y -= y_range * 0.1;
        max_y += y_range * 0.1;

        (min_x, max_x, min_y.max(0.0), max_y)
    }

    fn draw_grid(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        tokens: &crate::theme::ThemeTokens,
    ) {
        let grid_color = tokens.border.gamma_multiply(0.5);
        let num_lines = 5;

        // Horizontal grid lines
        for i in 0..=num_lines {
            let y = rect.min.y + (rect.height() * i as f32 / num_lines as f32);
            painter.line_segment(
                [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                Stroke::new(1.0, grid_color),
            );

            // Y axis labels
            let value = max_y - (max_y - min_y) * i as f64 / num_lines as f64;
            painter.text(
                Pos2::new(rect.min.x - 5.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0}", value),
                egui::FontId::proportional(tokens.font_size_xs),
                tokens.muted_foreground,
            );
        }

        // Vertical grid lines
        for i in 0..=num_lines {
            let x = rect.min.x + (rect.width() * i as f32 / num_lines as f32);
            painter.line_segment(
                [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                Stroke::new(1.0, grid_color),
            );

            // X axis labels
            let value = min_x + (max_x - min_x) * i as f64 / num_lines as f64;
            painter.text(
                Pos2::new(x, rect.max.y + 5.0),
                egui::Align2::CENTER_TOP,
                format!("{:.0}", value),
                egui::FontId::proportional(tokens.font_size_xs),
                tokens.muted_foreground,
            );
        }
    }

    fn draw_series(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        series: &Series,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        color: Color32,
    ) {
        if series.data.is_empty() {
            return;
        }

        let x_range = max_x - min_x;
        let y_range = max_y - min_y;

        let to_screen = |point: &DataPoint| -> Pos2 {
            let x = if x_range > 0.0 {
                rect.min.x + ((point.x - min_x) / x_range) as f32 * rect.width()
            } else {
                rect.center().x
            };
            let y = if y_range > 0.0 {
                rect.max.y - ((point.y - min_y) / y_range) as f32 * rect.height()
            } else {
                rect.center().y
            };
            Pos2::new(x, y)
        };

        // Draw line
        let points: Vec<Pos2> = series.data.iter().map(to_screen).collect();
        for i in 1..points.len() {
            painter.line_segment([points[i - 1], points[i]], Stroke::new(2.0, color));
        }

        // Draw points
        if self.show_points {
            for point in &points {
                painter.circle_filled(*point, 4.0, color);
                painter.circle_stroke(*point, 4.0, Stroke::new(2.0, Color32::WHITE));
            }
        }
    }
}

/// Bar chart component
pub struct BarChart {
    data: Vec<(String, f64)>,
    width: f32,
    height: f32,
    horizontal: bool,
    show_values: bool,
    color: Option<Color32>,
}

impl BarChart {
    /// Create a new bar chart
    pub fn new(data: Vec<(String, f64)>) -> Self {
        Self {
            data,
            width: 400.0,
            height: 250.0,
            horizontal: false,
            show_values: true,
            color: None,
        }
    }

    /// Set chart size
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Make horizontal bar chart
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Hide value labels
    pub fn hide_values(mut self) -> Self {
        self.show_values = false;
        self
    }

    /// Set bar color
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Show the chart
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);

            // Background
            painter.rect_filled(rect, tokens.radius_md, tokens.card);
            painter.rect_stroke(
                rect,
                tokens.radius_md,
                Stroke::new(1.0, tokens.border),
                StrokeKind::Inside,
            );

            let margin = 50.0;
            let chart_rect = Rect::from_min_max(
                Pos2::new(rect.min.x + margin, rect.min.y + 20.0),
                Pos2::new(rect.max.x - 20.0, rect.max.y - 30.0),
            );

            let max_value = self.data.iter().map(|(_, v)| *v).fold(0.0, f64::max);
            let color = self.color.unwrap_or(tokens.primary);
            let bar_gap = 4.0;

            if self.horizontal {
                let bar_height = (chart_rect.height() - bar_gap * (self.data.len() - 1) as f32)
                    / self.data.len() as f32;

                for (i, (label, value)) in self.data.iter().enumerate() {
                    let y = chart_rect.min.y + i as f32 * (bar_height + bar_gap);
                    let bar_width = if max_value > 0.0 {
                        (*value / max_value) as f32 * chart_rect.width()
                    } else {
                        0.0
                    };

                    let bar_rect = Rect::from_min_size(
                        Pos2::new(chart_rect.min.x, y),
                        Vec2::new(bar_width, bar_height),
                    );

                    painter.rect_filled(bar_rect, tokens.radius_sm, color);

                    // Label
                    painter.text(
                        Pos2::new(chart_rect.min.x - 5.0, y + bar_height / 2.0),
                        egui::Align2::RIGHT_CENTER,
                        label,
                        egui::FontId::proportional(tokens.font_size_xs),
                        tokens.foreground,
                    );

                    // Value
                    if self.show_values {
                        painter.text(
                            Pos2::new(chart_rect.min.x + bar_width + 5.0, y + bar_height / 2.0),
                            egui::Align2::LEFT_CENTER,
                            format!("{:.0}", value),
                            egui::FontId::proportional(tokens.font_size_xs),
                            tokens.muted_foreground,
                        );
                    }
                }
            } else {
                let bar_width = (chart_rect.width() - bar_gap * (self.data.len() - 1) as f32)
                    / self.data.len() as f32;

                for (i, (label, value)) in self.data.iter().enumerate() {
                    let x = chart_rect.min.x + i as f32 * (bar_width + bar_gap);
                    let bar_height = if max_value > 0.0 {
                        (*value / max_value) as f32 * chart_rect.height()
                    } else {
                        0.0
                    };

                    let bar_rect = Rect::from_min_size(
                        Pos2::new(x, chart_rect.max.y - bar_height),
                        Vec2::new(bar_width, bar_height),
                    );

                    painter.rect_filled(bar_rect, tokens.radius_sm, color);

                    // Label
                    painter.text(
                        Pos2::new(x + bar_width / 2.0, chart_rect.max.y + 10.0),
                        egui::Align2::CENTER_TOP,
                        label,
                        egui::FontId::proportional(tokens.font_size_xs),
                        tokens.foreground,
                    );

                    // Value
                    if self.show_values {
                        painter.text(
                            Pos2::new(x + bar_width / 2.0, chart_rect.max.y - bar_height - 5.0),
                            egui::Align2::CENTER_BOTTOM,
                            format!("{:.0}", value),
                            egui::FontId::proportional(tokens.font_size_xs),
                            tokens.muted_foreground,
                        );
                    }
                }
            }
        }

        response
    }
}

/// Pie chart component
pub struct PieChart {
    data: Vec<(String, f64, Option<Color32>)>,
    size: f32,
    show_legend: bool,
    show_labels: bool,
    donut: bool,
    donut_ratio: f32,
}

impl PieChart {
    /// Create a new pie chart
    pub fn new(data: Vec<(String, f64)>) -> Self {
        Self {
            data: data.into_iter().map(|(l, v)| (l, v, None)).collect(),
            size: 200.0,
            show_legend: true,
            show_labels: true,
            donut: false,
            donut_ratio: 0.5,
        }
    }

    /// Create with custom colors
    pub fn with_colors(data: Vec<(String, f64, Color32)>) -> Self {
        Self {
            data: data.into_iter().map(|(l, v, c)| (l, v, Some(c))).collect(),
            size: 200.0,
            show_legend: true,
            show_labels: true,
            donut: false,
            donut_ratio: 0.5,
        }
    }

    /// Set chart size
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Hide legend
    pub fn hide_legend(mut self) -> Self {
        self.show_legend = false;
        self
    }

    /// Hide labels
    pub fn hide_labels(mut self) -> Self {
        self.show_labels = false;
        self
    }

    /// Make donut chart
    pub fn donut(mut self) -> Self {
        self.donut = true;
        self
    }

    /// Set donut hole ratio (0.0 - 1.0)
    pub fn donut_ratio(mut self, ratio: f32) -> Self {
        self.donut = true;
        self.donut_ratio = ratio.clamp(0.1, 0.9);
        self
    }

    /// Show the chart
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let legend_width = if self.show_legend { 100.0 } else { 0.0 };
        let total_width = self.size + legend_width;

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(total_width, self.size), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);

            let center = Pos2::new(rect.min.x + self.size / 2.0, rect.center().y);
            let radius = self.size / 2.0 - 10.0;

            let total: f64 = self.data.iter().map(|(_, v, _)| *v).sum();

            let colors = [
                tokens.primary,
                tokens.accent,
                Color32::from_rgb(34, 197, 94),
                Color32::from_rgb(168, 85, 247),
                Color32::from_rgb(249, 115, 22),
                Color32::from_rgb(236, 72, 153),
                Color32::from_rgb(14, 165, 233),
            ];

            let mut start_angle = -std::f32::consts::FRAC_PI_2;

            for (i, (label, value, custom_color)) in self.data.iter().enumerate() {
                if *value <= 0.0 || total <= 0.0 {
                    continue;
                }

                let color = custom_color.unwrap_or(colors[i % colors.len()]);
                let sweep = (*value / total) as f32 * std::f32::consts::TAU;

                // Draw pie slice
                self.draw_arc(&painter, center, radius, start_angle, sweep, color);

                // Draw label
                if self.show_labels {
                    let mid_angle = start_angle + sweep / 2.0;
                    let label_radius = radius * 0.7;
                    let label_pos = Pos2::new(
                        center.x + label_radius * mid_angle.cos(),
                        center.y + label_radius * mid_angle.sin(),
                    );

                    let percentage = (*value / total * 100.0) as i32;
                    painter.text(
                        label_pos,
                        egui::Align2::CENTER_CENTER,
                        format!("{}%", percentage),
                        egui::FontId::proportional(tokens.font_size_xs),
                        Color32::WHITE,
                    );
                }

                start_angle += sweep;
            }

            // Donut hole
            if self.donut {
                painter.circle_filled(center, radius * self.donut_ratio, tokens.card);
            }

            // Legend
            if self.show_legend {
                let legend_x = rect.min.x + self.size + 10.0;
                let mut legend_y = rect.min.y + 20.0;

                for (i, (label, value, custom_color)) in self.data.iter().enumerate() {
                    let color = custom_color.unwrap_or(colors[i % colors.len()]);

                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(legend_x, legend_y - 4.0), Vec2::splat(8.0)),
                        2.0,
                        color,
                    );

                    painter.text(
                        Pos2::new(legend_x + 12.0, legend_y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(tokens.font_size_xs),
                        tokens.foreground,
                    );

                    legend_y += 20.0;
                }
            }
        }

        response
    }

    fn draw_arc(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        radius: f32,
        start_angle: f32,
        sweep: f32,
        color: Color32,
    ) {
        let inner_radius = if self.donut {
            radius * self.donut_ratio
        } else {
            0.0
        };

        let segments = (sweep.abs() * 20.0).max(8.0) as usize;
        let mut points = Vec::with_capacity(segments * 2 + 2);

        // Outer arc
        for i in 0..=segments {
            let angle = start_angle + sweep * i as f32 / segments as f32;
            points.push(Pos2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }

        // Inner arc (reverse)
        for i in (0..=segments).rev() {
            let angle = start_angle + sweep * i as f32 / segments as f32;
            points.push(Pos2::new(
                center.x + inner_radius * angle.cos(),
                center.y + inner_radius * angle.sin(),
            ));
        }

        painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

/// Sparkline - minimal inline chart
pub struct Sparkline {
    data: Vec<f64>,
    width: f32,
    height: f32,
    color: Option<Color32>,
    show_area: bool,
}

impl Sparkline {
    /// Create a new sparkline
    pub fn new(data: Vec<f64>) -> Self {
        Self {
            data,
            width: 100.0,
            height: 24.0,
            color: None,
            show_area: false,
        }
    }

    /// Set size
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Set color
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Show filled area under line
    pub fn area(mut self) -> Self {
        self.show_area = true;
        self
    }

    /// Show the sparkline
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;
        let color = self.color.unwrap_or(tokens.primary);

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), egui::Sense::hover());

        if ui.is_rect_visible(rect) && !self.data.is_empty() {
            let painter = ui.painter_at(rect);

            let min_val = self.data.iter().cloned().fold(f64::MAX, f64::min);
            let max_val = self.data.iter().cloned().fold(f64::MIN, f64::max);
            let range = max_val - min_val;

            let points: Vec<Pos2> = self
                .data
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let x = rect.min.x
                        + (i as f32 / (self.data.len() - 1).max(1) as f32) * rect.width();
                    let y = if range > 0.0 {
                        rect.max.y - ((v - min_val) / range) as f32 * rect.height()
                    } else {
                        rect.center().y
                    };
                    Pos2::new(x, y)
                })
                .collect();

            // Area fill
            if self.show_area {
                let mut area_points = points.clone();
                area_points.push(Pos2::new(rect.max.x, rect.max.y));
                area_points.push(Pos2::new(rect.min.x, rect.max.y));
                painter.add(egui::Shape::convex_polygon(
                    area_points,
                    color.gamma_multiply(0.2),
                    Stroke::NONE,
                ));
            }

            // Line
            for i in 1..points.len() {
                painter.line_segment([points[i - 1], points[i]], Stroke::new(1.5, color));
            }
        }

        response
    }
}
