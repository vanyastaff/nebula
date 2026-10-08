//! Workflow graph on a canvas. Columns follow the longest path from a source node. Each node is a square
//! card with its name and action key underneath. Dragging an output port onto an input port connects two
//! nodes; both are local edits on the draft.
use crate::{theme, workbench::Workbench};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;

/// Side of the square card that holds the node badge.
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
const CANVAS_HEIGHT: f32 = 320.0;

/// A node as the canvas draws it: id, display name and action key.
pub(crate) struct NodeView {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) action: String,
}

/// User intents expressed on the canvas. They are applied to the draft after drawing.
enum Gesture {
    Select { id: String, name: String },
    Connect { from: String, to: String },
}

/// Top-left corners of the cards, relative to the canvas origin. Columns follow the longest path from
/// any source. The pass count is bounded, so a cycle cannot push columns forever.
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

fn extent(positions: &[(String, Pos2)]) -> Vec2 {
    let far = positions.iter().fold(Pos2::ZERO, |far, (_, at)| {
        Pos2::new(far.x.max(at.x), far.y.max(at.y))
    });
    Vec2::new(far.x + CARD, far.y + CARD + LABEL_HEIGHT) + Vec2::splat(PADDING)
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

/// Faint dot grid behind the graph, so an empty canvas still reads as a workspace.
fn paint_grid(painter: &egui::Painter, canvas: Rect) {
    let columns = (canvas.width() / GRID) as usize;
    let rows = (canvas.height() / GRID) as usize;
    for column in 1..=columns {
        for row in 1..=rows {
            let at = canvas.min + Vec2::new(column as f32 * GRID, row as f32 * GRID);
            painter.circle_filled(at, 1.0, theme::BORDER);
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let Some(draft) = workbench.session.draft() else {
        return;
    };
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
    let positions = layout(&ids, &edges);
    let size = extent(&positions).max(Vec2::new(ui.available_width(), CANVAS_HEIGHT));

    let mut gestures = Vec::new();
    egui::ScrollArea::both()
        .id_salt("workflow-canvas")
        .max_height(CANVAS_HEIGHT)
        .show(ui, |ui| {
            let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
            let painter = ui.painter_at(canvas);
            painter.rect_filled(canvas, theme::RADIUS_MD, theme::SIDEBAR);
            paint_grid(&painter, canvas);
            if nodes.is_empty() {
                painter.text(
                    canvas.center(),
                    Align2::CENTER_CENTER,
                    "No nodes yet. Add one below.",
                    FontId::proportional(14.0),
                    theme::TEXT_MUTED,
                );
                return;
            }

            let cards: HashMap<&str, Rect> = positions
                .iter()
                .map(|(id, at)| {
                    (
                        id.as_str(),
                        Rect::from_min_size(canvas.min + at.to_vec2(), Vec2::splat(CARD)),
                    )
                })
                .collect();
            for (from, to) in &edges {
                if let (Some(source), Some(target)) =
                    (cards.get(from.as_str()), cards.get(to.as_str()))
                {
                    painter.add(egui::Shape::line(
                        curve(out_port(*source), in_port(*target)),
                        Stroke::new(1.6, theme::EDGE),
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
                    Stroke::new(1.6, theme::ACCENT),
                ));
            }

            for node in &nodes {
                let Some(&card) = cards.get(node.id.as_str()) else {
                    continue;
                };
                paint_node(&painter, card, node, workbench.selected_node.as_deref());
                let body = ui.interact(
                    card,
                    egui::Id::new(("workflow-node", node.id.as_str())),
                    Sense::click(),
                );
                if body.clicked() {
                    gestures.push(Gesture::Select {
                        id: node.id.clone(),
                        name: node.name.clone(),
                    });
                }
                let out = ui.interact(
                    Rect::from_center_size(out_port(card), Vec2::splat(PORT_HIT)),
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
                                    Vec2::splat(PORT_HIT),
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
            },
            Gesture::Connect { from, to } => connect(workbench, &from, &to),
        }
    }
}

/// One node: a square card with a coloured badge, the name and the action key beneath it, and ports on
/// its left and right edges.
fn paint_node(painter: &egui::Painter, card: Rect, node: &NodeView, selected_id: Option<&str>) {
    let selected = selected_id == Some(node.id.as_str());
    let (width, outline) = if selected {
        (2.0, theme::ACCENT)
    } else {
        (1.0, theme::BORDER)
    };
    painter.rect(
        card,
        theme::RADIUS_MD,
        theme::SURFACE,
        Stroke::new(width, outline),
        StrokeKind::Inside,
    );
    let badge = Rect::from_center_size(card.center(), Vec2::splat(BADGE));
    painter.rect_filled(badge, theme::RADIUS_SM, theme::node_accent(&node.action));
    let initial = node
        .name
        .chars()
        .next()
        .map(|letter| letter.to_uppercase().collect::<String>())
        .unwrap_or_default();
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        initial,
        FontId::proportional(18.0),
        theme::SURFACE,
    );
    let below = card.bottom() + 8.0;
    painter.text(
        Pos2::new(card.center().x, below),
        Align2::CENTER_TOP,
        &node.name,
        FontId::proportional(13.0),
        theme::TEXT,
    );
    painter.text(
        Pos2::new(card.center().x, below + 18.0),
        Align2::CENTER_TOP,
        &node.action,
        FontId::proportional(11.0),
        theme::TEXT_MUTED,
    );
    painter.circle_filled(in_port(card), PORT_RADIUS, theme::TEXT_MUTED);
    painter.circle_filled(out_port(card), PORT_RADIUS, theme::TEXT_MUTED);
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
    fn a_cycle_does_not_push_columns_forever() {
        let positions: HashMap<String, Pos2> =
            layout(&ids(&["a", "b"]), &[edge("a", "b"), edge("b", "a")])
                .into_iter()
                .collect();
        assert!(positions["a"].x.is_finite() && positions["b"].x.is_finite());
        assert!(positions["a"].x <= 2.0f32.mul_add(CARD + COLUMN_GAP, PADDING));
    }
}
