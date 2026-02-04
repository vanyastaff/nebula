//! Comprehensive showcase of all nebula-ui flow components.
//!
//! Run with: `cargo run -p nebula-ui --example flow_showcase`
//!
//! This demo showcases:
//! - BoardEditor with pan/zoom canvas
//! - Multiple node types with different categories
//! - Connection types: Straight, Bezier, SmoothStep, Smart
//! - Background patterns: Dots, Lines, Cross
//! - Minimap navigation
//! - Controls panel
//! - Node selection and dragging
//! - Connection creation

use eframe::egui;
use egui::Pos2;
use nebula_ui::flow::{
    Background, BackgroundVariant, BoardEditor, BoardEvent, BoardState, Connection, ControlAction,
    Controls, ControlsConfig, ControlsPosition, DataType, EdgeType, Minimap, MinimapConfig,
    MinimapPosition, Node, NodeId, Pin,
};
use nebula_ui::prelude::*;
use std::collections::HashMap;

// When compiling natively
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("Nebula UI - Flow Components Showcase"),
        ..Default::default()
    };

    eframe::run_native(
        "Flow Showcase",
        options,
        Box::new(|cc| Ok(Box::new(FlowShowcaseApp::new(cc)))),
    )
}

// When compiling for web
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(FlowShowcaseApp::new(cc)))),
            )
            .await;

        // Remove the loading text and spinner:
        let loading_text = document.get_element_by_id("loading_text");
        if let Some(loading_text) = loading_text {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(&format!("<p>Error starting app: {e:?}</p>"));
                }
            }
        }
    });
}

#[derive(Default, PartialEq, Clone, Copy)]
enum DemoTab {
    #[default]
    FullEditor,
    BackgroundPatterns,
    EdgeTypes,
    NodeStyles,
    MinimapDemo,
    ControlsDemo,
}

struct FlowShowcaseApp {
    current_tab: DemoTab,
    dark_mode: bool,

    // Main editor state
    board_state: BoardState,
    nodes: Vec<Node>,
    connections: Vec<Connection>,

    // Demo states
    selected_edge_type: EdgeType,
    selected_background: BackgroundVariant,
    show_minimap: bool,
    show_controls: bool,
    minimap_position: MinimapPosition,
    controls_position: ControlsPosition,

    // Node counter for unique IDs
    node_counter: u64,
}

