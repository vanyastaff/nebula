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
    /// Cubic bezier curve (default).
    #[default]
    Bezier,
    /// Step path with rounded corners.
    SmoothStep,
    /// Smart path that avoids obstacles (nodes).
    Smart,
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

/// Bounds of obstacles for routing calculations.
#[derive(Debug, Clone, Copy)]
struct ObstacleBounds {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

/// Renders connections between pins.
pub struct ConnectionRenderer {
    style: ConnectionStyle,
    /// Node rectangles for smart routing (screen space).
    obstacles: Vec<Rect>,
}

impl ConnectionRenderer {
    /// Creates a new connection renderer.
    pub fn new() -> Self {
        Self {
            style: ConnectionStyle::default(),
            obstacles: Vec::new(),
        }
    }

    /// Sets the style.
    pub fn style(mut self, style: ConnectionStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets obstacles (node rects) for smart routing.
    pub fn obstacles(mut self, obstacles: Vec<Rect>) -> Self {
        self.obstacles = obstacles;
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
            EdgeType::SmoothStep => self.smooth_step_path(from, to, zoom),
            EdgeType::Smart => self.smart_path(from, to, zoom),
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

    /// Smart path that avoids obstacles using optimized pathfinding.
    fn smart_path(&self, from: Pos2, to: Pos2, zoom: f32) -> Vec<Pos2> {
        let padding = self.style.routing_padding * zoom;
        let lead_out = self.style.step_offset * zoom;
        let radius = self.style.border_radius * zoom;

        // Create lead points (horizontal exit from source, horizontal entry to target)
        let from_lead = Pos2::new(from.x + lead_out, from.y);
        let to_lead = Pos2::new(to.x - lead_out, to.y);

        // Expand obstacles by padding once
        let expanded_obstacles: Vec<Rect> = self
            .obstacles
            .iter()
            .map(|obs| obs.expand(padding))
            .collect();

        // Strategy 1: Try simple orthogonal paths (90% of cases, instant)
        if let Some(path) =
            self.try_simple_orthogonal_paths(from, from_lead, to_lead, to, &expanded_obstacles)
        {
            let simplified = self.simplify_path(&path);
            return self.rounded_polyline(&simplified, radius);
        }

        // Strategy 2: Fall back to grid-based A* for complex cases
        let path = self.grid_astar_path(from_lead, to_lead, &expanded_obstacles);

        let mut waypoints = vec![from];
        waypoints.extend(path);
        waypoints.push(to);

        let simplified = self.simplify_path(&waypoints);
        self.rounded_polyline(&simplified, radius)
    }

    /// Calculate bounds of all obstacles only (not including start/end points).
    fn calculate_obstacle_bounds(&self, obstacles: &[Rect]) -> ObstacleBounds {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;

        for obs in obstacles {
            min_x = min_x.min(obs.left());
            max_x = max_x.max(obs.right());
            min_y = min_y.min(obs.top());
            max_y = max_y.max(obs.bottom());
        }

        // Handle empty obstacles case
        if obstacles.is_empty() {
            return ObstacleBounds {
                min_x: 0.0,
                max_x: 0.0,
                min_y: 0.0,
                max_y: 0.0,
            };
        }

        ObstacleBounds {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    /// Try simple orthogonal paths before resorting to A*.
    /// Returns shortest valid path or None.
    fn try_simple_orthogonal_paths(
        &self,
        from: Pos2,
        from_lead: Pos2,
        to_lead: Pos2,
        to: Pos2,
        obstacles: &[Rect],
    ) -> Option<Vec<Pos2>> {
        let bounds = self.calculate_obstacle_bounds(obstacles);
        let offset = 30.0;

        // === DEBUG: Log input data ===
        #[cfg(debug_assertions)]
        {
            eprintln!("\n=== Smart Routing Debug ===");
            eprintln!("From: ({:.1}, {:.1})", from.x, from.y);
            eprintln!("From_lead: ({:.1}, {:.1})", from_lead.x, from_lead.y);
            eprintln!("To_lead: ({:.1}, {:.1})", to_lead.x, to_lead.y);
            eprintln!("To: ({:.1}, {:.1})", to.x, to.y);
            eprintln!("Obstacles count: {}", obstacles.len());
            for (i, obs) in obstacles.iter().enumerate() {
                eprintln!(
                    "  Obstacle {}: x=[{:.1}..{:.1}], y=[{:.1}..{:.1}]",
                    i,
                    obs.left(),
                    obs.right(),
                    obs.top(),
                    obs.bottom()
                );
            }
            eprintln!(
                "Bounds: x=[{:.1}..{:.1}], y=[{:.1}..{:.1}]",
                bounds.min_x, bounds.max_x, bounds.min_y, bounds.max_y
            );
        }

        let mut candidates: Vec<Vec<Pos2>> = Vec::new();

        // PRIORITY 1: Direct paths (no intermediate turns)

        // Path 1a: Direct horizontal then vertical (L-shape, most common case)
        // This goes: from -> from_lead -> corner at (to_lead.x, from_lead.y) -> to_lead -> to
        if to_lead.x >= from_lead.x {
            candidates.push(vec![
                from,
                from_lead,
                Pos2::new(to_lead.x, from_lead.y),
                to_lead,
                to,
            ]);
        }

        // Path 1b: Direct vertical then horizontal (reverse L-shape)
        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(from_lead.x, to_lead.y),
            to_lead,
            to,
        ]);

        // PRIORITY 2: Simple middle path (S-shape through center)
        let mid_x = (from_lead.x + to_lead.x) / 2.0;
        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(mid_x, from_lead.y),
            Pos2::new(mid_x, to_lead.y),
            to_lead,
            to,
        ]);

        // PRIORITY 3: Route above/below ALL obstacles (clean horizontal routes)
        let route_above = bounds.min_y - offset;
        let route_below = bounds.max_y + offset;

        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(from_lead.x, route_above),
            Pos2::new(to_lead.x, route_above),
            to_lead,
            to,
        ]);

        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(from_lead.x, route_below),
            Pos2::new(to_lead.x, route_below),
            to_lead,
            to,
        ]);

        // PRIORITY 4: Route left/right of ALL obstacles
        let route_right = bounds.max_x + offset;
        let route_left = bounds.min_x - offset;

        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(route_right, from_lead.y),
            Pos2::new(route_right, to_lead.y),
            to_lead,
            to,
        ]);

        candidates.push(vec![
            from,
            from_lead,
            Pos2::new(route_left, from_lead.y),
            Pos2::new(route_left, to_lead.y),
            to_lead,
            to,
        ]);

        // === DEBUG: Check each candidate ===
        #[cfg(debug_assertions)]
        {
            let path_names = [
                "Direct L (H->V)",
                "Direct L (V->H)",
                "S-shape middle",
                "Route above",
                "Route below",
                "Route right",
                "Route left",
            ];

            eprintln!("\nChecking {} candidates:", candidates.len());
            for (i, path) in candidates.iter().enumerate() {
                let is_clear = self.is_path_clear(path, obstacles);
                let len = self.calc_path_length(path);
                let name = path_names.get(i).unwrap_or(&"Unknown");
                eprintln!("  [{i}] {name}: len={len:.1}, clear={is_clear}");

                if !is_clear {
                    // Show which segment is blocked
                    for (seg_idx, window) in path.windows(2).enumerate() {
                        for (obs_idx, obs) in obstacles.iter().enumerate() {
                            if self.segment_intersects_rect(window[0], window[1], *obs) {
                                eprintln!(
                                    "      -> Seg {seg_idx} blocked by obs {obs_idx}: ({:.1},{:.1})->({:.1},{:.1})",
                                    window[0].x, window[0].y, window[1].x, window[1].y
                                );
                            }
                        }
                    }
                }
            }
        }

        // Find all clear paths and sort by length (shortest first)
        let mut valid: Vec<(Vec<Pos2>, f32)> = candidates
            .into_iter()
            .filter(|path| self.is_path_clear(path, obstacles))
            .map(|path| {
                let len = self.calc_path_length(&path);
                (path, len)
            })
            .collect();

        // Sort by length - shortest path wins
        valid.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // === DEBUG: Show result ===
        #[cfg(debug_assertions)]
        {
            if valid.is_empty() {
                eprintln!("Result: No valid paths, falling back to A*");
            } else {
                eprintln!("Result: Selected path with length {:.1}", valid[0].1);
            }
        }

        // Return the shortest valid path
        valid.into_iter().next().map(|(path, _)| path)
    }

    /// Check if path is clear of obstacles (obstacles should already be expanded).
    fn is_path_clear(&self, path: &[Pos2], obstacles: &[Rect]) -> bool {
        for window in path.windows(2) {
            for obs in obstacles {
                if self.segment_intersects_rect(window[0], window[1], *obs) {
                    return false; // Blocked by obstacle
                }
            }
        }
        true // Path is clear
    }

    /// Calculate total path length.
    fn calc_path_length(&self, path: &[Pos2]) -> f32 {
        path.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Grid-based A* pathfinding (fallback for complex cases).
    /// Obstacles should already be expanded by padding.
    fn grid_astar_path(&self, from: Pos2, to: Pos2, obstacles: &[Rect]) -> Vec<Pos2> {
        use std::collections::{BinaryHeap, HashMap};

        let grid_size = 15.0; // Smaller grid = more accurate paths

        // Calculate grid bounds
        let mut min_x = from.x.min(to.x) - 100.0;
        let mut max_x = from.x.max(to.x) + 100.0;
        let mut min_y = from.y.min(to.y) - 100.0;
        let mut max_y = from.y.max(to.y) + 100.0;

        for obs in obstacles {
            min_x = min_x.min(obs.left() - 50.0);
            max_x = max_x.max(obs.right() + 50.0);
            min_y = min_y.min(obs.top() - 50.0);
            max_y = max_y.max(obs.bottom() + 50.0);
        }

        // Convert to grid coordinates
        let to_grid = |p: Pos2| -> (i32, i32) {
            (
                ((p.x - min_x) / grid_size) as i32,
                ((p.y - min_y) / grid_size) as i32,
            )
        };

        let to_world = |gx: i32, gy: i32| -> Pos2 {
            Pos2::new(
                min_x + (gx as f32 + 0.5) * grid_size,
                min_y + (gy as f32 + 0.5) * grid_size,
            )
        };

        // Check if grid cell is blocked (obstacles already expanded)
        let is_blocked = |gx: i32, gy: i32| -> bool {
            let world_pos = to_world(gx, gy);
            obstacles.iter().any(|obs| obs.contains(world_pos))
        };

        let start = to_grid(from);
        let goal = to_grid(to);

        // A* algorithm with orthogonal movement only
        #[derive(Copy, Clone, Eq, PartialEq)]
        struct State {
            cost: i32,
            pos: (i32, i32),
        }

        impl Ord for State {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                other.cost.cmp(&self.cost)
            }
        }

        impl PartialOrd for State {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        let heuristic =
            |pos: (i32, i32)| -> i32 { (pos.0 - goal.0).abs() + (pos.1 - goal.1).abs() };

        let mut open = BinaryHeap::new();
        let mut came_from: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
        let mut g_score: HashMap<(i32, i32), i32> = HashMap::new();

        g_score.insert(start, 0);
        open.push(State {
            cost: heuristic(start),
            pos: start,
        });

        // Orthogonal neighbors only
        let neighbors = [(0, -1), (0, 1), (-1, 0), (1, 0)];

        while let Some(State { pos, .. }) = open.pop() {
            if pos == goal {
                // Reconstruct path
                let mut path = vec![to];
                let mut current = goal;
                while let Some(&prev) = came_from.get(&current) {
                    path.push(to_world(current.0, current.1));
                    current = prev;
                }
                path.push(from);
                path.reverse();
                return path;
            }

            let current_g = *g_score.get(&pos).unwrap_or(&i32::MAX);

            for (dx, dy) in neighbors {
                let next = (pos.0 + dx, pos.1 + dy);

                if is_blocked(next.0, next.1) {
                    continue;
                }

                let tentative_g = current_g + 1;

                if tentative_g < *g_score.get(&next).unwrap_or(&i32::MAX) {
                    came_from.insert(next, pos);
                    g_score.insert(next, tentative_g);
                    open.push(State {
                        cost: tentative_g + heuristic(next),
                        pos: next,
                    });
                }
            }
        }

        // No path found - fallback to simple route around obstacles
        let route_y = if from.y < to.y {
            min_y - 50.0
        } else {
            max_y + 50.0
        };

        vec![
            from,
            Pos2::new(from.x, route_y),
            Pos2::new(to.x, route_y),
            to,
        ]
    }

    /// Simplify path by removing unnecessary points.
    /// Uses visibility check - if we can see a point further ahead, skip intermediate points.
    fn simplify_path(&self, path: &[Pos2]) -> Vec<Pos2> {
        if path.len() <= 2 {
            return path.to_vec();
        }

        // First pass: remove collinear points
        let mut pass1 = vec![path[0]];

        for i in 1..path.len() - 1 {
            let prev = *pass1.last().unwrap();
            let curr = path[i];
            let next = path[i + 1];

            let all_same_x = (prev.x - curr.x).abs() < 1.0 && (curr.x - next.x).abs() < 1.0;
            let all_same_y = (prev.y - curr.y).abs() < 1.0 && (curr.y - next.y).abs() < 1.0;

            if !all_same_x && !all_same_y {
                pass1.push(curr);
            }
        }
        pass1.push(*path.last().unwrap());

        // Second pass: visibility-based simplification
        // If we can draw straight line from point A to point C (skipping B), do it
        if pass1.len() <= 2 {
            return pass1;
        }

        let mut result = vec![pass1[0]];
        let mut i = 1;

        while i < pass1.len() - 1 {
            let start = *result.last().unwrap();

            // Look ahead as far as possible
            let mut furthest_visible = i;
            for j in (i + 1)..pass1.len() {
                if self.is_segment_clear_no_obstacles(start, pass1[j]) {
                    furthest_visible = j;
                } else {
                    break;
                }
            }

            // Add the furthest visible point
            if furthest_visible > i {
                result.push(pass1[furthest_visible]);
                i = furthest_visible;
            } else {
                result.push(pass1[i]);
                i += 1;
            }
        }

        // Always add the last point
        if result.last() != Some(&pass1[pass1.len() - 1]) {
            result.push(*pass1.last().unwrap());
        }

        result
    }

    /// Check if segment is clear (no obstacle check, just for path simplification).
    fn is_segment_clear_no_obstacles(&self, from: Pos2, to: Pos2) -> bool {
        // For orthogonal paths, we can safely skip intermediate points
        // if they maintain horizontal or vertical alignment
        let is_horizontal = (from.y - to.y).abs() < 1.0;
        let is_vertical = (from.x - to.x).abs() < 1.0;

        // Only simplify purely orthogonal segments
        is_horizontal || is_vertical
    }

    /// Check if a line segment intersects a rectangle.
    fn segment_intersects_rect(&self, p1: Pos2, p2: Pos2, rect: Rect) -> bool {
        // Check if either endpoint is inside
        if rect.contains(p1) || rect.contains(p2) {
            return true;
        }

        // For orthogonal lines, use simple range overlap check
        let is_horizontal = (p1.y - p2.y).abs() < 0.1;
        let is_vertical = (p1.x - p2.x).abs() < 0.1;

        if is_horizontal {
            let y = p1.y;
            let min_x = p1.x.min(p2.x);
            let max_x = p1.x.max(p2.x);

            if y >= rect.top() && y <= rect.bottom() {
                if max_x >= rect.left() && min_x <= rect.right() {
                    return true;
                }
            }
            return false;
        }

        if is_vertical {
            let x = p1.x;
            let min_y = p1.y.min(p2.y);
            let max_y = p1.y.max(p2.y);

            if x >= rect.left() && x <= rect.right() {
                if max_y >= rect.top() && min_y <= rect.bottom() {
                    return true;
                }
            }
            return false;
        }

        // For diagonal lines, check intersection with rectangle edges
        let corners = [
            (rect.left_top(), rect.right_top()),
            (rect.right_top(), rect.right_bottom()),
            (rect.right_bottom(), rect.left_bottom()),
            (rect.left_bottom(), rect.left_top()),
        ];

        for (a, b) in corners {
            if self.segments_intersect(p1, p2, a, b) {
                return true;
            }
        }

        false
    }

    /// Check if two line segments intersect.
    fn segments_intersect(&self, p1: Pos2, p2: Pos2, p3: Pos2, p4: Pos2) -> bool {
        let d1 = self.cross_product(p3, p4, p1);
        let d2 = self.cross_product(p3, p4, p2);
        let d3 = self.cross_product(p1, p2, p3);
        let d4 = self.cross_product(p1, p2, p4);

        ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
            && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    }

    fn cross_product(&self, a: Pos2, b: Pos2, c: Pos2) -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
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
