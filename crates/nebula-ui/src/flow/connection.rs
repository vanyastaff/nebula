//! Connection rendering for flow editor.
//!
//! Supports multiple edge types: Straight, Bezier, SmoothStep, and Smart (pathfinding).

use egui::{Pos2, Rect, Stroke, Ui, Vec2};

use crate::theme;

use super::types::{Connection, ConnectionId, DataType, PinKind};

/// Type of edge/connection path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeType {
    /// Straight line from source to target.
    Straight,
    /// Cubic bezier curve (default, like ReactFlow's 'default' and 'simplebezier').
    #[default]
    Bezier,
    /// Step path with 90-degree corners.
    Step,
    /// Step path with rounded corners (like ReactFlow's 'smoothstep').
    SmoothStep,
}

/// Style configuration for connection rendering.
#[derive(Debug, Clone)]
pub struct ConnectionStyle {
    /// Type of edge path.
    pub edge_type: EdgeType,
    /// Line width.
    pub width: f32,
    /// Control point distance factor (how "curvy" the bezier is).
    pub curve_factor: f32,
    /// Border radius for SmoothStep corners.
    pub border_radius: f32,
    /// Offset for SmoothStep path.
    pub step_offset: f32,
    /// Whether to show flow animation.
    pub animated: bool,
    /// Animation speed (pixels per second).
    pub animation_speed: f32,
    /// Dash pattern for animated connections [dash_length, gap_length].
    pub dash_pattern: [f32; 2],
    /// Padding around nodes for Smart routing.
    pub routing_padding: f32,
}

impl Default for ConnectionStyle {
    fn default() -> Self {
        Self {
            edge_type: EdgeType::Bezier,
            width: 2.0,
            curve_factor: 0.5,
            border_radius: 8.0,
            step_offset: 20.0,
            animated: false,
            animation_speed: 50.0,
            dash_pattern: [8.0, 4.0],
            routing_padding: 20.0,
        }
    }
}

/// State of a connection for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Normal connection.
    #[default]
    Normal,
    /// Connection is hovered.
    Hovered,
    /// Connection is selected.
    Selected,
    /// Connection is being created (preview).
    Creating,
    /// Connection has an error.
    Error,
    /// Connection is executing (data flowing through).
    Executing,
}

/// Renders connections between pins.
pub struct ConnectionRenderer {
    style: ConnectionStyle,
}

impl ConnectionRenderer {
    /// Creates a new connection renderer.
    pub fn new() -> Self {
        Self {
            style: ConnectionStyle::default(),
        }
    }

    /// Sets the style.
    pub fn style(mut self, style: ConnectionStyle) -> Self {
        self.style = style;
        self
    }

    /// Draws a connection between two points.
    pub fn draw(
        &self,
        ui: &mut Ui,
        from: Pos2,
        to: Pos2,
        data_type: &DataType,
        state: ConnectionState,
        zoom: f32,
    ) {
        let theme = theme::current_theme();
        let painter = ui.painter();

        let base_color = theme.color_for_data_type(data_type);
        let (color, width) = match state {
            ConnectionState::Normal => (base_color, self.style.width),
            ConnectionState::Hovered => (base_color, self.style.width * 1.5),
            ConnectionState::Selected => (theme.tokens.accent, self.style.width * 1.5),
            ConnectionState::Creating => (base_color.gamma_multiply(0.7), self.style.width),
            ConnectionState::Error => (theme.tokens.destructive, self.style.width * 1.5),
            ConnectionState::Executing => (theme.tokens.warning, self.style.width * 2.0),
        };

        let width = width * zoom;
        let stroke = Stroke::new(width, color);

        // Generate path points based on edge type
        let points = self.generate_path(from, to, zoom);

        if self.style.animated && state == ConnectionState::Executing {
            // Draw animated dashed line
            self.draw_animated_line(painter, &points, stroke, ui.input(|i| i.time));
        } else {
            // Draw solid line
            painter.add(egui::Shape::line(points.clone(), stroke));
        }

        // Draw glow for hovered/selected states
        if matches!(
            state,
            ConnectionState::Hovered | ConnectionState::Selected | ConnectionState::Executing
        ) {
            let glow_stroke = Stroke::new(width * 2.0, color.gamma_multiply(0.2));
            painter.add(egui::Shape::line(points, glow_stroke));
        }
    }

