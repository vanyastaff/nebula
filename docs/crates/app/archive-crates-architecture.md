# Archived From "docs/archive/crates-architecture.md"

## 11. nebula-ui

**Purpose**: egui-based UI application.

```rust
// nebula-ui/src/lib.rs
use eframe::egui;
use egui_node_graph::{NodeGraph, NodeId as EguiNodeId};

pub struct NebulaApp {
    workflow_editor: WorkflowEditor,
    node_palette: NodePalette,
    properties_panel: PropertiesPanel,
    execution_viewer: ExecutionViewer,
    api_client: ApiClient,
}

impl eframe::App for NebulaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Workflow").clicked() {
                        self.workflow_editor.new_workflow();
                    }
                    if ui.button("Open...").clicked() {
                        self.open_workflow_dialog();
                    }
                    if ui.button("Save").clicked() {
                        self.save_current_workflow();
                    }
                });
            });
        });
        
        // Left panel - Node palette
        egui::SidePanel::left("node_palette").show(ctx, |ui| {
            self.node_palette.render(ui);
        });
        
        // Right panel - Properties
        egui::SidePanel::right("properties").show(ctx, |ui| {
            self.properties_panel.render(ui, &mut self.workflow_editor);
        });
        
        // Central panel - Workflow editor
        egui::CentralPanel::default().show(ctx, |ui| {
            self.workflow_editor.render(ui);
        });
    }
}

// nebula-ui/src/editor.rs
pub struct WorkflowEditor {
    graph: NodeGraph,
    selected_node: Option<EguiNodeId>,
    connection_in_progress: Option<ConnectionInProgress>,
}

impl WorkflowEditor {
    pub fn render(&mut self, ui: &mut egui::Ui) {
        let response = self.graph.show(ui);
        
        // Handle node selection
        if let Some(node_id) = response.selected_nodes.first() {
            self.selected_node = Some(*node_id);
        }
        
        // Handle connection creation
        if let Some(connection) = &response.connection_in_progress {
            self.connection_in_progress = Some(connection.clone());
        }
        
        // Handle node deletion
        for node_id in response.deleted_nodes {
            self.graph.remove_node(node_id);
        }
    }
}
```