impl FlowShowcaseApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Theme::dark().apply(&cc.egui_ctx);

        let nodes = create_demo_workflow();
        let connections = create_demo_connections(&nodes);

        // Initialize board state with pan offset to center the demo nodes
        let mut board_state = BoardState::new();
        board_state.canvas.pan = egui::Vec2::new(100.0, 50.0);

        Self {
            current_tab: DemoTab::default(),
            dark_mode: true,
            board_state,
            nodes,
            connections,
            selected_edge_type: EdgeType::Bezier,
            selected_background: BackgroundVariant::Dots,
            show_minimap: true,
            show_controls: true,
            minimap_position: MinimapPosition::BottomRight,
            controls_position: ControlsPosition::BottomLeft,
            node_counter: 100,
        }
    }

    fn show_full_editor(&mut self, ui: &mut egui::Ui) {
        // Toolbar
        ui.horizontal(|ui| {
            ui.label("Edge Type:");
            egui::ComboBox::from_id_salt("edge_type")
                .selected_text(format!("{:?}", self.selected_edge_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        EdgeType::Straight,
                        "Straight",
                    );
                    ui.selectable_value(&mut self.selected_edge_type, EdgeType::Bezier, "Bezier");
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        EdgeType::SmoothStep,
                        "SmoothStep",
                    );
                    ui.selectable_value(&mut self.selected_edge_type, EdgeType::Step, "Step");
                });

            ui.separator();

            ui.label("Background:");
            egui::ComboBox::from_id_salt("background")
                .selected_text(format!("{:?}", self.selected_background))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.selected_background,
                        BackgroundVariant::Dots,
                        "Dots",
                    );
                    ui.selectable_value(
                        &mut self.selected_background,
                        BackgroundVariant::Lines,
                        "Lines",
                    );
                    ui.selectable_value(
                        &mut self.selected_background,
                        BackgroundVariant::Cross,
                        "Cross",
                    );
                });

            ui.separator();

            ui.checkbox(&mut self.show_minimap, "Minimap");
            ui.checkbox(&mut self.show_controls, "Controls");

            ui.separator();

            if Button::new("Add Node").primary().small().show(ui).clicked() {
                self.add_random_node();
            }

            if Button::new("Reset View")
                .secondary()
                .small()
                .show(ui)
                .clicked()
            {
                self.board_state.reset_view();
            }

            if Button::new("Fit Content")
                .secondary()
                .small()
                .show(ui)
                .clicked()
            {
                let viewport = self.board_state.canvas.viewport;
                println!("Fit Content clicked!");
                println!("  viewport: {:?}", viewport);
                println!("  nodes count: {}", self.nodes.len());
                for node in &self.nodes {
                    println!(
                        "    node {:?} at {:?} size {:?}",
                        node.id, node.position, node.size
                    );
                }
                self.board_state.fit_to_nodes(&self.nodes, viewport);
                println!(
                    "  result: zoom={}, pan={:?}",
                    self.board_state.canvas.zoom, self.board_state.canvas.pan
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!(
                    "Nodes: {} | Connections: {} | Zoom: {:.0}%",
                    self.nodes.len(),
                    self.connections.len(),
                    self.board_state.canvas.zoom * 100.0
                ));
            });
        });

        ui.separator();

        // Update connection edge types
        for conn in &mut self.connections {
            conn.edge_type = self.selected_edge_type;
        }

        // Build pins map
        let pins_map: HashMap<NodeId, (Vec<Pin>, Vec<Pin>)> = self
            .nodes
            .iter()
            .map(|n| (n.id, (n.inputs.clone(), n.outputs.clone())))
            .collect();

        // Show board editor and collect events
        let events = {
            let mut editor = BoardEditor::new(
                &self.nodes,
                &pins_map,
                &self.connections,
                &mut self.board_state,
            );

            editor.show_in_ui(ui);
            editor.take_events()
        };

        // Now board_state is no longer borrowed, we can use it freely

        // Show minimap overlay
        if self.show_minimap {
            let minimap_config = MinimapConfig {
                position: self.minimap_position,
                width: 200.0,
                height: 150.0,
                ..Default::default()
            };

            let viewport = self.board_state.canvas.viewport;
            let pan = self.board_state.canvas.pan;
            let zoom = self.board_state.canvas.zoom;

            let minimap_response =
                Minimap::new(&self.nodes, &self.connections, viewport, pan, zoom)
                    .config(minimap_config)
                    .show(ui);

            // Handle minimap navigation
            if let Some(canvas_pos) = minimap_response.clicked_position {
                // Center view on clicked position
                self.board_state.canvas.pan = egui::Vec2::new(
                    viewport.width() / 2.0 - canvas_pos.x * zoom,
                    viewport.height() / 2.0 - canvas_pos.y * zoom,
                );
            }
        }

        // Show controls overlay
        if self.show_controls {
            let controls_config = ControlsConfig {
                position: self.controls_position,
                show_zoom: true,
                show_fit_view: true,
                show_zoom_reset: true,
                show_lock: true,
                ..Default::default()
            };

            let controls_response = Controls::new().config(controls_config).show(ui);

            for action in controls_response.actions {
                match action {
                    ControlAction::ZoomIn => {
                        self.board_state.canvas.zoom =
                            (self.board_state.canvas.zoom * 1.2).min(3.0);
                    }
                    ControlAction::ZoomOut => {
                        self.board_state.canvas.zoom =
                            (self.board_state.canvas.zoom / 1.2).max(0.1);
                    }
                    ControlAction::ZoomReset => {
                        self.board_state.canvas.zoom = 1.0;
                    }
                    ControlAction::FitView => {
                        let viewport = self.board_state.canvas.viewport;
                        self.board_state.fit_to_nodes(&self.nodes, viewport);
                    }
                    _ => {}
                }
            }
        }

        // Handle board events
        for event in events {
            match event {
                BoardEvent::NodeMoved { id, position } => {
                    if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
                        node.position = position;
                    }
                }
                BoardEvent::NodeDeleteRequested(id) => {
                    self.nodes.retain(|n| n.id != id);
                    // Remove connections to/from this node
                    self.connections
                        .retain(|c| c.source.node != id && c.target.node != id);
                }
                BoardEvent::ConnectionCreated {
                    from_pin, to_pin, ..
                } => {
                    let conn = Connection::with_edge_type(
                        from_pin,
                        to_pin,
                        DataType::Generic,
                        self.selected_edge_type,
                    );
                    self.connections.push(conn);
                }
                BoardEvent::ConnectionDeleted(id) => {
                    self.connections.retain(|c| c.id != id);
                }
                BoardEvent::CanvasDoubleClicked(pos) => {
                    // Add node at double-click position
                    self.add_node_at(pos);
                }
                _ => {}
            }
        }
    }

    fn show_background_patterns(&mut self, ui: &mut egui::Ui) {
        ui.heading("Background Patterns");
        ui.label("The flow editor supports three background patterns inspired by ReactFlow.");
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            // Dots pattern
            ui.vertical(|ui| {
                ui.label("Dots (default)");
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(300.0, 200.0), egui::Sense::hover());
                Background::new()
                    .variant(BackgroundVariant::Dots)
                    .gap(20.0)
                    .draw(ui, rect, egui::Vec2::ZERO, 1.0);
            });

            ui.add_space(16.0);

            // Lines pattern
            ui.vertical(|ui| {
                ui.label("Lines");
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(300.0, 200.0), egui::Sense::hover());
                Background::new()
                    .variant(BackgroundVariant::Lines)
                    .gap(20.0)
                    .draw(ui, rect, egui::Vec2::ZERO, 1.0);
            });

            ui.add_space(16.0);

            // Cross pattern
            ui.vertical(|ui| {
                ui.label("Cross");
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(300.0, 200.0), egui::Sense::hover());
                Background::new()
                    .variant(BackgroundVariant::Cross)
                    .gap(20.0)
                    .draw(ui, rect, egui::Vec2::ZERO, 1.0);
            });
        });

        ui.add_space(24.0);
        ui.heading("With Zoom");
        ui.label("Background patterns scale with zoom level.");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            for zoom in [0.5, 1.0, 2.0] {
                ui.vertical(|ui| {
                    ui.label(format!("Zoom: {:.0}%", zoom * 100.0));
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::new(200.0, 150.0), egui::Sense::hover());
                    Background::new().variant(BackgroundVariant::Dots).draw(
                        ui,
                        rect,
                        egui::Vec2::ZERO,
                        zoom,
                    );
                });
                ui.add_space(8.0);
            }
        });
    }

    fn show_edge_types(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edge/Connection Types");
        ui.label("Four edge types for different visual styles.");
        ui.add_space(16.0);

        let edge_types = [
            (
                EdgeType::Straight,
                "Straight",
                "Direct line from source to target",
            ),
            (
                EdgeType::Bezier,
                "Bezier",
                "Smooth cubic bezier curve (default)",
            ),
            (
                EdgeType::SmoothStep,
                "SmoothStep",
                "Orthogonal path with rounded corners",
            ),
            (
                EdgeType::Step,
                "Step",
                "Orthogonal path with 90-degree corners",
            ),
        ];

        for (edge_type, name, description) in edge_types {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.strong(name);
                    ui.label(description);
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Draw a mini preview
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::new(200.0, 60.0), egui::Sense::hover());
                    let painter = ui.painter();

                    let theme = nebula_ui::theme::current_theme();
                    painter.rect_filled(rect, 4.0, theme.tokens.card);

                    let from = rect.left_center() + egui::Vec2::new(20.0, 0.0);
                    let to = rect.right_center() - egui::Vec2::new(20.0, 0.0);

                    // Draw simple line representation
                    let color = theme.tokens.accent;
                    let stroke = egui::Stroke::new(2.0, color);

                    match edge_type {
                        EdgeType::Straight => {
                            painter.line_segment([from, to], stroke);
                        }
                        EdgeType::Bezier => {
                            let cp1 = from + egui::Vec2::new(50.0, 0.0);
                            let cp2 = to - egui::Vec2::new(50.0, 0.0);
                            let points: Vec<Pos2> = (0..=20)
                                .map(|i| {
                                    let t = i as f32 / 20.0;
                                    cubic_bezier(from, cp1, cp2, to, t)
                                })
                                .collect();
                            painter.add(egui::Shape::line(points, stroke));
                        }
                        EdgeType::Step => {
                            let mid_x = (from.x + to.x) / 2.0;
                            let points =
                                vec![from, Pos2::new(mid_x, from.y), Pos2::new(mid_x, to.y), to];
                            painter.add(egui::Shape::line(points, stroke));
                        }
                        EdgeType::SmoothStep => {
                            let mid_x = (from.x + to.x) / 2.0;
                            let points =
                                vec![from, Pos2::new(mid_x, from.y), Pos2::new(mid_x, to.y), to];
                            painter.add(egui::Shape::line(points, stroke));
                        }
                    }

                    // Draw endpoints
                    painter.circle_filled(from, 4.0, color);
                    painter.circle_filled(to, 4.0, color);
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
        }
    }

    fn show_node_styles(&mut self, ui: &mut egui::Ui) {
        ui.heading("Node Categories & Styles");
        ui.label("Nodes are colored by category for easy identification.");
        ui.add_space(16.0);

        let categories = [
            ("triggers", "Triggers", "Event sources that start workflows"),
            ("transform", "Transform", "Data transformation operations"),
            ("logic", "Logic", "Conditional and control flow"),
            ("network", "Network", "HTTP, API, and external services"),
            ("storage", "Storage", "Database and file operations"),
            ("custom", "Custom", "User-defined nodes"),
        ];

        ui.horizontal_wrapped(|ui| {
            for (category, name, description) in categories {
                Card::new().show(ui, |ui| {
                    ui.set_min_width(180.0);

                    let theme = nebula_ui::theme::current_theme();
                    let color = theme.color_for_category(category);

                    ui.horizontal(|ui| {
                        let (rect, _) = ui
                            .allocate_exact_size(egui::Vec2::new(16.0, 16.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 4.0, color);
                        ui.strong(name);
                    });

                    ui.label(description);
                });

                ui.add_space(8.0);
            }
        });

        ui.add_space(24.0);
        ui.heading("Data Types");
        ui.label("Pins are colored by data type for type-safe connections.");
        ui.add_space(16.0);

        let data_types = [
            (DataType::Execution, "Execution", "Control flow"),
            (DataType::String, "String", "Text data"),
            (DataType::Number, "Number", "Numeric values"),
            (DataType::Boolean, "Boolean", "True/false"),
            (DataType::Object, "Object", "Key-value maps"),
            (
                DataType::Array(Box::new(DataType::Generic)),
                "Array",
                "Lists of items",
            ),
            (DataType::Generic, "Generic", "Any type"),
        ];

        ui.horizontal_wrapped(|ui| {
            for (data_type, name, description) in data_types {
                let theme = nebula_ui::theme::current_theme();
                let color = theme.color_for_data_type(&data_type);

                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::new(12.0, 12.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 6.0, color);
                    ui.label(format!("{}: {}", name, description));
                });

                ui.add_space(16.0);
            }
        });
    }

    fn show_minimap_demo(&mut self, ui: &mut egui::Ui) {
        ui.heading("Minimap Component");
        ui.label("Bird's-eye view for navigation in large workflows.");
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            ui.label("Position:");
            egui::ComboBox::from_id_salt("minimap_pos")
                .selected_text(format!("{:?}", self.minimap_position))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.minimap_position,
                        MinimapPosition::TopLeft,
                        "TopLeft",
                    );
                    ui.selectable_value(
                        &mut self.minimap_position,
                        MinimapPosition::TopRight,
                        "TopRight",
                    );
                    ui.selectable_value(
                        &mut self.minimap_position,
                        MinimapPosition::BottomLeft,
                        "BottomLeft",
                    );
                    ui.selectable_value(
                        &mut self.minimap_position,
                        MinimapPosition::BottomRight,
                        "BottomRight",
                    );
                });
        });

        ui.add_space(16.0);

        // Show a canvas with minimap
        let (rect, _) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_width(), 400.0),
            egui::Sense::click_and_drag(),
        );

        // Draw background
        Background::new().variant(BackgroundVariant::Dots).draw(
            ui,
            rect,
            self.board_state.canvas.pan,
            self.board_state.canvas.zoom,
        );

        // Draw minimap
        let minimap_config = MinimapConfig {
            position: self.minimap_position,
            width: 200.0,
            height: 150.0,
            colored_nodes: true,
            ..Default::default()
        };

        // Create a sub-ui for the canvas area
        let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));

        Minimap::new(
            &self.nodes,
            &self.connections,
            rect,
            self.board_state.canvas.pan,
            self.board_state.canvas.zoom,
        )
        .config(minimap_config)
        .show(&mut canvas_ui);
    }

    fn show_controls_demo(&mut self, ui: &mut egui::Ui) {
        ui.heading("Controls Panel");
        ui.label("Zoom and navigation controls for the flow editor.");
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            ui.label("Position:");
            egui::ComboBox::from_id_salt("controls_pos")
                .selected_text(format!("{:?}", self.controls_position))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.controls_position,
                        ControlsPosition::TopLeft,
                        "TopLeft",
                    );
                    ui.selectable_value(
                        &mut self.controls_position,
                        ControlsPosition::TopRight,
                        "TopRight",
                    );
                    ui.selectable_value(
                        &mut self.controls_position,
                        ControlsPosition::BottomLeft,
                        "BottomLeft",
                    );
                    ui.selectable_value(
                        &mut self.controls_position,
                        ControlsPosition::BottomRight,
                        "BottomRight",
                    );
                });
        });

        ui.add_space(16.0);

        // Show a canvas with controls
        let (rect, _) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_width(), 400.0),
            egui::Sense::hover(),
        );

        // Draw background
        Background::new()
            .variant(BackgroundVariant::Lines)
            .draw(ui, rect, egui::Vec2::ZERO, 1.0);

        // Draw controls
        let controls_config = ControlsConfig {
            position: self.controls_position,
            show_zoom: true,
            show_fit_view: true,
            show_zoom_reset: true,
            show_fullscreen: true,
            show_lock: true,
            ..Default::default()
        };

        let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));

        let response = Controls::new().config(controls_config).show(&mut canvas_ui);

        // Show which action was triggered
        if !response.actions.is_empty() {
            ui.label(format!("Action triggered: {:?}", response.actions));
        }
    }

    fn add_random_node(&mut self) {
        let categories = [
            "triggers",
            "transform",
            "logic",
            "network",
            "storage",
            "custom",
        ];
        let category = categories[self.node_counter as usize % categories.len()];

        let pos = Pos2::new(
            100.0 + (self.node_counter % 5) as f32 * 200.0,
            100.0 + (self.node_counter / 5) as f32 * 150.0,
        );

        self.add_node_at_with_category(pos, category);
    }

    fn add_node_at(&mut self, pos: Pos2) {
        self.add_node_at_with_category(pos, "custom");
    }

    fn add_node_at_with_category(&mut self, pos: Pos2, category: &str) {
        self.node_counter += 1;

        let node = Node::new(
            format!("node_{}", self.node_counter),
            format!("Node {}", self.node_counter),
        )
        .category(category)
        .at(pos)
        .input("input", DataType::Generic)
        .output("output", DataType::Generic);

        self.nodes.push(node);
    }
}

