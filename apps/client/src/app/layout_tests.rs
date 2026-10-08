//! The whole app at a phone's width: the real shell, effects and demo workspace, drawn headless with
//! AccessKit on, and checked through the tree it hands to the platform.

use super::ClientApp;
use crate::{views::Intent, workbench::Page};
use eframe::egui::{
    self,
    accesskit::{Action, Node, Role},
};

/// A phone held upright, in points.
const PHONE: egui::Vec2 = egui::vec2(375.0, 812.0);
/// Width a laptop window has, for comparison.
const LAPTOP: egui::Vec2 = egui::vec2(1280.0, 800.0);

struct Harness {
    context: egui::Context,
    app: ClientApp,
    frame: eframe::Frame,
    size: egui::Vec2,
}

impl Harness {
    fn new(size: egui::Vec2) -> Self {
        let context = egui::Context::default();
        context.enable_accesskit();
        let creation = eframe::CreationContext::_new_kittest(context.clone());
        let app = ClientApp::new(&creation).unwrap();
        Self {
            context,
            app,
            frame: eframe::Frame::_new_kittest(),
            size,
        }
    }

    /// One frame; returns every node of the accessibility tree it produced.
    fn step(&mut self) -> Vec<Node> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            ..Default::default()
        };
        let (app, frame) = (&mut self.app, &mut self.frame);
        let mut output = self
            .context
            .run_ui(input, |ui| eframe::App::ui(app, ui, frame));
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .map(|update| update.nodes.into_iter().map(|(_, node)| node).collect())
            .unwrap_or_default()
    }

    /// Steps until `done` holds, as the effects answer off the render thread.
    fn until(&mut self, what: &str, mut done: impl FnMut(&ClientApp) -> bool) -> Vec<Node> {
        for _ in 0..400 {
            let nodes = self.step();
            if done(&self.app) {
                // One more frame lays out what the answer changed.
                self.step();
                return self.step();
            }
            drop(nodes);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("{what} never happened");
    }

    /// Signs in to the demo and waits for its workflow list.
    fn demo(size: egui::Vec2) -> Self {
        let mut harness = Self::new(size);
        harness.step();
        let context = harness.context.clone();
        harness.app.run_intent(&context, Intent::OpenDemo);
        harness.until("the demo's workflow list", |app| {
            !app.workbench.navigator.workflows.is_empty()
        });
        harness
    }
}

fn name(node: &Node) -> &str {
    node.label().or_else(|| node.value()).unwrap_or_default()
}

/// Nodes that reach past the right edge, which would make the page scroll sideways.
fn overflowing(nodes: &[Node], width: f32) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| {
            let bounds = node.bounds()?;
            (bounds.x1 > f64::from(width) + 0.5 && bounds.x0 < f64::from(width)).then(|| {
                format!(
                    "{:?} {:?} from {:.0} to {:.0}, {} children",
                    node.role(),
                    name(node),
                    bounds.x0,
                    bounds.x1,
                    node.children().len()
                )
            })
        })
        .collect()
}

/// Elements the keyboard can reach that have no name: a screen reader moving through them would
/// announce only their role.
fn unnamed_focusable(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.supports_action(Action::Focus))
        .filter(|node| node.label().is_none_or(|label| label.trim().is_empty()))
        .map(|node| format!("{:?} at {:?}", node.role(), node.bounds()))
        .collect()
}

/// Bounds of the page entries of the navigation, by title.
fn navigation(nodes: &[Node]) -> Vec<(String, f64, f64)> {
    Page::NAVIGATION
        .iter()
        .filter_map(|page| {
            let node = nodes
                .iter()
                .find(|node| node.role() == Role::Button && name(node) == page.title())?;
            let bounds = node.bounds()?;
            Some((page.title().to_owned(), bounds.x0, bounds.y0))
        })
        .collect()
}

#[test]
fn a_laptop_window_lists_the_pages_in_a_rail() {
    let mut harness = Harness::demo(LAPTOP);
    let nodes = harness.step();
    let entries = navigation(&nodes);
    assert_eq!(entries.len(), Page::NAVIGATION.len(), "{entries:?}");
    // A rail: one column, one entry under the other.
    let left = entries[0].1;
    assert!(
        entries.iter().all(|(_, x, _)| (x - left).abs() < 1.0),
        "{entries:?}"
    );
    assert!(
        entries.windows(2).all(|pair| pair[1].2 > pair[0].2),
        "{entries:?}"
    );
}

