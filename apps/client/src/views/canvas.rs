//! Workflow graph on a canvas. Columns follow the longest path from a source node. Dragging an
//! output port onto an input port connects two nodes; both are local edits on the draft.
use crate::{theme, workbench::Workbench};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;

const NODE_SIZE: Vec2 = Vec2::new(184.0, 58.0);
const COLUMN_GAP: f32 = 96.0;
const ROW_GAP: f32 = 28.0;
const PADDING: f32 = 24.0;
const PORT_RADIUS: f32 = 6.0;
const PORT_HIT: f32 = 22.0;
const CANVAS_HEIGHT: f32 = 300.0;

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

/// Top-left corners of the nodes, relative to the canvas origin. Columns follow the longest path
/// from any source. The pass count is bounded, so a cycle cannot push columns forever.
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
                (col as f32).mul_add(NODE_SIZE.x + COLUMN_GAP, PADDING),
                (*row as f32).mul_add(NODE_SIZE.y + ROW_GAP, PADDING),
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
    Vec2::new(far.x + NODE_SIZE.x, far.y + NODE_SIZE.y) + Vec2::splat(PADDING)
}

fn out_port(node: Rect) -> Pos2 {
    Pos2::new(node.right(), node.center().y)
}

fn in_port(node: Rect) -> Pos2 {
    Pos2::new(node.left(), node.center().y)
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

            let rects: HashMap<&str, Rect> = positions
                .iter()
                .map(|(id, at)| {
                    (
                        id.as_str(),
                        Rect::from_min_size(canvas.min + at.to_vec2(), NODE_SIZE),
                    )
                })
                .collect();
            for (from, to) in &edges {
                if let (Some(source), Some(target)) =
                    (rects.get(from.as_str()), rects.get(to.as_str()))
                {
                    painter.add(egui::Shape::line(
                        curve(out_port(*source), in_port(*target)),
                        Stroke::new(1.8, theme::TEXT_MUTED),
                    ));
                }
            }
            if let (Some(from), Some(pointer)) = (
                workbench.link_from.as_ref(),
                ui.ctx().input(|input| input.pointer.latest_pos()),
            ) && let Some(source) = rects.get(from.as_str())
            {
                painter.add(egui::Shape::line(
                    curve(out_port(*source), pointer),
                    Stroke::new(1.8, theme::ACCENT),
                ));
            }

            for node in &nodes {
                let Some(&rect) = rects.get(node.id.as_str()) else {
                    continue;
                };
                let selected = workbench.selected_node.as_deref() == Some(node.id.as_str());
                let (width, color) = if selected {
                    (2.0, theme::ACCENT)
                } else {
                    (1.0, theme::BORDER)
                };
                painter.rect(
                    rect,
                    theme::RADIUS_MD,
                    theme::SURFACE,
                    Stroke::new(width, color),
                    StrokeKind::Inside,
                );
                painter.text(
                    rect.min + Vec2::new(14.0, 11.0),
                    Align2::LEFT_TOP,
                    &node.name,
                    FontId::proportional(15.0),
                    theme::TEXT,
                );
                painter.text(
                    rect.min + Vec2::new(14.0, 33.0),
                    Align2::LEFT_TOP,
                    &node.action,
                    FontId::proportional(12.0),
                    theme::TEXT_MUTED,
                );
                painter.circle_filled(in_port(rect), PORT_RADIUS, theme::ACCENT);
                painter.circle_filled(out_port(rect), PORT_RADIUS, theme::ACCENT);

                let body = ui.interact(
                    rect,
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
                    Rect::from_center_size(out_port(rect), Vec2::splat(PORT_HIT)),
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
                        rects
                            .get(candidate.id.as_str())
                            .is_some_and(|candidate_rect| {
                                Rect::from_center_size(
                                    in_port(*candidate_rect),
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

fn connect(workbench: &mut Workbench, from: &str, to: &str) {
    if from == to {
        workbench.feedback.error("A node cannot connect to itself.");
        return;
    }
    let source = node_name(workbench, from);
    let target = node_name(workbench, to);
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    match draft.connect(from, to) {
        Ok(()) => workbench
            .feedback
            .info(format!("Connected {source} to {target}.")),
        Err(error) => workbench.feedback.error(error.to_string()),
    }
}

/// The display name of a node for feedback, falling back to its id.
fn node_name(workbench: &Workbench, id: &str) -> String {
    workbench
        .session
        .draft()
        .and_then(|draft| draft.definition["nodes"].as_array())
        .and_then(|nodes| nodes.iter().find(|node| node["id"].as_str() == Some(id)))
        .and_then(|node| node["name"].as_str())
        .unwrap_or(id)
        .to_owned()
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
        assert!(positions["a"].x <= 2.0f32.mul_add(NODE_SIZE.x + COLUMN_GAP, PADDING));
    }
}
