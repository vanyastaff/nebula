//! Basic example demonstrating nebula-ui theme and components.
//!
//! Run with: `cargo run -p nebula-ui --example basic`

use eframe::egui;
use nebula_ui::prelude::*;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Nebula UI - Basic Example"),
        ..Default::default()
    };

    eframe::run_native(
        "Nebula UI Basic",
        options,
        Box::new(|cc| Ok(Box::new(BasicApp::new(cc)))),
    )
}

struct BasicApp {
    text_input: String,
    number_value: f64,
    checkbox_checked: bool,
    switch_enabled: bool,
    selected_option: usize,
    dark_mode: bool,
}

impl BasicApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Apply dark theme by default
        let theme = Theme::dark();
        theme.apply(&cc.egui_ctx);

        Self {
            text_input: String::new(),
            number_value: 42.0,
            checkbox_checked: false,
            switch_enabled: true,
            selected_option: 0,
            dark_mode: true,
        }
    }
}

impl eframe::App for BasicApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Nebula UI Components");
            ui.add_space(16.0);

            // Theme toggle
            ui.horizontal(|ui| {
                ui.label("Theme:");
                if ui.selectable_label(self.dark_mode, "Dark").clicked() {
                    self.dark_mode = true;
                    Theme::dark().apply(ctx);
                }
                if ui.selectable_label(!self.dark_mode, "Light").clicked() {
                    self.dark_mode = false;
                    Theme::light().apply(ctx);
                }
            });

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Buttons section
            ui.label("Buttons");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if Button::new("Primary").primary().show(ui).clicked() {
                    println!("Primary clicked!");
                }
                if Button::new("Secondary").secondary().show(ui).clicked() {
                    println!("Secondary clicked!");
                }
                if Button::new("Ghost").ghost().show(ui).clicked() {
                    println!("Ghost clicked!");
                }
                if Button::new("Destructive").destructive().show(ui).clicked() {
                    println!("Destructive clicked!");
                }
                if Button::new("Disabled")
                    .primary()
                    .disabled(true)
                    .show(ui)
                    .clicked()
                {
                    // Won't fire
                }
            });

            ui.add_space(16.0);

            // Button sizes
            ui.label("Button Sizes");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                Button::new("Small").primary().small().show(ui);
                Button::new("Medium").primary().show(ui);
                Button::new("Large").primary().large().show(ui);
            });

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Input section
            ui.label("Text Input");
            ui.add_space(8.0);
            TextInput::new(&mut self.text_input)
                .placeholder("Enter some text...")
                .show(ui);

            ui.add_space(16.0);

            ui.label("Number Input");
            ui.add_space(8.0);
            NumberInput::new(&mut self.number_value)
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .show(ui);

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Checkbox and Switch
            ui.label("Checkbox & Switch");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                Checkbox::new(&mut self.checkbox_checked, "Enable feature").show(ui);
                ui.add_space(24.0);
                Switch::new(&mut self.switch_enabled).show(ui);
                ui.label(if self.switch_enabled { "ON" } else { "OFF" });
            });

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Select
            ui.label("Select");
            ui.add_space(8.0);
            let options = vec![
                SelectOption::new(0, "Option 1"),
                SelectOption::new(1, "Option 2"),
                SelectOption::new(2, "Option 3"),
            ];
            Select::new(&mut self.selected_option, options)
                .placeholder("Choose an option...")
                .show(ui);

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Badges
            ui.label("Badges");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                Badge::new("Default").show(ui);
                Badge::new("Primary").primary().show(ui);
                Badge::new("Success").success().show(ui);
                Badge::new("Warning").warning().show(ui);
                Badge::new("Destructive").destructive().show(ui);
            });

            ui.add_space(24.0);
            ui.separator();
            ui.add_space(16.0);

            // Spinner and Progress
            ui.label("Loading Indicators");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                Spinner::new().show(ui);
                ui.add_space(16.0);
                ProgressBar::new(0.65).show(ui);
            });
        });
    }
}