    /// Generates path points based on edge type.
    fn generate_path(&self, from: Pos2, to: Pos2, zoom: f32) -> Vec<Pos2> {
        match self.style.edge_type {
            EdgeType::Straight => self.straight_path(from, to),
            EdgeType::Bezier => self.bezier_path(from, to, zoom),
            EdgeType::Step => self.step_path(from, to, zoom),
            EdgeType::SmoothStep => self.smooth_step_path(from, to, zoom),
        }
    }

    /// Straight line path.
    fn straight_path(&self, from: Pos2, to: Pos2) -> Vec<Pos2> {
        vec![from, to]
    }

    /// Cubic bezier curve path.
    fn bezier_path(&self, from: Pos2, to: Pos2, zoom: f32) -> Vec<Pos2> {
        let (cp1, cp2) = self.calculate_bezier_control_points(from, to, zoom);
        self.bezier_points(from, cp1, cp2, to, 32)
    }

    /// Step path - orthogonal with 90-degree corners (no rounding).
    fn step_path(&self, from: Pos2, to: Pos2, zoom: f32) -> Vec<Pos2> {
        let offset = self.style.step_offset * zoom;
        let dx = to.x - from.x;
        let dy = to.y - from.y;

        if dx.abs() < 1.0 {
            return vec![from, to];
        }

        let mid_x = from.x + dx / 2.0;

        if dx < offset * 2.0 {
            let out_x = from.x + offset;
            let back_x = to.x - offset;
            let mid_y = if dy > 0.0 {
                from.y.max(to.y) + offset
            } else {
                from.y.min(to.y) - offset
            };

            vec![
                from,
                Pos2::new(out_x, from.y),
                Pos2::new(out_x, mid_y),
                Pos2::new(back_x, mid_y),
                Pos2::new(back_x, to.y),
                to,
            ]
        } else {
            vec![from, Pos2::new(mid_x, from.y), Pos2::new(mid_x, to.y), to]
        }
    }

    /// SmoothStep path - orthogonal with rounded corners.
    fn smooth_step_path(&self, from: Pos2, to: Pos2, zoom: f32) -> Vec<Pos2> {
        let offset = self.style.step_offset * zoom;
        let radius = self.style.border_radius * zoom;

        let dx = to.x - from.x;
        let dy = to.y - from.y;

        if dx.abs() < 1.0 {
            return vec![from, to];
        }

        let mid_x = from.x + dx / 2.0;

        if dx < offset * 2.0 {
            let out_x = from.x + offset;
            let back_x = to.x - offset;
            let mid_y = if dy > 0.0 {
                from.y.max(to.y) + offset
            } else {
                from.y.min(to.y) - offset
            };

            self.rounded_polyline(
                &[
                    from,
                    Pos2::new(out_x, from.y),
                    Pos2::new(out_x, mid_y),
                    Pos2::new(back_x, mid_y),
                    Pos2::new(back_x, to.y),
                    to,
                ],
                radius,
            )
        } else {
            self.rounded_polyline(
                &[from, Pos2::new(mid_x, from.y), Pos2::new(mid_x, to.y), to],
                radius,
            )
        }
    }

