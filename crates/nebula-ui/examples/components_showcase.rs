//! Comprehensive showcase of all nebula-ui components.
//!
//! Run with: `cargo run -p nebula-ui --example components_showcase`

use chrono::NaiveDate;
use eframe::egui;
use egui::Pos2;
use nebula_ui::components::{
    BarChart, Calendar, Carousel, CommandItem, CommandPalette, CommandPaletteResponse, DataColumn,
    DataPoint, DataTable, DataTableState, LineChart, Panel as ResizePanel, PieChart, RadioGroup,
    RadioOption, ResizablePanels, Series, Sparkline, Toaster, Toggle,
};
use nebula_ui::flow::{BoardEditor, BoardState, Connection, DataType, Node, NodeId, Pin};
use nebula_ui::prelude::*;
use std::collections::HashMap;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("Nebula UI - Components Showcase"),
        ..Default::default()
    };

    eframe::run_native(
        "Nebula UI Showcase",
        options,
        Box::new(|cc| Ok(Box::new(ShowcaseApp::new(cc)))),
    )
}

#[derive(Default, PartialEq, Clone)]
enum ShowcaseTab {
    #[default]
    Buttons,
    Inputs,
    Selection,
    Feedback,
    Cards,
    Dialogs,
    Charts,
    DataDisplay,
    Layout,
    Advanced,
    Workflow,
}

#[derive(PartialEq, Clone, Debug)]
enum ThemeOption {
    Light,
    Dark,
    System,
}

#[derive(Clone)]
struct User {
    id: usize,
    name: String,
    email: String,
    age: u32,
}

struct ShowcaseApp {
    current_tab: ShowcaseTab,
    dark_mode: bool,

    // Inputs state
    text_value: String,
    password_value: String,
    number_value: f64,
    textarea_value: String,

    // Selection state
    checkbox_a: bool,
    checkbox_b: bool,
    checkbox_c: bool,
    switch_value: bool,
    selected_index: usize,

    // Radio state
    theme_option: ThemeOption,

    // Toggle state
    toggle_value: bool,
    toggle_notifications: bool,
    toggle_dark_mode: bool,

    // Dialog state
    show_dialog: bool,
    show_alert: bool,
    dialog_input: String,

    // Progress state
    progress: f32,

    // Calendar state
    selected_date: Option<NaiveDate>,

    // Carousel state
    carousel_index: usize,

    // Command palette state
    command_palette_open: bool,
    command_query: String,
    command_selected: usize,

    // Data table state
    table_state: DataTableState,
    users: Vec<User>,

    // Resizable state
    split_ratio: f32,

    // Toast state
    toaster: Toaster,

    // Accordion state
    accordion_section1: bool,
    accordion_section2: bool,
    accordion_section3: bool,

    // Collapsible state
    collapsible_open: bool,

    // Workflow state
    board_state: BoardState,
    workflow_nodes: Vec<Node>,
    workflow_connections: Vec<Connection>,
    selected_edge_type: nebula_ui::flow::EdgeType,
}

