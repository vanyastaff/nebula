//! Workflow graph on a canvas. Nodes are square cards with the name and action key underneath. A node
//! sits where the user last dropped it, or else in a layered layout that follows the connections. Dragging
//! a card moves it and records the placement as an undoable edit. Dragging an output port onto an input
//! port connects two nodes. Zoom scales everything, and drawing never changes the graph by itself.
use crate::{
    theme, widgets,
    workbench::{NodeDrag, Workbench},
};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;

/// Side of the square card that holds the node badge, in canvas units (before zoom).
const CARD: f32 = 84.0;
/// Room under the card for the name and the action key.
const LABEL_HEIGHT: f32 = 44.0;
const BADGE: f32 = 40.0;
const COLUMN_GAP: f32 = 120.0;
const ROW_GAP: f32 = 24.0;
const PADDING: f32 = 40.0;
const PORT_RADIUS: f32 = 5.0;
const PORT_HIT: f32 = 22.0;
const GRID: f32 = 24.0;
/// What assistive technology calls the graph area. Clicking it clears the selection.
pub(crate) const CANVAS_NAME: &str = "Workflow canvas";
/// Farthest a stored position may place a card, in canvas units; a larger one is brought back.
const MAX_COORDINATE: f32 = 20_000.0;
/// A card dropped closer than this to where it started, in screen points, was clicked rather than moved.
const CLICK_SLOP: f32 = 4.0;
pub(crate) const MIN_ZOOM: f32 = 0.5;
pub(crate) const MAX_ZOOM: f32 = 1.5;
const ZOOM_STEP: f32 = 0.1;

/// A node as the canvas draws it: id, display name and action key.
pub(crate) struct NodeView {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) action: String,
}

/// User intents expressed on the canvas. They are applied to the draft after drawing.
enum Gesture {
    Select {
        id: String,
        name: String,
    },
    /// A click on the empty canvas, which clears the selection as in other node editors.
    Deselect,
    Connect {
        from: String,
        to: String,
    },
    Place {
        id: String,
        at: Pos2,
    },
    AddAfter {
        id: String,
    },
}

/// Top-left corners of the cards, in canvas units. Columns follow the longest path from any source. The
/// pass count is bounded, so a cycle cannot push columns forever.
pub(crate) fn layout(ids: &[String], edges: &[(String, String)]) -> Vec<(String, Pos2)> {
    let mut column: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0)).collect();
    for _ in 0..ids.len() {
        for (from, to) in edges {
            let next = column.get(from.as_str()).copied().unwrap_or(0) + 1;
            if let Some(target) = column.get_mut(to.as_str())
                && next > *target
                && next < ids.len()
            {
                *target = next;
            }
        }
    }
    let mut rows: HashMap<usize, usize> = HashMap::new();
    ids.iter()
        .map(|id| {
            let col = column.get(id.as_str()).copied().unwrap_or(0);
            let row = rows.entry(col).or_insert(0);
            let position = Pos2::new(
                (col as f32).mul_add(CARD + COLUMN_GAP, PADDING),
                (*row as f32).mul_add(CARD + LABEL_HEIGHT + ROW_GAP, PADDING),
            );
            *row += 1;
            (id.clone(), position)
        })
        .collect()
}

/// Moves laid-out cards down a row at a time until they overlap no card already standing, so a card
/// placed by hand never hides one the layout placed. Each settled card stands for the next.
fn clear_of(standing: &[Pos2], laid_out: Vec<(String, Pos2)>) -> Vec<(String, Pos2)> {
    let footprint = |at: Pos2| Rect::from_min_size(at, Vec2::new(CARD, CARD + LABEL_HEIGHT));
    let mut taken: Vec<Rect> = standing.iter().map(|at| footprint(*at)).collect();
    laid_out
        .into_iter()
        .map(|(id, mut at)| {
            // One row per card that could stand in the way is always enough.
            for _ in 0..=taken.len() {
                if !taken.iter().any(|rect| rect.intersects(footprint(at))) {
                    break;
                }
                at.y += CARD + LABEL_HEIGHT + ROW_GAP;
            }
            taken.push(footprint(at));
            (id, at)
        })
        .collect()
}