    /// Create a polyline with rounded corners.
    fn rounded_polyline(&self, points: &[Pos2], radius: f32) -> Vec<Pos2> {
        if points.len() < 3 || radius < 1.0 {
            return points.to_vec();
        }

        let mut result = Vec::new();
        result.push(points[0]);

        for i in 1..points.len() - 1 {
            let prev = points[i - 1];
            let curr = points[i];
            let next = points[i + 1];

            let dir_in = (curr - prev).normalized();
            let dir_out = (next - curr).normalized();

            let dist_in = (curr - prev).length();
            let dist_out = (next - curr).length();

            let max_radius = (dist_in.min(dist_out) / 2.0).min(radius);

            if max_radius < 1.0 {
                result.push(curr);
                continue;
            }

            let arc_start = curr - dir_in * max_radius;
            let arc_end = curr + dir_out * max_radius;

            result.push(arc_start);

            // Use fewer segments for cleaner paths (4 instead of 8)
            let segments = 4;
            for j in 1..segments {
                let t = j as f32 / segments as f32;
                let p = self.quadratic_bezier(arc_start, curr, arc_end, t);
                result.push(p);
            }

            result.push(arc_end);
        }

        result.push(*points.last().unwrap());
        result
    }

    fn quadratic_bezier(&self, p0: Pos2, p1: Pos2, p2: Pos2, t: f32) -> Pos2 {
        let mt = 1.0 - t;
        Pos2::new(
            mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
            mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
        )
    }

    /// Draws a connection preview from a pin to the cursor.
    pub fn draw_preview(
        &self,
        ui: &mut Ui,
        from: Pos2,
        to: Pos2,
        from_kind: PinKind,
        data_type: &DataType,
        valid: bool,
        zoom: f32,
    ) {
        let theme = theme::current_theme();
        let painter = ui.painter();

        let color = if valid {
            theme.color_for_data_type(data_type).gamma_multiply(0.7)
        } else {
            theme.tokens.destructive.gamma_multiply(0.5)
        };

        let width = self.style.width * zoom;
        let stroke = Stroke::new(width, color);

        let (start, end) = match from_kind {
            PinKind::Output => (from, to),
            PinKind::Input => (to, from),
        };

        let points = self.generate_path(start, end, zoom);
        self.draw_dashed_line(painter, &points, stroke);
    }

    /// Draws a connection from a Connection struct.
    pub fn draw_connection(
        &self,
        ui: &mut Ui,
        connection: &Connection,
        from_pos: Pos2,
        to_pos: Pos2,
        state: ConnectionState,
        zoom: f32,
    ) {
        self.draw(ui, from_pos, to_pos, &connection.data_type, state, zoom);
    }

    fn calculate_bezier_control_points(&self, from: Pos2, to: Pos2, zoom: f32) -> (Pos2, Pos2) {
        let dx = (to.x - from.x).abs();
        let min_curve = 50.0 * zoom;
        let curve_distance = (dx * self.style.curve_factor).max(min_curve);

        let cp1 = Pos2::new(from.x + curve_distance, from.y);
        let cp2 = Pos2::new(to.x - curve_distance, to.y);

        (cp1, cp2)
    }