impl ShowcaseApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Theme::dark().apply(&cc.egui_ctx);

        let users = vec![
            User {
                id: 1,
                name: "Alice".into(),
                email: "alice@example.com".into(),
                age: 28,
            },
            User {
                id: 2,
                name: "Bob".into(),
                email: "bob@example.com".into(),
                age: 35,
            },
            User {
                id: 3,
                name: "Charlie".into(),
                email: "charlie@example.com".into(),
                age: 42,
            },
            User {
                id: 4,
                name: "Diana".into(),
                email: "diana@example.com".into(),
                age: 31,
            },
            User {
                id: 5,
                name: "Eve".into(),
                email: "eve@example.com".into(),
                age: 26,
            },
        ];

        Self {
            current_tab: ShowcaseTab::default(),
            dark_mode: true,
            text_value: String::new(),
            password_value: String::new(),
            number_value: 50.0,
            textarea_value: String::new(),
            checkbox_a: false,
            checkbox_b: true,
            checkbox_c: false,
            switch_value: true,
            selected_index: 0,
            theme_option: ThemeOption::Dark,
            toggle_value: false,
            toggle_notifications: true,
            toggle_dark_mode: true,
            show_dialog: false,
            show_alert: false,
            dialog_input: String::new(),
            progress: 0.0,
            selected_date: None,
            carousel_index: 0,
            command_palette_open: false,
            command_query: String::new(),
            command_selected: 0,
            table_state: DataTableState::new(),
            users,
            split_ratio: 0.3,
            toaster: Toaster::new(),
            accordion_section1: false,
            accordion_section2: false,
            accordion_section3: false,
            collapsible_open: false,
            board_state: BoardState::new(),
            workflow_nodes: create_sample_workflow(),
            workflow_connections: Vec::new(),
            selected_edge_type: nebula_ui::flow::EdgeType::Bezier,
        }
    }

    fn show_buttons(&mut self, ui: &mut egui::Ui) {
        ui.heading("Button Variants");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Button::new("Primary").primary().show(ui);
            Button::new("Secondary").secondary().show(ui);
            Button::new("Ghost").ghost().show(ui);
            Button::new("Destructive").destructive().show(ui);
            Button::new("Outline").outline().show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Button Sizes");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Button::new("Small").primary().small().show(ui);
            Button::new("Medium").primary().show(ui);
            Button::new("Large").primary().large().show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Icon Buttons");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            IconButton::new("▶").show(ui);
            IconButton::new("⏸").show(ui);
            IconButton::new("⏹").show(ui);
            IconButton::new("⚙").show(ui);
            IconButton::new("✎").show(ui);
            IconButton::new("🗑").show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Disabled States");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Button::new("Disabled Primary")
                .primary()
                .disabled(true)
                .show(ui);
            Button::new("Disabled Secondary")
                .secondary()
                .disabled(true)
                .show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Buttons with Icons");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Button::new("Play").primary().icon("▶").show(ui);
            Button::new("Save").secondary().icon("💾").show(ui);
            Button::new("Delete").destructive().icon("🗑").show(ui);
        });
    }

    fn show_inputs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Text Input");
        ui.add_space(12.0);

        TextInput::new(&mut self.text_value)
            .placeholder("Enter text here...")
            .show(ui);

        ui.add_space(16.0);

        ui.heading("Password Input");
        ui.add_space(12.0);

        TextInput::new(&mut self.password_value)
            .placeholder("Enter password...")
            .password()
            .show(ui);

        ui.add_space(16.0);

        ui.heading("Number Input");
        ui.add_space(12.0);

        NumberInput::new(&mut self.number_value)
            .min(0.0)
            .max(100.0)
            .step(5.0)
            .show(ui);

        ui.label(format!("Value: {:.1}", self.number_value));

        ui.add_space(16.0);

        ui.heading("Text Area");
        ui.add_space(12.0);

        TextArea::new(&mut self.textarea_value)
            .placeholder("Enter multiple lines...")
            .rows(4)
            .show(ui);

        ui.add_space(16.0);

        ui.heading("Input with Validation Error");
        ui.add_space(12.0);

        let mut error_text = "invalid value".to_string();
        TextInput::new(&mut error_text)
            .error("This field has an error")
            .show(ui);
    }

    fn show_selection(&mut self, ui: &mut egui::Ui) {
        ui.heading("Checkboxes");
        ui.add_space(12.0);

        Checkbox::new(&mut self.checkbox_a, "Option A").show(ui);
        Checkbox::new(&mut self.checkbox_b, "Option B (pre-checked)").show(ui);
        Checkbox::new(&mut self.checkbox_c, "Option C").show(ui);

        ui.add_space(24.0);
        ui.heading("Switch");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Switch::new(&mut self.switch_value).show(ui);
            ui.label(if self.switch_value {
                "Enabled"
            } else {
                "Disabled"
            });
        });

        ui.add_space(24.0);
        ui.heading("Toggle");
        ui.add_space(12.0);

        Toggle::new(&mut self.toggle_value)
            .label("Enable feature")
            .show(ui);

        ui.add_space(16.0);

        Toggle::new(&mut self.toggle_notifications)
            .label("Notifications")
            .description("Receive push notifications")
            .show(ui);

        ui.add_space(24.0);
        ui.heading("Radio Group");
        ui.add_space(12.0);

        let radio_options = vec![
            RadioOption::new(ThemeOption::Light, "Light Theme"),
            RadioOption::new(ThemeOption::Dark, "Dark Theme"),
            RadioOption::new(ThemeOption::System, "System Default"),
        ];

        RadioGroup::new(&mut self.theme_option, radio_options)
            .label("Theme Selection")
            .show(ui);

        ui.add_space(24.0);
        ui.heading("Select Dropdown");
        ui.add_space(12.0);

        let options = vec![
            SelectOption::new(0, "First Option"),
            SelectOption::new(1, "Second Option"),
            SelectOption::new(2, "Third Option"),
            SelectOption::new(3, "Fourth Option"),
        ];

        Select::new(&mut self.selected_index, options)
            .placeholder("Choose...")
            .show(ui);

        ui.label(format!("Selected index: {}", self.selected_index));
    }

    fn show_feedback(&mut self, ui: &mut egui::Ui) {
        ui.heading("Spinner");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Spinner::new().show(ui);
            ui.label("Loading...");
        });

        ui.add_space(24.0);
        ui.heading("Progress Bar");
        ui.add_space(12.0);

        // Animate progress
        self.progress += 0.005;
        if self.progress > 1.0 {
            self.progress = 0.0;
        }

        ProgressBar::new(self.progress).show(ui);
        ui.label(format!("{:.0}%", self.progress * 100.0));

        ui.add_space(24.0);
        ui.heading("Badges");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Badge::new("Default").show(ui);
            Badge::new("Primary").primary().show(ui);
            Badge::new("Success").success().show(ui);
            Badge::new("Warning").warning().show(ui);
            Badge::new("Destructive").destructive().show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Toasts");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if Button::new("Info Toast").primary().show(ui).clicked() {
                self.toaster.info("This is an info message");
            }
            if Button::new("Success Toast").secondary().show(ui).clicked() {
                self.toaster.success("Operation completed successfully!");
            }
            if Button::new("Error Toast").destructive().show(ui).clicked() {
                self.toaster.error("An error occurred");
            }
            if Button::new("Warning Toast").outline().show(ui).clicked() {
                self.toaster.warning("Please review your input");
            }
        });

        ui.add_space(24.0);
        ui.heading("Separator");
        ui.add_space(12.0);

        Separator::new().show(ui);

        ui.add_space(12.0);
        ui.label("Content after separator");

        // Request repaint for animation
        ui.ctx().request_repaint();
    }

    fn show_cards(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cards");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            Card::new().show(ui, |ui| {
                ui.label("Basic Card");
                ui.add_space(8.0);
                ui.label("This is a simple card with some content.");
            });

            Card::new().show(ui, |ui| {
                ui.label("Another Card");
                ui.add_space(8.0);
                Button::new("Action").primary().show(ui);
            });
        });

        ui.add_space(24.0);
        ui.heading("Card with Custom Styling");
        ui.add_space(12.0);

        Card::new().show(ui, |ui| {
            ui.horizontal(|ui| {
                Badge::new("NEW").success().show(ui);
                ui.heading("Feature Card");
            });
            ui.add_space(8.0);
            ui.label("Cards can contain any UI elements including other components.");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                Button::new("Learn More").secondary().show(ui);
                Button::new("Get Started").primary().show(ui);
            });
        });
    }

    fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Dialogs");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if Button::new("Open Dialog").primary().show(ui).clicked() {
                self.show_dialog = true;
            }

            if Button::new("Show Alert").destructive().show(ui).clicked() {
                self.show_alert = true;
            }
        });

        // Regular dialog
        if self.show_dialog {
            Dialog::new("Example Dialog", &mut self.show_dialog)
                .width(400.0)
                .show(ui.ctx(), |ui| {
                    ui.label("This is a dialog window.");
                    ui.add_space(12.0);
                    TextInput::new(&mut self.dialog_input)
                        .placeholder("Enter something...")
                        .show(ui);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if Button::new("Cancel").secondary().show(ui).clicked() {
                            // Dialog will close
                        }
                        if Button::new("Confirm").primary().show(ui).clicked() {
                            println!("Dialog confirmed with: {}", self.dialog_input);
                        }
                    });
                });
        }

        // Alert dialog
        if self.show_alert {
            AlertDialog::new("Warning", &mut self.show_alert)
                .description("Are you sure you want to proceed? This action cannot be undone.")
                .confirm_text("Yes, proceed")
                .cancel_text("Cancel")
                .show(ui.ctx());
        }

        ui.add_space(24.0);
        ui.heading("Tooltips");
        ui.add_space(12.0);

        let response = Button::new("Hover me").primary().show(ui);
        Tooltip::new("This is a tooltip that appears on hover").show(&response, ui);
    }

    fn show_charts(&mut self, ui: &mut egui::Ui) {
        ui.heading("Line Chart");
        ui.add_space(12.0);

        let sales_data = vec![
            DataPoint::new(0.0, 10.0),
            DataPoint::new(1.0, 25.0),
            DataPoint::new(2.0, 18.0),
            DataPoint::new(3.0, 35.0),
            DataPoint::new(4.0, 28.0),
            DataPoint::new(5.0, 42.0),
            DataPoint::new(6.0, 38.0),
        ];

        let revenue_data = vec![
            DataPoint::new(0.0, 5.0),
            DataPoint::new(1.0, 15.0),
            DataPoint::new(2.0, 12.0),
            DataPoint::new(3.0, 22.0),
            DataPoint::new(4.0, 18.0),
            DataPoint::new(5.0, 30.0),
            DataPoint::new(6.0, 25.0),
        ];

        LineChart::new(vec![
            Series::new("Sales", sales_data),
            Series::new("Revenue", revenue_data),
        ])
        .size(500.0, 200.0)
        .title("Weekly Performance")
        .x_label("Day")
        .show(ui);

        ui.add_space(24.0);
        ui.heading("Bar Chart");
        ui.add_space(12.0);

        let bar_data = vec![
            ("Mon".to_string(), 45.0),
            ("Tue".to_string(), 62.0),
            ("Wed".to_string(), 38.0),
            ("Thu".to_string(), 71.0),
            ("Fri".to_string(), 55.0),
        ];

        BarChart::new(bar_data).size(400.0, 200.0).show(ui);

        ui.add_space(24.0);
        ui.heading("Pie Chart");
        ui.add_space(12.0);

        let pie_data = vec![
            ("Desktop".to_string(), 45.0),
            ("Mobile".to_string(), 35.0),
            ("Tablet".to_string(), 15.0),
            ("Other".to_string(), 5.0),
        ];

        ui.horizontal(|ui| {
            PieChart::new(pie_data.clone()).size(180.0).show(ui);

            ui.add_space(24.0);

            PieChart::new(pie_data).size(180.0).donut().show(ui);
        });

        ui.add_space(24.0);
        ui.heading("Sparklines");
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.label("Trend:");
            Sparkline::new(vec![10.0, 25.0, 15.0, 30.0, 22.0, 35.0, 28.0])
                .size(100.0, 24.0)
                .show(ui);

            ui.add_space(24.0);

            ui.label("With area:");
            Sparkline::new(vec![5.0, 15.0, 10.0, 25.0, 18.0, 30.0, 22.0])
                .size(100.0, 24.0)
                .area()
                .show(ui);
        });
    }

    fn show_data_display(&mut self, ui: &mut egui::Ui) {
        ui.heading("Calendar");
        ui.add_space(12.0);

        let response = Calendar::new(&mut self.selected_date).show(ui);

        if let Some(date) = self.selected_date {
            ui.label(format!("Selected: {}", date));
        }

        ui.add_space(24.0);
        ui.heading("Data Table");
        ui.add_space(12.0);

        let columns: Vec<DataColumn<User>> = vec![
            DataColumn::new("id", "ID", |u: &User| u.id.to_string())
                .width(60.0)
                .sortable(|a, b| a.id.cmp(&b.id)),
            DataColumn::new("name", "Name", |u: &User| u.name.clone())
                .width(120.0)
                .sortable(|a, b| a.name.cmp(&b.name)),
            DataColumn::new("email", "Email", |u: &User| u.email.clone()).width(200.0),
            DataColumn::new("age", "Age", |u: &User| u.age.to_string())
                .width(80.0)
                .right()
                .sortable(|a, b| a.age.cmp(&b.age)),
        ];

        DataTable::new(&self.users, &columns, &mut self.table_state)
            .selectable()
            .page_size(5)
            .show(ui);

        ui.add_space(24.0);
        ui.heading("Carousel");
        ui.add_space(12.0);

        Carousel::new(&mut self.carousel_index, 4)
            .item_width(300.0)
            .item_height(150.0)
            .show(ui, |ui, idx| {
                ui.centered_and_justified(|ui| {
                    ui.heading(format!("Slide {}", idx + 1));
                });
            });
    }

    fn show_layout(&mut self, ui: &mut egui::Ui) {
        ui.heading("Resizable Panels");
        ui.add_space(12.0);

        ui.set_min_height(300.0);

        let split_pct = self.split_ratio * 100.0;
        ResizablePanels::horizontal(&mut self.split_ratio)
            .min_size(100.0)
            .show(ui, |ui, panel| match panel {
                ResizePanel::First => {
                    Card::new().show(ui, |ui| {
                        ui.heading("Left Panel");
                        ui.label("Drag the handle to resize");
                        ui.add_space(8.0);
                        ui.label(format!("Split: {:.0}%", split_pct));
                    });
                }
                ResizePanel::Second => {
                    Card::new().show(ui, |ui| {
                        ui.heading("Right Panel");
                        ui.label("This panel takes the remaining space");
                    });
                }
            });
    }

    fn show_advanced(&mut self, ui: &mut egui::Ui) {
        ui.heading("Command Palette");
        ui.add_space(12.0);

        if Button::new("Open Command Palette (Ctrl+K)")
            .primary()
            .show(ui)
            .clicked()
        {
            self.command_palette_open = true;
            self.command_query.clear();
            self.command_selected = 0;
        }

        // Handle Ctrl+K
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::K)) {
            self.command_palette_open = true;
            self.command_query.clear();
            self.command_selected = 0;
        }

        let commands = vec![
            CommandItem::new("new_file", "New File")
                .shortcut("Ctrl+N")
                .icon("📄")
                .category("File"),
            CommandItem::new("open_file", "Open File")
                .shortcut("Ctrl+O")
                .icon("📂")
                .category("File"),
            CommandItem::new("save", "Save")
                .shortcut("Ctrl+S")
                .icon("💾")
                .category("File"),
            CommandItem::new("find", "Find")
                .shortcut("Ctrl+F")
                .icon("🔍")
                .category("Edit"),
            CommandItem::new("replace", "Replace")
                .shortcut("Ctrl+H")
                .icon("🔄")
                .category("Edit"),
            CommandItem::new("settings", "Settings")
                .shortcut("Ctrl+,")
                .icon("⚙")
                .category("Preferences"),
        ];

        let response = CommandPalette::new(
            &mut self.command_palette_open,
            &mut self.command_query,
            &commands,
            &mut self.command_selected,
        )
        .placeholder("Type a command...")
        .show(ui.ctx());

        if let CommandPaletteResponse::Selected(id) = response {
            self.toaster.info(format!("Selected command: {}", id));
        }

        ui.add_space(24.0);
        ui.heading("Accordion");
        ui.add_space(12.0);

        AccordionItem::new("Getting Started", &mut self.accordion_section1).show(ui, |ui| {
            ui.label("Welcome to Nebula UI! This section covers the basics.");
        });
        AccordionItem::new("Components", &mut self.accordion_section2).show(ui, |ui| {
            ui.label("Learn about all available UI components.");
        });
        AccordionItem::new("Theming", &mut self.accordion_section3).show(ui, |ui| {
            ui.label("Customize colors, fonts, and spacing.");
        });

        ui.add_space(24.0);
        ui.heading("Collapsible");
        ui.add_space(12.0);

        Collapsible::new("Advanced Settings", &mut self.collapsible_open).show(ui, |ui| {
            ui.label("Hidden content that can be expanded");
            Toggle::new(&mut self.toggle_dark_mode)
                .label("Dark mode")
                .show(ui);
        });
    }

    fn show_workflow(&mut self, ui: &mut egui::Ui) {
        ui.heading("Workflow Editor");
        ui.add_space(12.0);

        ui.label("Visual node-based workflow editor. Drag nodes, connect pins, pan with middle mouse, zoom with scroll.");

        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if Button::new("Reset View").secondary().show(ui).clicked() {
                self.board_state.reset_view();
            }
            if Button::new("Add Node").primary().show(ui).clicked() {
                let new_node = Node::new("custom", "New Node")
                    .category("custom")
                    .at(Pos2::new(400.0, 200.0))
                    .input("input", DataType::Generic)
                    .output("output", DataType::Generic);
                self.workflow_nodes.push(new_node);
            }
        });

        ui.add_space(12.0);

        // Build pins map from nodes
        let pins_map: HashMap<NodeId, (Vec<Pin>, Vec<Pin>)> = self
            .workflow_nodes
            .iter()
            .map(|n| (n.id, (n.inputs.clone(), n.outputs.clone())))
            .collect();

        ui.horizontal(|ui| {
            ui.label(format!(
                "Nodes: {} | Connections: {}",
                self.workflow_nodes.len(),
                self.workflow_connections.len()
            ));

            ui.add_space(16.0);
            ui.label("Edge Type:");

            egui::ComboBox::from_id_salt("edge_type_selector")
                .selected_text(match self.selected_edge_type {
                    nebula_ui::flow::EdgeType::Straight => "Straight",
                    nebula_ui::flow::EdgeType::Bezier => "Bezier",
                    nebula_ui::flow::EdgeType::SmoothStep => "SmoothStep",
                    nebula_ui::flow::EdgeType::Smart => "Smart",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        nebula_ui::flow::EdgeType::Straight,
                        "Straight",
                    );
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        nebula_ui::flow::EdgeType::Bezier,
                        "Bezier",
                    );
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        nebula_ui::flow::EdgeType::SmoothStep,
                        "SmoothStep",
                    );
                    ui.selectable_value(
                        &mut self.selected_edge_type,
                        nebula_ui::flow::EdgeType::Smart,
                        "Smart",
                    );
                });

            // Update all existing connections when edge type changes
            for conn in &mut self.workflow_connections {
                conn.edge_type = self.selected_edge_type;
            }
        });

        ui.add_space(8.0);

        // Board editor takes remaining space
        let mut editor = BoardEditor::new(
            &self.workflow_nodes,
            &pins_map,
            &self.workflow_connections,
            &mut self.board_state,
        );
        editor.show_in_ui(ui);

        // Handle events - update node positions
        for event in editor.take_events() {
            match event {
                nebula_ui::flow::BoardEvent::NodeMoved { id, position } => {
                    if let Some(node) = self.workflow_nodes.iter_mut().find(|n| n.id == id) {
                        node.position = position;
                    }
                }
                nebula_ui::flow::BoardEvent::NodeDeleteRequested(id) => {
                    self.workflow_nodes.retain(|n| n.id != id);
                }
                nebula_ui::flow::BoardEvent::ConnectionCreated {
                    from_node: _,
                    from_pin,
                    to_node: _,
                    to_pin,
                } => {
                    // Create a new connection with selected edge type
                    let conn = Connection::with_edge_type(
                        from_pin,
                        to_pin,
                        nebula_ui::flow::DataType::Execution,
                        self.selected_edge_type,
                    );
                    self.workflow_connections.push(conn);
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for ShowcaseApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Show toasts
        self.toaster.show(ctx);

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Nebula UI Showcase");
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
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Components");
                ui.add_space(8.0);

                let tabs = [
                    (ShowcaseTab::Buttons, "Buttons"),
                    (ShowcaseTab::Inputs, "Inputs"),
                    (ShowcaseTab::Selection, "Selection"),
                    (ShowcaseTab::Feedback, "Feedback"),
                    (ShowcaseTab::Cards, "Cards"),
                    (ShowcaseTab::Dialogs, "Dialogs"),
                    (ShowcaseTab::Charts, "Charts"),
                    (ShowcaseTab::DataDisplay, "Data Display"),
                    (ShowcaseTab::Layout, "Layout"),
                    (ShowcaseTab::Advanced, "Advanced"),
                    (ShowcaseTab::Workflow, "Workflow"),
                ];

                for (tab, label) in tabs {
                    if ui
                        .selectable_label(self.current_tab == tab, label)
                        .clicked()
                    {
                        self.current_tab = tab;
                    }
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // Workflow needs full space without ScrollArea
            if self.current_tab == ShowcaseTab::Workflow {
                self.show_workflow(ui);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(16.0);
                    match self.current_tab {
                        ShowcaseTab::Buttons => self.show_buttons(ui),
                        ShowcaseTab::Inputs => self.show_inputs(ui),
                        ShowcaseTab::Selection => self.show_selection(ui),
                        ShowcaseTab::Feedback => self.show_feedback(ui),
                        ShowcaseTab::Cards => self.show_cards(ui),
                        ShowcaseTab::Dialogs => self.show_dialogs(ui),
                        ShowcaseTab::Charts => self.show_charts(ui),
                        ShowcaseTab::DataDisplay => self.show_data_display(ui),
                        ShowcaseTab::Layout => self.show_layout(ui),
                        ShowcaseTab::Advanced => self.show_advanced(ui),
                        ShowcaseTab::Workflow => unreachable!(),
                    }
                    ui.add_space(32.0);
                });
            }
        });
    }
}