impl eframe::App for FlowShowcaseApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Flow Components Showcase");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.dark_mode, "Dark").clicked() {
                        self.dark_mode = true;
                        Theme::dark().apply(ctx);
                    }
                    if ui.selectable_label(!self.dark_mode, "Light").clicked() {
                        self.dark_mode = false;
                        Theme::light().apply(ctx);
                    }
                    ui.label("Theme:");
                });
            });
        });

        egui::SidePanel::left("nav_panel")
            .resizable(false)
            .default_width(160.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Demos");
                ui.add_space(8.0);

                let tabs = [
                    (DemoTab::FullEditor, "Full Editor"),
                    (DemoTab::BackgroundPatterns, "Backgrounds"),
                    (DemoTab::EdgeTypes, "Edge Types"),
                    (DemoTab::NodeStyles, "Node Styles"),
                    (DemoTab::MinimapDemo, "Minimap"),
                    (DemoTab::ControlsDemo, "Controls"),
                ];

                for (tab, label) in tabs {
                    if ui
                        .selectable_label(self.current_tab == tab, label)
                        .clicked()
                    {
                        self.current_tab = tab;
                    }
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                ui.heading("Instructions");
                ui.add_space(4.0);
                ui.small("Pan: Middle mouse / Alt+drag");
                ui.small("Zoom: Scroll wheel");
                ui.small("Select: Click node");
                ui.small("Connect: Click pin to pin");
                ui.small("Delete: Select + Delete key");
                ui.small("Add node: Double-click canvas");
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            DemoTab::FullEditor => self.show_full_editor(ui),
            DemoTab::BackgroundPatterns => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_background_patterns(ui);
                });
            }
            DemoTab::EdgeTypes => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_edge_types(ui);
                });
            }
            DemoTab::NodeStyles => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_node_styles(ui);
                });
            }
            DemoTab::MinimapDemo => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_minimap_demo(ui);
                });
            }
            DemoTab::ControlsDemo => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_controls_demo(ui);
                });
            }
        });

        // Always request repaint for smooth animations
        ctx.request_repaint();
    }
}