/// The canvas size in canvas units that holds every card and its label.
fn extent(positions: &[Pos2]) -> Vec2 {
    let far = positions.iter().fold(Pos2::ZERO, |far, at| {
        Pos2::new(far.x.max(at.x), far.y.max(at.y))
    });
    Vec2::new(far.x + CARD, far.y + CARD + LABEL_HEIGHT) + Vec2::splat(PADDING)
}

/// The zoom at which the whole graph fits the given width, never above 100%.
fn fit_zoom(graph_width: f32, available_width: f32) -> f32 {
    (available_width / graph_width).clamp(MIN_ZOOM, 1.0)
}

fn out_port(card: Rect) -> Pos2 {
    Pos2::new(card.right(), card.center().y)
}

fn in_port(card: Rect) -> Pos2 {
    Pos2::new(card.left(), card.center().y)
}

/// Sampled cubic curve from an output port to an input port, leaving and entering horizontally.
fn curve(from: Pos2, to: Pos2) -> Vec<Pos2> {
    let reach = ((to.x - from.x).abs() / 2.0).max(48.0);
    let (p0, p3) = (from, to);
    let p1 = Pos2::new(p0.x + reach, p0.y);
    let p2 = Pos2::new(p3.x - reach, p3.y);
    (0..=24)
        .map(|step| {
            let t = step as f32 / 24.0;
            // De Casteljau: three rounds of linear interpolation between the control points.
            let (q0, q1, q2) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
            lerp(lerp(q0, q1, t), lerp(q1, q2, t), t)
        })
        .collect()
}

fn lerp(from: Pos2, to: Pos2, t: f32) -> Pos2 {
    Pos2::new(
        (to.x - from.x).mul_add(t, from.x),
        (to.y - from.y).mul_add(t, from.y),
    )
}

/// A stored canvas position as the canvas can hold it. A position that is not a finite number in
/// `f32` has no place, so the layout places the node; a far one is brought within reach.
fn placement(x: f64, y: f64) -> Option<Pos2> {
    let (x, y) = (x as f32, y as f32);
    (x.is_finite() && y.is_finite())
        .then(|| Pos2::new(x.clamp(0.0, MAX_COORDINATE), y.clamp(0.0, MAX_COORDINATE)))
}

/// Faint dot grid behind the graph, so an empty canvas still reads as a workspace. Only the dots in
/// the visible part are painted, however large the graph.
fn paint_grid(painter: &egui::Painter, canvas: Rect, zoom: f32) {
    for at in grid_dots(canvas, painter.clip_rect(), zoom) {
        painter.circle_filled(at, 1.0, theme::BORDER);
    }
}

/// The grid's dots inside `clip`, counted from the canvas origin so they keep their place while
/// scrolling.
fn grid_dots(canvas: Rect, clip: Rect, zoom: f32) -> Vec<Pos2> {
    let spacing = GRID * zoom;
    let visible = canvas.intersect(clip);
    if !visible.is_positive() {
        return Vec::new();
    }
    let first = |from: f32, origin: f32| ((from - origin) / spacing).ceil().max(1.0);
    let (first_column, first_row) = (
        first(visible.left(), canvas.left()),
        first(visible.top(), canvas.top()),
    );
    let columns = (visible.width() / spacing).ceil() as usize + 1;
    let rows = (visible.height() / spacing).ceil() as usize + 1;
    (0..columns)
        .flat_map(|column| (0..rows).map(move |row| (column, row)))
        .map(|(column, row)| {
            canvas.min
                + Vec2::new(
                    (first_column + column as f32) * spacing,
                    (first_row + row as f32) * spacing,
                )
        })
        .filter(|at| visible.contains(*at))
        .collect()
}

/// Where the canvas landed on screen, so the editor can float its controls over it.
pub(crate) struct CanvasFrame {
    /// The visible part of the canvas.
    pub(crate) viewport: Rect,
    /// Width of the whole graph in canvas units, for fitting it to the viewport.
    pub(crate) graph_width: f32,
}