/// Create sample workflow nodes
fn create_sample_workflow() -> Vec<Node> {
    // Center nodes in typical canvas area (around 200-400 Y coordinate)
    vec![
        Node::new("trigger", "HTTP Trigger")
            .category("triggers")
            .at(Pos2::new(100.0, 200.0))
            .output("request", DataType::Object)
            .output("headers", DataType::Object)
            .event(),
        Node::new("transform", "Transform Data")
            .category("transform")
            .at(Pos2::new(350.0, 150.0))
            .input("data", DataType::Object)
            .output("result", DataType::Object),
        Node::new("filter", "Filter")
            .category("logic")
            .at(Pos2::new(350.0, 350.0))
            .input("items", DataType::Array(Box::new(DataType::Object)))
            .input("condition", DataType::String)
            .output("matched", DataType::Array(Box::new(DataType::Object)))
            .output("unmatched", DataType::Array(Box::new(DataType::Object))),
        Node::new("http", "HTTP Request")
            .category("network")
            .at(Pos2::new(600.0, 200.0))
            .input("url", DataType::String)
            .input("body", DataType::Object)
            .input("headers", DataType::Object)
            .output("response", DataType::Object)
            .output("status", DataType::Number),
        Node::new("response", "Send Response")
            .category("triggers")
            .at(Pos2::new(850.0, 200.0))
            .input("body", DataType::Object)
            .input("status", DataType::Number)
            .input("headers", DataType::Object),
    ]
}