    fn bezier_points(&self, p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, segments: usize) -> Vec<Pos2> {
        (0..=segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                self.cubic_bezier(p0, p1, p2, p3, t)
            })
            .collect()
    }

    fn cubic_bezier(&self, p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        Pos2::new(
            mt3 * p0.x + 3.0 * mt2 * t * p1.x + 3.0 * mt * t2 * p2.x + t3 * p3.x,
            mt3 * p0.y + 3.0 * mt2 * t * p1.y + 3.0 * mt * t2 * p2.y + t3 * p3.y,
        )
    }

    fn draw_dashed_line(&self, painter: &egui::Painter, points: &[Pos2], stroke: Stroke) {
        let dash_len = self.style.dash_pattern[0];
        let gap_len = self.style.dash_pattern[1];
        let pattern_len = dash_len + gap_len;

        let mut accumulated = 0.0;
        let mut dash_points = Vec::new();
        let mut in_dash = true;

        for window in points.windows(2) {
            let start = window[0];
            let end = window[1];
            let segment_len = start.distance(end);

            if segment_len < 0.001 {
                continue;
            }

            let dir = (end - start) / segment_len;
            let mut pos = 0.0;

            while pos < segment_len {
                let remaining_in_pattern = if in_dash {
                    dash_len - (accumulated % pattern_len)
                } else {
                    gap_len - ((accumulated - dash_len) % pattern_len)
                };

                let step = remaining_in_pattern.min(segment_len - pos);
                let point = start + dir * (pos + step);

                if in_dash {
                    if dash_points.is_empty() {
                        dash_points.push(start + dir * pos);
                    }
                    dash_points.push(point);
                } else if !dash_points.is_empty() {
                    painter.add(egui::Shape::line(dash_points.clone(), stroke));
                    dash_points.clear();
                }

                pos += step;
                accumulated += step;

                if accumulated >= pattern_len {
                    accumulated -= pattern_len;
                }

                let current_in_pattern = accumulated % pattern_len;
                in_dash = current_in_pattern < dash_len;
            }
        }

        if !dash_points.is_empty() {
            painter.add(egui::Shape::line(dash_points, stroke));
        }
    }

    fn draw_animated_line(
        &self,
        painter: &egui::Painter,
        points: &[Pos2],
        stroke: Stroke,
        time: f64,
    ) {
        let offset = (time * self.style.animation_speed as f64) as f32;
        let dash_len = self.style.dash_pattern[0];
        let gap_len = self.style.dash_pattern[1];
        let pattern_len = dash_len + gap_len;

        let mut accumulated = offset % pattern_len;
        let mut dash_points = Vec::new();
        let mut in_dash = accumulated < dash_len;

        for window in points.windows(2) {
            let start = window[0];
            let end = window[1];
            let segment_len = start.distance(end);

            if segment_len < 0.001 {
                continue;
            }

            let dir = (end - start) / segment_len;
            let mut pos = 0.0;

            while pos < segment_len {
                let remaining_in_pattern = if in_dash {
                    dash_len - (accumulated % pattern_len).min(dash_len)
                } else {
                    pattern_len - accumulated
                };

                let step = remaining_in_pattern.min(segment_len - pos);
                let point = start + dir * (pos + step);

                if in_dash {
                    if dash_points.is_empty() {
                        dash_points.push(start + dir * pos);
                    }
                    dash_points.push(point);
                } else if !dash_points.is_empty() {
                    painter.add(egui::Shape::line(dash_points.clone(), stroke));
                    dash_points.clear();
                }

                pos += step;
                accumulated += step;

                if accumulated >= pattern_len {
                    accumulated -= pattern_len;
                }

                in_dash = accumulated < dash_len;
            }
        }

        if !dash_points.is_empty() {
            painter.add(egui::Shape::line(dash_points, stroke));
        }
    }
}

impl Default for ConnectionRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Checks if a point is near a connection path.
pub fn point_near_connection(
    point: Pos2,
    from: Pos2,
    to: Pos2,
    curve_factor: f32,
    threshold: f32,
) -> bool {
    let dx = (to.x - from.x).abs();
    let curve_distance = (dx * curve_factor).max(50.0);

    let cp1 = Pos2::new(from.x + curve_distance, from.y);
    let cp2 = Pos2::new(to.x - curve_distance, to.y);

    for i in 0..=32 {
        let t = i as f32 / 32.0;
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        let curve_point = Pos2::new(
            mt3 * from.x + 3.0 * mt2 * t * cp1.x + 3.0 * mt * t2 * cp2.x + t3 * to.x,
            mt3 * from.y + 3.0 * mt2 * t * cp1.y + 3.0 * mt * t2 * cp2.y + t3 * to.y,
        );

        if point.distance(curve_point) < threshold {
            return true;
        }
    }

    false
}

/// Gets the ID of a connection.
pub fn connection_id(connection: &Connection) -> ConnectionId {
    connection.id
}