#[test]
fn everything_the_keyboard_reaches_has_a_name() {
    let mut harness = Harness::demo(LAPTOP);
    for page in Page::NAVIGATION {
        harness.app.workbench.go(page);
        let nodes = harness.until(page.title(), |app| !app.workbench.session.busy());
        let unnamed = unnamed_focusable(&nodes);
        assert!(unnamed.is_empty(), "{}: {unnamed:?}", page.title());
    }

    // The editor with a node open in its panel: the canvas, its nodes and the node's form.
    let workflow = harness.app.workbench.navigator.workflows[0].id.clone();
    assert!(harness.app.workbench.select_workflow(&workflow));
    let context = harness.context.clone();
    harness
        .app
        .run_intent(&context, Intent::LoadWorkflow(workflow));
    harness.until("the workflow to open", |app| {
        app.workbench.session.draft().is_some() && !app.workbench.session.busy()
    });
    let node = harness.app.workbench.session.draft().unwrap().definition["nodes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    harness.app.workbench.selected_node = Some(node);
    let nodes = harness.until("the node's form", |app| {
        app.workbench
            .schemas
            .values()
            .any(|schema| matches!(schema, crate::workbench::SchemaState::Ready(_)))
            && !app.workbench.session.busy()
    });
    let unnamed = unnamed_focusable(&nodes);
    assert!(unnamed.is_empty(), "editor: {unnamed:?}");
    let names: Vec<&str> = nodes.iter().map(name).collect();
    assert!(
        names.iter().any(|name| name.starts_with("Node ")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("Add a node after ")),
        "{names:?}"
    );
}

#[test]
fn a_phone_gets_tabs_and_no_page_scrolls_sideways() {
    let mut harness = Harness::demo(PHONE);
    let nodes = harness.step();
    let entries = navigation(&nodes);
    assert_eq!(entries.len(), Page::NAVIGATION.len(), "{entries:?}");
    // Tabs: entries side by side, wrapped into a few rows under the top bar, not a column.
    let rows: std::collections::BTreeSet<i64> =
        entries.iter().map(|(_, _, y)| y.round() as i64).collect();
    assert!(rows.len() < entries.len() / 2 + 1, "{entries:?}");

    for page in Page::NAVIGATION {
        harness.app.workbench.go(page);
        // Pages read their data when shown; wait for it so the data, not a spinner, is checked.
        let nodes = harness.until(page.title(), |app| !app.workbench.session.busy());
        let nodes = if nodes.len() < 20 {
            harness.step()
        } else {
            nodes
        };
        let wide = overflowing(&nodes, PHONE.x);
        let mut widest: Vec<(f64, f64, String)> = nodes
            .iter()
            .filter_map(|node| {
                let bounds = node.bounds()?;
                Some((
                    bounds.x1,
                    bounds.width(),
                    format!("{:?} {:?}", node.role(), name(node)),
                ))
            })
            .collect();
        widest.sort_by(|a, b| b.1.total_cmp(&a.1));
        widest.truncate(12);
        assert!(wide.is_empty(), "{}: {wide:?}\n{widest:?}", page.title());
    }
}

#[test]
fn the_editor_stacks_its_panels_on_a_phone() {
    let mut harness = Harness::demo(PHONE);
    let workflow = harness.app.workbench.navigator.workflows[0].id.clone();
    assert!(harness.app.workbench.select_workflow(&workflow));
    let context = harness.context.clone();
    harness
        .app
        .run_intent(&context, Intent::LoadWorkflow(workflow));
    let nodes = harness.until("the workflow to open", |app| {
        app.workbench.session.draft().is_some() && !app.workbench.session.busy()
    });
    let nodes = if nodes.is_empty() {
        harness.step()
    } else {
        nodes
    };

    assert_eq!(harness.app.workbench.page, Page::Editor);
    // The graph scrolls inside its canvas, which may be wider than the phone; everything else on
    // the page fits the window.
    let canvas = nodes
        .iter()
        .find(|node| name(node) == crate::views::canvas::CANVAS_NAME)
        .and_then(Node::bounds)
        .unwrap();
    let outside: Vec<Node> = nodes
        .iter()
        .filter(|node| {
            node.bounds().is_none_or(|bounds| {
                !(bounds.x0 >= canvas.x0 - 0.5
                    && bounds.y0 >= canvas.y0 - 0.5
                    && bounds.y1 <= canvas.y1 + 0.5)
            })
        })
        .cloned()
        .collect();
    let wide = overflowing(&outside, PHONE.x);
    assert!(wide.is_empty(), "editor: {wide:?}");
    // The runs panel sits under the canvas in the same column rather than beside it.
    let runs = nodes
        .iter()
        .find(|node| name(node) == "Runs")
        .and_then(Node::bounds)
        .unwrap();
    assert!(
        runs.y0 > canvas.y1,
        "runs at {runs:?}, canvas at {canvas:?}"
    );
    let names: Vec<&str> = nodes.iter().map(name).collect();
    assert!(names.contains(&"Execute workflow"), "{names:?}");
}