/// Create a demo workflow with various node types
fn create_demo_workflow() -> Vec<Node> {
    vec![
        // Trigger nodes
        Node::new("webhook", "Webhook Trigger")
            .category("triggers")
            .at(Pos2::new(50.0, 150.0))
            .output("body", DataType::Object)
            .output("headers", DataType::Object)
            .event(),
        Node::new("schedule", "Schedule Trigger")
            .category("triggers")
            .at(Pos2::new(50.0, 350.0))
            .output("timestamp", DataType::Number)
            .event(),
        // Transform nodes
        Node::new("json_parse", "Parse JSON")
            .category("transform")
            .at(Pos2::new(300.0, 100.0))
            .input("text", DataType::String)
            .output("data", DataType::Object),
        Node::new("map", "Map Items")
            .category("transform")
            .at(Pos2::new(300.0, 280.0))
            .input("items", DataType::Array(Box::new(DataType::Object)))
            .input("expression", DataType::String)
            .output("result", DataType::Array(Box::new(DataType::Object))),
        // Logic nodes
        Node::new("if", "If Condition")
            .category("logic")
            .at(Pos2::new(550.0, 150.0))
            .input("value", DataType::Generic)
            .input("condition", DataType::String)
            .output("true", DataType::Generic)
            .output("false", DataType::Generic),
        Node::new("switch", "Switch")
            .category("logic")
            .at(Pos2::new(550.0, 350.0))
            .input("value", DataType::Generic)
            .output("case1", DataType::Generic)
            .output("case2", DataType::Generic)
            .output("default", DataType::Generic),
        // Network nodes
        Node::new("http", "HTTP Request")
            .category("network")
            .at(Pos2::new(800.0, 100.0))
            .input("url", DataType::String)
            .input("method", DataType::String)
            .input("body", DataType::Object)
            .output("response", DataType::Object)
            .output("status", DataType::Number),
        Node::new("graphql", "GraphQL Query")
            .category("network")
            .at(Pos2::new(800.0, 320.0))
            .input("endpoint", DataType::String)
            .input("query", DataType::String)
            .input("variables", DataType::Object)
            .output("data", DataType::Object),
        // Storage nodes
        Node::new("db_query", "Database Query")
            .category("storage")
            .at(Pos2::new(1050.0, 150.0))
            .input("query", DataType::String)
            .input("params", DataType::Array(Box::new(DataType::Generic)))
            .output("rows", DataType::Array(Box::new(DataType::Object))),
        Node::new("cache", "Cache")
            .category("storage")
            .at(Pos2::new(1050.0, 350.0))
            .input("key", DataType::String)
            .input("value", DataType::Generic)
            .input("ttl", DataType::Number)
            .output("cached", DataType::Generic),
    ]
}