/// Draws the graph in a scrollable canvas `height` tall that fills the available width.
pub(crate) fn show(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    height: f32,
) -> Option<CanvasFrame> {
    let draft = workbench.session.draft()?;
    let nodes: Vec<NodeView> = draft.definition["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .map(|node| NodeView {
                    id: node["id"].as_str().unwrap_or_default().to_owned(),
                    name: node["name"]
                        .as_str()
                        .or_else(|| node["id"].as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    action: node["action_key"].as_str().unwrap_or_default().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let edges: Vec<(String, String)> = draft
        .connections()
        .iter()
        .map(|connection| {
            (
                connection["from_node"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                connection["to_node"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect();
    let ids: Vec<String> = nodes.iter().map(|node| node.id.clone()).collect();
    // A placement the user made wins over the layout; the layout only places the rest, clear of them.
    let placed: Vec<(String, Pos2)> = ids
        .iter()
        .filter_map(|id| {
            let (x, y) = draft.placed_position(id)?;
            Some((id.clone(), placement(x, y)?))
        })
        .collect();
    let laid_out: Vec<(String, Pos2)> = layout(&ids, &edges)
        .into_iter()
        .filter(|(id, _)| !placed.iter().any(|(placed_id, _)| placed_id == id))
        .collect();
    let standing: Vec<Pos2> = placed.iter().map(|(_, at)| *at).collect();
    let base: HashMap<String, Pos2> = placed
        .into_iter()
        .chain(clear_of(&standing, laid_out))
        .collect();
    let base_positions: Vec<Pos2> = base.values().copied().collect();
    let graph = extent(&base_positions);

    let zoom = workbench.zoom;
    // The canvas covers at least the visible area, so the grid and background never stop short. One
    // point less than the viewport keeps a canvas that just fits from growing a scroll bar.
    let visible = Vec2::new(ui.available_width(), height) - Vec2::splat(1.0);

    let drag = workbench.node_drag.clone();
    let mut gestures = Vec::new();
    let output = egui::ScrollArea::both()
        .id_salt("workflow-canvas")
        .auto_shrink([false, false])
        .max_height(height)
        .show(ui, |ui| {
            let (canvas, background) =
                ui.allocate_exact_size((graph * zoom).max(visible), Sense::click());
            background.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, CANVAS_NAME)
            });
            if background.clicked() {
                gestures.push(Gesture::Deselect);
            }
            let painter = ui.painter_at(canvas);
            painter.rect_filled(canvas, theme::RADIUS_MD, theme::SIDEBAR);
            paint_grid(&painter, canvas, zoom);
            if nodes.is_empty() {
                painter.text(
                    canvas.center(),
                    Align2::CENTER_CENTER,
                    "This workflow has no nodes yet. Use Add node to place the first one.",
                    FontId::proportional(theme::SIZE_LABEL),
                    theme::TEXT_MUTED,
                );
                return;
            }

            // The card each node occupies on screen, including the drag in progress.
            let cards: HashMap<&str, Rect> = nodes
                .iter()
                .map(|node| {
                    let mut at = canvas.min + base[&node.id].to_vec2() * zoom;
                    if let Some(drag) = drag.as_ref().filter(|drag| drag.node == node.id) {
                        at += Vec2::from(drag.offset);
                    }
                    (
                        node.id.as_str(),
                        Rect::from_min_size(at, Vec2::splat(CARD * zoom)),
                    )
                })
                .collect();
            for (from, to) in &edges {
                if let (Some(source), Some(target)) =
                    (cards.get(from.as_str()), cards.get(to.as_str()))
                {
                    painter.add(egui::Shape::line(
                        curve(out_port(*source), in_port(*target)),
                        Stroke::new(1.6 * zoom, theme::EDGE),
                    ));
                }
            }
            if let (Some(from), Some(pointer)) = (
                workbench.link_from.as_ref(),
                ui.ctx().input(|input| input.pointer.latest_pos()),
            ) && let Some(source) = cards.get(from.as_str())
            {
                painter.add(egui::Shape::line(
                    curve(out_port(*source), pointer),
                    Stroke::new(1.6 * zoom, theme::ACCENT),
                ));
            }

            for node in &nodes {
                let Some(&card) = cards.get(node.id.as_str()) else {
                    continue;
                };
                paint_node(
                    &painter,
                    card,
                    node,
                    zoom,
                    workbench.selected_node.as_deref() == Some(node.id.as_str()),
                );
                let body = ui.interact(
                    card,
                    egui::Id::new(("workflow-node", node.id.as_str())),
                    Sense::click_and_drag(),
                );
                if body.clicked() {
                    gestures.push(Gesture::Select {
                        id: node.id.clone(),
                        name: node.name.clone(),
                    });
                }
                if body.dragged() {
                    let previous = drag
                        .as_ref()
                        .filter(|drag| drag.node == node.id)
                        .map_or(Vec2::ZERO, |drag| Vec2::from(drag.offset));
                    let offset = previous + body.drag_delta();
                    workbench.node_drag = Some(NodeDrag {
                        node: node.id.clone(),
                        offset: [offset.x, offset.y],
                    });
                }
                if body.drag_stopped()
                    && let Some(drag) = workbench.node_drag.take()
                {
                    let offset = Vec2::from(drag.offset);
                    if offset.length() < CLICK_SLOP {
                        // Pointer jitter during a click is a selection, not a move worth an undo step.
                        gestures.push(Gesture::Select {
                            id: node.id.clone(),
                            name: node.name.clone(),
                        });
                    } else {
                        // The card moves by the screen offset divided by zoom, in canvas units, and never
                        // above or left of the canvas origin.
                        let at = (base[&node.id] + offset / zoom).max(Pos2::ZERO);
                        gestures.push(Gesture::Place {
                            id: node.id.clone(),
                            at,
                        });
                    }
                }
                // The "+" after the output port adds a node that connects to this one.
                let plus_center = out_port(card) + Vec2::new(22.0 * zoom, 0.0);
                let plus = Rect::from_center_size(plus_center, Vec2::splat(20.0 * zoom));
                painter.line_segment(
                    [out_port(card), plus_center - Vec2::new(10.0 * zoom, 0.0)],
                    Stroke::new(1.6 * zoom, theme::EDGE),
                );
                painter.rect(
                    plus,
                    theme::RADIUS_SM,
                    theme::SURFACE,
                    Stroke::new(1.0, theme::BORDER),
                    StrokeKind::Inside,
                );
                painter.text(
                    plus_center,
                    Align2::CENTER_CENTER,
                    "+",
                    FontId::proportional(theme::SIZE_LABEL * zoom),
                    theme::TEXT,
                );
                let add = ui.interact(
                    plus,
                    egui::Id::new(("workflow-add-after", node.id.as_str())),
                    Sense::click(),
                );
                if add.clicked() {
                    gestures.push(Gesture::AddAfter {
                        id: node.id.clone(),
                    });
                }
                let out = ui.interact(
                    Rect::from_center_size(out_port(card), Vec2::splat(PORT_HIT * zoom)),
                    egui::Id::new(("workflow-out", node.id.as_str())),
                    Sense::drag(),
                );
                if out.drag_started() {
                    workbench.link_from = Some(node.id.clone());
                }
                if out.drag_stopped()
                    && let Some(from) = workbench.link_from.take()
                    && let Some(pointer) = ui.ctx().input(|input| input.pointer.latest_pos())
                {
                    let target = nodes.iter().find(|candidate| {
                        cards
                            .get(candidate.id.as_str())
                            .is_some_and(|candidate_card| {
                                Rect::from_center_size(
                                    in_port(*candidate_card),
                                    Vec2::splat(PORT_HIT * zoom),
                                )
                                .contains(pointer)
                            })
                    });
                    if let Some(target) = target {
                        gestures.push(Gesture::Connect {
                            from,
                            to: target.id.clone(),
                        });
                    }
                }
            }
        });

    for gesture in gestures {
        match gesture {
            Gesture::Select { id, name } => {
                workbench.selected_node = Some(id);
                workbench.rename = name;
                workbench.parameter.close();
                // The side panel shows the node now, not the palette.
                workbench.add_node.close();
            },
            Gesture::Deselect => {
                workbench.selected_node = None;
                workbench.parameter.close();
            },
            Gesture::Connect { from, to } => connect(workbench, &from, &to),
            Gesture::Place { id, at } => place(workbench, &id, at),
            Gesture::AddAfter { id } => {
                workbench.selected_node = None;
                workbench.add_node.open_after(Some(id));
            },
        }
    }
    Some(CanvasFrame {
        viewport: output.inner_rect,
        graph_width: graph.x,
    })
}

/// Zoom buttons. Fit scales the graph to the visible width, never above 100%.
pub(crate) fn zoom_controls(ui: &mut egui::Ui, workbench: &mut Workbench, frame: &CanvasFrame) {
    ui.horizontal(|ui| {
        if ui.button("−").on_hover_text("Zoom out").clicked() {
            workbench.zoom = (workbench.zoom - ZOOM_STEP).max(MIN_ZOOM);
        }
        ui.label(format!("{:.0}%", workbench.zoom * 100.0));
        if ui.button("+").on_hover_text("Zoom in").clicked() {
            workbench.zoom = (workbench.zoom + ZOOM_STEP).min(MAX_ZOOM);
        }
        if ui
            .button("Fit")
            .on_hover_text("Fit the graph to the canvas width")
            .clicked()
        {
            workbench.zoom = fit_zoom(frame.graph_width, frame.viewport.width());
        }
    });
}

/// One node: a square card with a coloured badge, the name and the action key beneath it, and ports on
/// its left and right edges. Everything scales with zoom.
fn paint_node(painter: &egui::Painter, card: Rect, node: &NodeView, zoom: f32, selected: bool) {
    let (width, outline) = if selected {
        (2.0, theme::ACCENT)
    } else {
        (1.0, theme::BORDER)
    };
    painter.rect(
        card,
        theme::RADIUS_MD,
        theme::SURFACE,
        Stroke::new(width * zoom, outline),
        StrokeKind::Inside,
    );
    let badge = Rect::from_center_size(card.center(), Vec2::splat(BADGE * zoom));
    painter.rect_filled(badge, theme::RADIUS_SM, theme::node_accent(&node.action));
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        widgets::initial(&node.name),
        FontId::proportional(theme::SIZE_TITLE * zoom),
        theme::ON_ACCENT,
    );
    let below = 8.0f32.mul_add(zoom, card.bottom());
    painter.text(
        Pos2::new(card.center().x, below),
        Align2::CENTER_TOP,
        &node.name,
        FontId::proportional(theme::SIZE_BODY * zoom),
        theme::TEXT,
    );
    painter.text(
        Pos2::new(card.center().x, 18.0f32.mul_add(zoom, below)),
        Align2::CENTER_TOP,
        &node.action,
        FontId::proportional(theme::SIZE_OVERLINE * zoom),
        theme::TEXT_MUTED,
    );
    painter.circle_filled(in_port(card), PORT_RADIUS * zoom, theme::TEXT_MUTED);
    painter.circle_filled(out_port(card), PORT_RADIUS * zoom, theme::TEXT_MUTED);
}

fn place(workbench: &mut Workbench, node: &str, at: Pos2) {
    workbench.node_drag = None;
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    // Placement is a silent layout change, so it reports only a failure.
    if let Err(error) = draft.move_node(node, f64::from(at.x), f64::from(at.y)) {
        workbench.feedback.error(error.to_string());
    }
}

fn connect(workbench: &mut Workbench, from: &str, to: &str) {
    if from == to {
        workbench.feedback.error("A node cannot connect to itself.");
        return;
    }
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let message = format!(
        "Connected {} to {}.",
        draft.node_name(from),
        draft.node_name(to)
    );
    let result = draft.connect(from, to);
    workbench.feedback.report(result, &message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn edge(from: &str, to: &str) -> (String, String) {
        (from.to_owned(), to.to_owned())
    }

    #[test]
    fn layout_places_connected_nodes_in_later_columns() {
        let positions: HashMap<String, Pos2> =
            layout(&ids(&["a", "b", "c"]), &[edge("a", "b"), edge("b", "c")])
                .into_iter()
                .collect();
        assert!(positions["a"].x < positions["b"].x);
        assert!(positions["b"].x < positions["c"].x);
    }

    #[test]
    fn unconnected_nodes_share_the_first_column_on_separate_rows() {
        let positions: HashMap<String, Pos2> = layout(&ids(&["x", "y"]), &[]).into_iter().collect();
        assert_eq!(positions["x"].x, positions["y"].x);
        assert!(positions["x"].y < positions["y"].y);
    }

    #[test]
    fn a_laid_out_card_moves_clear_of_a_placed_one() {
        let footprint = |at: Pos2| Rect::from_min_size(at, Vec2::new(CARD, CARD + LABEL_HEIGHT));
        let placed = Pos2::new(PADDING + 30.0, PADDING + 10.0);
        let settled = clear_of(
            &[placed],
            vec![
                ("a".into(), Pos2::new(PADDING, PADDING)),
                ("b".into(), Pos2::new(PADDING, PADDING)),
                ("far".into(), Pos2::new(900.0, 900.0)),
            ],
        );
        let at: HashMap<String, Pos2> = settled.into_iter().collect();
        assert!(!footprint(at["a"]).intersects(footprint(placed)));
        assert!(!footprint(at["b"]).intersects(footprint(at["a"])));
        assert!(!footprint(at["b"]).intersects(footprint(placed)));
        // A card with room where the layout put it stays there.
        assert_eq!(at["far"], Pos2::new(900.0, 900.0));
    }

    #[test]
    fn a_cycle_does_not_push_columns_forever() {
        let positions: HashMap<String, Pos2> =
            layout(&ids(&["a", "b"]), &[edge("a", "b"), edge("b", "a")])
                .into_iter()
                .collect();
        assert!(positions["a"].x.is_finite() && positions["b"].x.is_finite());
        assert!(positions["a"].x <= 2.0f32.mul_add(CARD + COLUMN_GAP, PADDING));
    }

    #[test]
    fn an_unreachable_stored_position_is_left_to_the_layout_or_brought_within_reach() {
        // 1e308 is a valid JSON number but no `f32`; the layout places that node instead.
        assert_eq!(placement(1e308, 10.0), None);
        assert_eq!(placement(f64::NAN, 10.0), None);
        assert_eq!(placement(1e9, -50.0), Some(Pos2::new(MAX_COORDINATE, 0.0)));
        assert_eq!(placement(120.0, 80.0), Some(Pos2::new(120.0, 80.0)));
    }

    #[test]
    fn the_grid_paints_only_what_is_visible() {
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 120.0));
        // A canvas as large as the far bound allows still gets one viewport of dots.
        let canvas = Rect::from_min_size(Pos2::new(-5000.0, -5000.0), Vec2::splat(MAX_COORDINATE));
        let dots = grid_dots(canvas, clip, MIN_ZOOM);
        let spacing = GRID * MIN_ZOOM;
        let most = ((240.0 / spacing + 2.0) * (120.0 / spacing + 2.0)) as usize;
        assert!(
            !dots.is_empty() && dots.len() <= most,
            "{} dots",
            dots.len()
        );
        assert!(dots.iter().all(|at| clip.contains(*at)));
        // Dots sit on the canvas's grid, not the viewport's.
        assert!(dots.iter().all(|at| {
            let step = (at.x - canvas.left()) / spacing;
            (step - step.round()).abs() < 1e-3
        }));
        assert!(grid_dots(canvas, Rect::NOTHING, MIN_ZOOM).is_empty());
    }

    #[test]
    fn fit_never_zooms_past_full_size_or_below_the_minimum() {
        assert_eq!(fit_zoom(400.0, 2000.0), 1.0);
        assert_eq!(fit_zoom(4000.0, 400.0), MIN_ZOOM);
        assert!((fit_zoom(800.0, 400.0) - 0.5).abs() < f32::EPSILON);
    }
}