/// Create demo connections between nodes
fn create_demo_connections(nodes: &[Node]) -> Vec<Connection> {
    let mut connections = Vec::new();

    // Helper to find node by type_id
    let find_node =
        |type_id: &str| -> Option<&Node> { nodes.iter().find(|n| n.type_id == type_id) };

    // Connect webhook -> json_parse
    if let (Some(webhook), Some(json_parse)) = (find_node("webhook"), find_node("json_parse")) {
        if let (Some(out_pin), Some(in_pin)) = (webhook.outputs.first(), json_parse.inputs.first())
        {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Object));
        }
    }

    // Connect json_parse -> if
    if let (Some(json_parse), Some(if_node)) = (find_node("json_parse"), find_node("if")) {
        if let (Some(out_pin), Some(in_pin)) = (json_parse.outputs.first(), if_node.inputs.first())
        {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Object));
        }
    }

    // Connect if (true) -> http
    if let (Some(if_node), Some(http)) = (find_node("if"), find_node("http")) {
        if let (Some(out_pin), Some(in_pin)) = (if_node.outputs.first(), http.inputs.get(2)) {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Generic));
        }
    }

    // Connect http -> db_query
    if let (Some(http), Some(db_query)) = (find_node("http"), find_node("db_query")) {
        if let (Some(out_pin), Some(in_pin)) = (http.outputs.first(), db_query.inputs.first()) {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Object));
        }
    }

    // Connect schedule -> map
    if let (Some(schedule), Some(map_node)) = (find_node("schedule"), find_node("map")) {
        if let (Some(out_pin), Some(in_pin)) = (schedule.outputs.first(), map_node.inputs.first()) {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Number));
        }
    }

    // Connect map -> switch
    if let (Some(map_node), Some(switch)) = (find_node("map"), find_node("switch")) {
        if let (Some(out_pin), Some(in_pin)) = (map_node.outputs.first(), switch.inputs.first()) {
            connections.push(Connection::new(
                out_pin.id,
                in_pin.id,
                DataType::Array(Box::new(DataType::Object)),
            ));
        }
    }

    // Connect switch -> graphql
    if let (Some(switch), Some(graphql)) = (find_node("switch"), find_node("graphql")) {
        if let (Some(out_pin), Some(in_pin)) = (switch.outputs.first(), graphql.inputs.get(2)) {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Generic));
        }
    }

    // Connect graphql -> cache
    if let (Some(graphql), Some(cache)) = (find_node("graphql"), find_node("cache")) {
        if let (Some(out_pin), Some(in_pin)) = (graphql.outputs.first(), cache.inputs.get(1)) {
            connections.push(Connection::new(out_pin.id, in_pin.id, DataType::Object));
        }
    }

    connections
}

/// Cubic bezier interpolation
fn cubic_bezier(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
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
